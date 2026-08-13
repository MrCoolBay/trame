//! ★ **The admission logic.** The one part of the product that exists nowhere else.
//!
//! This structure is **pure and synchronous**: it performs no I/O, never reads the clock
//! itself, and never touches the disk. It takes paths, contents and an instant, and
//! returns a verdict. The actor ([`crate::actor`]) owns it and hands it messages one at a
//! time.
//!
//! That separation is what makes the core testable with no runtime, no agent and no
//! database: a verdict is a function of the state and the event.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::TimeDelta;
use trame_core::clock::Timestamp;
use trame_core::{ContentHash, ProjectId, ProjectRoot, Seq, SessionId, StaleFile, Verdict};

use crate::error::RegistryError;
use crate::msg::{FileSnapshot, ReadKind, RegistrySnapshot, SessionSnapshot, ShadowStats};

/// How long a read-set entry lives.
///
/// Past that, we consider the agent's context to have turned over enough that the notice
/// would be noise. **This is the first dial to turn** if the measured false-positive rate
/// comes back too high — long before paying for hunk tracking.
pub const READ_SET_TTL: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// A tracked file's state.
#[derive(Debug, Clone)]
struct FileState {
    last_writer: SessionId,
    last_seq: Seq,
    content_hash: ContentHash,
    written_at: Timestamp,
    // v0.4+: `modified_regions: Vec<Range>`. Absent in v0.1, where granularity is the
    // whole file (ADR 0012).
}

/// What a session has read and written.
#[derive(Debug, Clone, Default)]
struct SessionState {
    name: String,
    /// Path -> (fingerprint read, read instant).
    read_set: HashMap<PathBuf, (ContentHash, Timestamp)>,
    /// ★ The **shadow** read-set: the files a search reported.
    ///
    /// It takes part in **no** verdict. It exists to count what would have been said if
    /// `Grep` hits counted (ADR 0027). Kept separate from the real one on purpose: a
    /// measurement that changes what it measures measures nothing.
    ///
    /// Path -> (fingerprint, instant, size of the originating `Grep` result).
    shadow_read_set: HashMap<PathBuf, (ContentHash, Timestamp, usize)>,
    write_set: Vec<PathBuf>,
}

/// A project's registry state.
#[derive(Debug)]
pub(crate) struct RegistryState {
    project: ProjectId,
    /// The working directory root. Every file key goes through it: without that, the same
    /// read and the same write can produce two different keys and `StaleRead` silently
    /// stops firing.
    root: ProjectRoot,
    seq: Seq,
    files: HashMap<PathBuf, FileState>,
    sessions: HashMap<SessionId, SessionState>,
    ttl: TimeDelta,
    /// Shadow mode's counters. **Cumulative, never reset.**
    shadow: ShadowStats,
}

/// What an out-of-band observation produces, when it is not an echo.
#[derive(Debug, Clone)]
pub(crate) struct Observation {
    pub seq: Seq,
    /// The root-relative path, normalised.
    pub key: PathBuf,
    pub hash_before: Option<ContentHash>,
    pub hash: ContentHash,
}

/// What an admission produces: the verdict, plus what has to be journalled.
///
/// The actor journals **after** returning the verdict: the journal is a sink, not a
/// source, and no query belongs on the hot path.
#[derive(Debug, Clone)]
pub(crate) struct Admission {
    pub verdict: Verdict,
    pub seq: Seq,
    /// The **root-relative** path, normalised. This is the registry's and the journal's
    /// key, not the path the agent phrased.
    pub key: PathBuf,
    /// The writing session's display name, resolved here because the registry is what
    /// holds the name table. It goes into the journal denormalised: an audit row must read
    /// without a join.
    pub session_name: String,
    pub hash_before: Option<ContentHash>,
    pub hash_after: ContentHash,
}

impl RegistryState {
    pub(crate) fn new(project: ProjectId, root: ProjectRoot) -> Self {
        Self {
            project,
            root,
            seq: Seq::from_u64(0),
            files: HashMap::new(),
            sessions: HashMap::new(),
            ttl: TimeDelta::from_std(READ_SET_TTL).unwrap_or_else(|_| TimeDelta::minutes(10)),
            shadow: ShadowStats::default(),
        }
    }

    /// Make a session and its display name known.
    pub(crate) fn register_session(&mut self, session: SessionId, name: String) {
        self.sessions.entry(session).or_default().name = name;
    }

