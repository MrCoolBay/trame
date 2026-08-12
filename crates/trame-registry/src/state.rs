//! ★ **La logique d'admission.** Le seul endroit du produit qui n'existe nulle part
//! ailleurs.
//!
//! Cette structure est **pure et synchrone** : elle ne fait aucune I/O, ne lit jamais
//! l'heure elle-meme, et ne touche pas au disque. Elle recoit des chemins, des contenus
//! et un instant, et rend un verdict. L'acteur ([`crate::actor`]) la possede et lui
//! passe les messages un par un.
//!
//! Cette separation est ce qui rend le coeur testable sans runtime, sans agent et sans
//! base : le verdict est une fonction de l'etat et de l'evenement.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::TimeDelta;
use trame_core::clock::Timestamp;
use trame_core::{ContentHash, ProjectId, ProjectRoot, Seq, SessionId, StaleFile, Verdict};

use crate::error::RegistryError;
use crate::msg::{FileSnapshot, ReadKind, RegistrySnapshot, SessionSnapshot};

/// Duree de vie d'une entree du read-set.
///
/// Au-dela, on considere que le contexte de l'agent a suffisamment tourne pour que
/// l'avertissement soit du bruit. **C'est le premier cadran a tourner** si le taux de
/// faux positifs mesure est trop haut — bien avant de payer le suivi de hunks.
pub const READ_SET_TTL: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// L'etat d'un fichier suivi.
#[derive(Debug, Clone)]
struct FileState {
    last_writer: SessionId,
    last_seq: Seq,
    content_hash: ContentHash,
    written_at: Timestamp,
    // v0.4+ : `modified_regions: Vec<Range>`. Absent en v0.1, ou la granularite est le
    // fichier entier (ADR 0012).
}

/// Ce qu'une session a lu et ecrit.
#[derive(Debug, Clone, Default)]
struct SessionState {
    name: String,
    /// Chemin -> (empreinte lue, instant de lecture).
    read_set: HashMap<PathBuf, (ContentHash, Timestamp)>,
    write_set: Vec<PathBuf>,
}

/// L'etat du registre d'un projet.
#[derive(Debug)]
pub(crate) struct RegistryState {
    project: ProjectId,
    /// La racine du working directory. Toute cle de fichier passe par elle : sans ca, la
    /// meme lecture et la meme ecriture peuvent produire deux cles differentes et
    /// `StaleRead` cesse de se declencher en silence.
    root: ProjectRoot,
    seq: Seq,
    files: HashMap<PathBuf, FileState>,
    sessions: HashMap<SessionId, SessionState>,
    ttl: TimeDelta,
}