    /// Record a read.
    ///
    /// Only substantial reads enter the read-set. The blake3 hash is computed here — at
    /// read time — and nowhere else.
    pub(crate) fn record_read(
        &mut self,
        session: SessionId,
        path: &Path,
        content: &str,
        kind: ReadKind,
        now: Timestamp,
    ) -> Option<(PathBuf, ContentHash)> {
        if !kind.is_substantial() {
            tracing::trace!(?kind, path = %path.display(), "lecture non substantielle, ignoree");
            return None;
        }
        // A path outside the project does not enter the read-set: it will never be the
        // target of an admission, so it cannot stale anything.
        let Ok(key) = self.root.relativize(path) else {
            tracing::debug!(path = %path.display(), "lecture hors du projet, ignoree");
            return None;
        };
        let hash = ContentHash::of(content);
        self.sessions
            .entry(session)
            .or_default()
            .read_set
            .insert(key.clone(), (hash, now));
        Some((key, hash))
    }

    /// ★ Record a read reported by a search, **in shadow**.
    ///
    /// It does not enter the real read-set, so it can produce no notice, and the product's
    /// behaviour is **exactly** the same with or without this call. That is the condition
    /// for the measurement to be a measurement at all (ADR 0027).
    pub(crate) fn record_shadow_read(
        &mut self,
        session: SessionId,
        path: &Path,
        content: &str,
        result_size: usize,
        now: Timestamp,
    ) {
        let Ok(key) = self.root.relativize(path) else {
            return;
        };
        self.shadow.shadow_reads += 1;
        self.sessions
            .entry(session)
            .or_default()
            .shadow_read_set
            .insert(key, (ContentHash::of(content), now, result_size));
    }

    /// Shadow mode's counters.
    pub(crate) fn shadow_stats(&self) -> ShadowStats {
        self.shadow.clone()
    }

    /// Count what the shadow reads **would** have produced for this write.
    ///
    /// Called at admission, after the real verdict and without influencing it. Counts only
    /// what the real verdict did **not** already say: otherwise a notice that already
    /// exists would be counted twice.
    fn count_shadow(&mut self, session: SessionId, deja_dits: &[PathBuf], now: Timestamp) {
        let Some(state) = self.sessions.get(&session) else {
            return;
        };
        let potential: Vec<usize> = state
            .shadow_read_set
            .iter()
            .filter_map(|(path, (hash_lu, lu_a, taille))| {
                if now - *lu_a > self.ttl || deja_dits.contains(path) {
                    return None;
                }
                let file = self.files.get(path)?;
                // Same rule as the real verdict: another session, and different content.
                if file.last_writer == session || file.content_hash == *hash_lu {
                    return None;
                }
                Some(*taille)
            })
            .collect();
        for taille in potential {
            self.shadow.potential_notices += 1;
            *self.shadow.by_size.entry(taille).or_default() += 1;
        }
    }

    /// ★ The admission controller.
    ///
    /// Order matters: the read-set is validated **before** the write is recorded, or the
    /// session would see its own change as a change in the world.
    pub(crate) fn evaluate_write(
        &mut self,
        session: SessionId,
        path: &Path,
        content: &str,
        now: Timestamp,
    ) -> Result<Admission, RegistryError> {
        let key = self
            .root
            .relativize(path)
            .map_err(|_| RegistryError::PathOutsideProject(path.to_path_buf()))?;

        let hash_after = ContentHash::of(content);
        let hash_before = self.files.get(&key).map(|state| state.content_hash);
        let verdict = self.evaluate(session, &key, now);

        // ★ Shadow mode counts **after** the real verdict and does not touch it. Files the
        // real verdict already named are excluded: a notice that exists is not a potential
        // notice.
        let deja_dits: Vec<PathBuf> = match &verdict {
            Verdict::StaleRead { stale } => stale.iter().map(|f| f.path.clone()).collect(),
            _ => Vec::new(),
        };
        self.count_shadow(session, &deja_dits, now);

        // The sequence number is assigned here, but the state is **not** modified yet:
        // `commit_write` does that, and only if the write succeeded. Otherwise the registry
        // would believe the file changed and would wrongly stale the other sessions'
        // reads.
        self.seq = self.seq.next();

        Ok(Admission {
            verdict,
            seq: self.seq,
            session_name: self.session_name(session),
            key,
            hash_before,
            hash_after,
        })
    }

    /// Record the write in the state. **Only call this after it succeeded on disk.**
    pub(crate) fn commit_write(
        &mut self,
        session: SessionId,
        admission: &Admission,
        now: Timestamp,
    ) {
        self.files.insert(
            admission.key.clone(),
            FileState {
                last_writer: session,
                last_seq: admission.seq,
                content_hash: admission.hash_after,
                written_at: now,
            },
        );

        let state = self.sessions.entry(session).or_default();
        if !state.write_set.contains(&admission.key) {
            state.write_set.push(admission.key.clone());
        }
        // Writing a file counts as reading it: the session now knows its content. Without
        // this, its own write would make it stale at the next admission.
        state
            .read_set
            .insert(admission.key.clone(), (admission.hash_after, now));
    }

    /// ★ Record an **out-of-band** write, noticed by the watcher.
    ///
    /// # Why this is essential and not a refinement
    ///
    /// Without it the registry becomes **wrong**. If a session changes `auth.rs` with
    /// `sed -i`, the `FileState` keeps the old hash — and the session that had read
    /// `auth.rs` **never** gets its `StaleRead`. The central mechanism fails silently,
    /// which is worse than not existing: the tool looks like it works.
    ///
    /// # The echo of an admitted write
    ///
    /// The registry writes to disk itself (ADR 0014), so the watcher sees **its own writes
    /// too**. Deduplication rule: **an observation whose fingerprint is already the known
    /// one is an echo, not an event.** It is ignored.
    ///
    /// This is robust with no timestamps and no tolerance window, and it happens to handle
    /// an external write that reproduces the content byte for byte — an ineffective
    /// formatter, say. Nothing changed, so nothing is reported.
    ///
    /// Returns `None` if the observation was ignored.
    pub(crate) fn observe_external_write(
        &mut self,
        path: &Path,
        hash: ContentHash,
        now: Timestamp,
    ) -> Option<Observation> {
        let key = self.root.relativize(path).ok()?;

        if let Some(state) = self.files.get(&key)
            && state.content_hash == hash
        {
            tracing::trace!(
                path = %key.display(),
                "observation ignored: identical fingerprint, this is the echo of a known write"
            );
            return None;
        }

        let hash_before = self.files.get(&key).map(|state| state.content_hash);
        self.seq = self.seq.next();
        self.files.insert(
            key.clone(),
            FileState {
                last_writer: SessionId::EXTERNAL,
                last_seq: self.seq,
                content_hash: hash,
                written_at: now,
            },
        );
        tracing::info!(
            path = %key.display(),
            seq = %self.seq,
            "out-of-band write observed — the registry notices, it prevented nothing"
        );
        Some(Observation {
            seq: self.seq,
            key,
            hash_before,
            hash,
        })
    }

    /// The absolute path a key should be written to.
    pub(crate) fn resolve(&self, key: &Path) -> PathBuf {
        self.root.resolve(key)
    }

    /// The verdict computation. No mutation.
    fn evaluate(&self, session: SessionId, path: &Path, now: Timestamp) -> Verdict {
        let Some(state) = self.sessions.get(&session) else {
            // Unknown session: it has read nothing, so nothing can be stale.
            return Verdict::Clean;
        };

        let mut stale: Vec<StaleFile> = state
            .read_set
            .iter()
            .filter_map(|(read_path, (read_hash, read_at))| {
                // Decay: past the TTL, the agent's context has turned over.
                if now - *read_at > self.ttl {
                    return None;
                }
                let file = self.files.get(read_path)?;
                // Another session, and different content. Rewriting identical content
                // stales nothing: the world did not change.
                if file.last_writer == session || file.content_hash == *read_hash {
                    return None;
                }
                Some(StaleFile {
                    path: read_path.clone(),
                    last_writer: file.last_writer,
                    last_writer_name: self.session_name(file.last_writer),
                    read_at: *read_at,
                    written_at: file.written_at,
                    seq: file.last_seq,
                })
            })
            .collect();

        if stale.is_empty() {
            // TODO(v0.4): this is where `DisjointWrite` and `Overlap` would be decided.
            // At whole-file granularity (ADR 0012), two sessions writing the same file
            // with no read overlap are indistinguishable: we do not know whether the
            // regions overlap, so we cannot choose between level 2 and level 3. Both
            // variants exist in `Verdict` so that adding them is a `match` arm to fill in
            // rather than a public type change — but they are **never produced** in v0.1.
            //
            // Prerequisite before implementing them: hunk tracking, and therefore
            // projecting old ranges through successive diffs.
            return Verdict::Clean;
        }

        // Most recently modified first: what just moved is what interests the agent
        // first.
        stale.sort_by(|a, b| {
            b.written_at
                .cmp(&a.written_at)
                .then_with(|| a.path.cmp(&b.path))
        });

        tracing::debug!(
            session = %session,
            path = %path.display(),
            stale = stale.len(),
            "lecture perimee detectee"
        );
        Verdict::StaleRead { stale }
    }

    /// A session's display name, or its short form if it was never registered. We do not
    /// panic over a missing name.
    fn session_name(&self, session: SessionId) -> String {
        // Out-of-band writes get a fixed, telling name: an agent reading "changed by the
        // out-of-band session" understands that no session did it.
        if session.is_external() {
            return "hors-bande".to_owned();
        }
        self.sessions
            .get(&session)
            .map(|state| state.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| session.to_string().chars().take(8).collect())
    }