/// Ce qu'une admission produit : le verdict, plus ce qu'il faut journaliser.
///
/// L'acteur journalise **apres** avoir rendu le verdict : le journal est un puits, pas
/// une source, et aucune requete ne doit se trouver sur le chemin chaud.
#[derive(Debug, Clone)]
pub(crate) struct Admission {
    pub verdict: Verdict,
    pub seq: Seq,
    /// Le chemin **relatif a la racine**, normalise. C'est la cle du registre et du
    /// journal, pas le chemin que l'agent a formule.
    pub key: PathBuf,
    /// Le nom affichable de la session ecrivante, resolu ici parce que c'est le
    /// registre qui tient la table des noms. Il part denormalise dans le journal :
    /// une ligne d'audit doit se lire sans jointure.
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
        }
    }

    /// Fait connaitre une session et son nom affichable.
    pub(crate) fn register_session(&mut self, session: SessionId, name: String) {
        self.sessions.entry(session).or_default().name = name;
    }

    /// Enregistre une lecture.
    ///
    /// Seules les lectures substantielles entrent dans le read-set. Le hash blake3 est
    /// calcule ici — a la lecture — et nulle part ailleurs.
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
        // Un chemin hors du projet n'entre pas dans le read-set : il ne sera jamais la
        // cible d'une admission, donc il ne peut rien perimer.
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

    /// ★ Le controleur d'admission.
    ///
    /// L'ordre compte : on valide le read-set **avant** d'enregistrer l'ecriture, sinon
    /// la session verrait son propre changement comme un changement du monde.
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

        // Le numero de sequence est attribue ici, mais l'etat n'est **pas** encore
        // modifie : c'est `commit_write` qui le fait, et seulement si l'ecriture a
        // reussi. Sinon le registre croirait le fichier modifie et perimerait a tort les
        // lectures des autres sessions.
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

    /// Enregistre l'ecriture dans l'etat. **A n'appeler qu'apres son succes sur disque.**
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
        // Ecrire un fichier vaut relecture : la session connait desormais son contenu.
        // Sans ca, sa propre ecriture la rendrait perimee a l'admission suivante.
        state
            .read_set
            .insert(admission.key.clone(), (admission.hash_after, now));
    }

    /// Le chemin absolu ou ecrire une cle.
    pub(crate) fn resolve(&self, key: &Path) -> PathBuf {
        self.root.resolve(key)
    }

    /// Le calcul du verdict. Aucune mutation.
    fn evaluate(&self, session: SessionId, path: &Path, now: Timestamp) -> Verdict {
        let Some(state) = self.sessions.get(&session) else {
            // Session inconnue : elle n'a rien lu, donc rien ne peut etre perime.
            return Verdict::Clean;
        };

        let mut stale: Vec<StaleFile> = state
            .read_set
            .iter()
            .filter_map(|(read_path, (read_hash, read_at))| {
                // Decroissance : au-dela du TTL, le contexte de l'agent a tourne.
                if now - *read_at > self.ttl {
                    return None;
                }
                let file = self.files.get(read_path)?;
                // Une autre session, et un contenu different. Une reecriture a
                // l'identique ne perime rien : le monde n'a pas change.
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
            // TODO(v0.4) : c'est ici que se decideraient `DisjointWrite` et `Overlap`.
            // A granularite fichier entier (ADR 0012), deux sessions qui ecrivent le
            // meme fichier sans recouvrement de lecture sont indistinguables : on ne
            // sait pas si les regions se recouvrent, donc on ne peut pas trancher entre
            // le niveau 2 et le niveau 3. Les deux variantes existent dans `Verdict`
            // pour que les ajouter soit un `match` a completer et non un changement de
            // type public — mais elles ne sont **jamais produites** en v0.1.
            //
            // Prerequis avant de les implementer : le suivi de hunks, donc la projection
            // des anciennes plages a travers les diffs successifs.
            return Verdict::Clean;
        }

        // Du plus recemment modifie au plus ancien : ce qui vient de bouger est ce qui
        // interesse l'agent en premier.
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

    /// Le nom affichable d'une session, ou sa forme courte si elle n'a pas ete
    /// enregistree. On ne panique pas pour un nom manquant.
    fn session_name(&self, session: SessionId) -> String {
        self.sessions
            .get(&session)
            .map(|state| state.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| session.to_string().chars().take(8).collect())
    }

    /// L'etat courant. Le read-set expose exclut les entrees expirees : c'est ce que le
    /// registre considere reellement.
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

    /// Joue une admission complete **sans toucher au disque** : c'est ce que fait
    /// l'acteur, moins l'ecriture. Les tests de logique pure n'ont pas de disque.
    fn admettre(
        state: &mut RegistryState,
        session: SessionId,
        path: &str,
        content: &str,
        now: Timestamp,
    ) -> Admission {
        let admission = state
            .evaluate_write(session, Path::new(path), content, now)
            .expect("chemin dans le projet");
        state.commit_write(session, &admission, now);
        admission
    }

    fn etat() -> RegistryState {
        RegistryState::new(ProjectId::new(), ProjectRoot::from_canonical("/projet"))
    }

    /// Le scenario canonique, teste **sans acteur, sans tokio, sans journal**.
    /// La logique est une fonction pure de l'etat : c'est ce qui la rend verifiable ici.
    #[test]
    fn le_scenario_canonique_au_niveau_de_la_logique_pure() {
        let clock = ManualClock::new();
        let mut state = etat();
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
            "premiere ecriture : pas d'empreinte d'avant"
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
    fn une_session_inconnue_est_propre() {
        let clock = ManualClock::new();
        let mut state = etat();
        let admission = admettre(&mut state, SessionId::new(), "x.rs", "x", clock.now());
        assert_eq!(
            admission.verdict,
            Verdict::Clean,
            "rien de lu, rien a signaler"
        );
    }

    #[test]
    fn le_nom_manquant_retombe_sur_l_identifiant_court() {
        let mut state = etat();
        let session = SessionId::new();
        let name = state.session_name(session);
        assert_eq!(name.len(), 8, "forme courte de l'UUID, pas une panique");
        state.register_session(session, "nomme".into());
        assert_eq!(state.session_name(session), "nomme");
    }

    #[test]
    fn le_ttl_est_celui_de_la_constante() {
        let state = etat();
        assert_eq!(state.ttl, TimeDelta::from_std(READ_SET_TTL).unwrap());
    }

    #[test]
    fn disjoint_write_et_overlap_ne_sont_jamais_produits_en_v0_1() {
        let clock = ManualClock::new();
        let mut state = etat();
        let a = SessionId::new();
        let b = SessionId::new();

        // Deux sessions ecrivent le meme fichier sans qu'aucune ne l'ait lu : c'est le
        // cas qui donnerait DisjointWrite ou Overlap a granularite hunk.
        admettre(&mut state, a, "gros.rs", "fn un() {}", clock.now());
        let admission = admettre(&mut state, b, "gros.rs", "fn deux() {}", clock.now());

        assert_eq!(
            admission.verdict,
            Verdict::Clean,
            "v0.1 : le niveau 2 n'existe pas encore"
        );
        assert_eq!(admission.verdict.level(), 0);
    }
}