    /// The current state. The exposed read-set excludes expired entries: that is what the
    /// registry actually considers.
    pub(crate) fn snapshot(&self, now: Timestamp) -> RegistrySnapshot {
        let mut files: Vec<_> = self
            .files
            .iter()
            .map(|(path, state)| FileSnapshot {
                path: path.clone(),
                last_writer: state.last_writer,
                last_seq: state.last_seq,
                hash: state.content_hash,
            })
            .collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));

        let mut sessions: Vec<_> = self
            .sessions
            .iter()
            .map(|(id, state)| {
                let mut read_set: Vec<_> = state
                    .read_set
                    .iter()
                    .filter(|(_, (_, read_at))| now - *read_at <= self.ttl)
                    .map(|(path, _)| path.clone())
                    .collect();
                read_set.sort();
                let mut write_set = state.write_set.clone();
                write_set.sort();
                SessionSnapshot {
                    session: *id,
                    name: state.name.clone(),
                    read_set,
                    write_set,
                }
            })
            .collect();
        sessions.sort_by_key(|s| s.session);

        RegistrySnapshot {
            project: self.project,
            seq: self.seq,
            files,
            sessions,
            shadow: self.shadow.clone(),
        }
    }

    pub(crate) fn project(&self) -> ProjectId {
        self.project
    }
}

#[cfg(test)]
mod tests {
    use trame_core::clock::{Clock, ManualClock};

    use super::*;

    /// Play a full admission **without touching the disk**: what the actor does, minus the
    /// write. Pure-logic tests have no disk.
    fn admettre(
        state: &mut RegistryState,
        session: SessionId,
        path: &str,
        content: &str,
        now: Timestamp,
    ) -> Admission {
        let admission = state
            .evaluate_write(session, Path::new(path), content, now)
            .expect("path inside the project");
        state.commit_write(session, &admission, now);
        admission
    }

    fn state() -> RegistryState {
        RegistryState::new(ProjectId::new(), ProjectRoot::from_canonical("/projet"))
    }

    /// The canonical scenario, tested **with no actor, no tokio, no journal**. The logic is
    /// a pure function of the state: that is what makes it verifiable here.
    #[test]
    fn the_canonical_scenario_at_the_pure_logic_level() {
        let clock = ManualClock::new();
        let mut state = state();
        let a = SessionId::new();
        let b = SessionId::new();
        state.register_session(a, "ajout-handlers".into());
        state.register_session(b, "refacto-api".into());

        state.record_read(
            a,
            Path::new("auth.rs"),
            "fn verify_token()",
            ReadKind::FullFile,
            clock.now(),
        );

        let admission = admettre(&mut state, b, "auth.rs", "fn validate_token()", clock.now());
        assert_eq!(admission.verdict, Verdict::Clean);
        assert_eq!(admission.seq, Seq::FIRST);
        assert!(
            admission.hash_before.is_none(),
            "first write: no before-fingerprint"
        );

        let admission = admettre(&mut state, a, "handlers.rs", "verify_token()", clock.now());
        let Verdict::StaleRead { stale } = &admission.verdict else {
            panic!("attendu StaleRead, obtenu {:?}", admission.verdict);
        };
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].path, PathBuf::from("auth.rs"));
        assert_eq!(stale[0].last_writer_name, "refacto-api");
    }

    #[test]
    fn an_unknown_session_is_clean() {
        let clock = ManualClock::new();
        let mut state = state();
        let admission = admettre(&mut state, SessionId::new(), "x.rs", "x", clock.now());
        assert_eq!(
            admission.verdict,
            Verdict::Clean,
            "rien de lu, rien a signaler"
        );
    }

    #[test]
    fn a_missing_session_name_falls_back_to_the_short_id() {
        let mut state = state();
        let session = SessionId::new();
        let name = state.session_name(session);
        assert_eq!(name.len(), 8, "the UUID short form, not a panic");
        state.register_session(session, "nomme".into());
        assert_eq!(state.session_name(session), "nomme");
    }

    #[test]
    fn the_ttl_applied_is_the_one_the_constant_declares() {
        let state = state();
        assert_eq!(state.ttl, TimeDelta::from_std(READ_SET_TTL).unwrap());
    }

    #[test]
    fn disjoint_write_and_overlap_are_never_produced_in_v0_1() {
        let clock = ManualClock::new();
        let mut state = state();
        let a = SessionId::new();
        let b = SessionId::new();

        // Two sessions write the same file with neither having read it: the case that
        // would give DisjointWrite or Overlap at hunk granularity.
        admettre(&mut state, a, "gros.rs", "fn un() {}", clock.now());
        let admission = admettre(&mut state, b, "gros.rs", "fn deux() {}", clock.now());

        assert_eq!(
            admission.verdict,
            Verdict::Clean,
            "v0.1: level 2 does not exist yet"
        );
        assert_eq!(admission.verdict.level(), 0);
    }
}
