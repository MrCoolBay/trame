//! ★ **La logique d'admission.** Le seul endroit du produit qui n'existe nulle part
//! ailleurs.
//!
//! Cette structure est **pure et synchrone** : elle ne fait aucune I/O, ne lit jamais
//! l'heure elle-meme, et ne touche pas au disque. Elle recoit des chemins, des contenus
//! et un instant, et rend un verdict. L'acteur ([`crate::actor`]) la possede et lui
//! passe les messages un par un.
//!
//! Cette separation est ce qui rend le coeur testable sans runtime, sans agent et sans
//! base : le verdict est une fonction de l'state et de l'evenement.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::TimeDelta;
use trame_core::clock::Timestamp;
use trame_core::{ContentHash, ProjectId, ProjectRoot, Seq, SessionId, StaleFile, Verdict};

use crate::error::RegistryError;
use crate::msg::{FileSnapshot, ReadKind, RegistrySnapshot, SessionSnapshot, ShadowStats};

/// Duree de vie d'une entree du read-set.
///
/// Au-dela, on considere que le contexte de l'agent a suffisamment tourne pour que
/// l'avertissement soit du bruit. **C'est le premier cadran a tourner** si le taux de
/// faux positifs mesure est trop haut — bien avant de payer le tracked de hunks.
pub const READ_SET_TTL: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// L'state d'un file tracked.
#[derive(Debug, Clone)]
struct FileState {
    last_writer: SessionId,
    last_seq: Seq,
    content_hash: ContentHash,
    written_at: Timestamp,
    // v0.4+ : `modified_regions: Vec<Range>`. Absent en v0.1, ou la granularite est le
    // file entier (ADR 0012).
}

/// Ce qu'une session a lu et ecrit.
#[derive(Debug, Clone, Default)]
struct SessionState {
    name: String,
    /// Chemin -> (empreinte lue, instant de lecture).
    read_set: HashMap<PathBuf, (ContentHash, Timestamp)>,
    /// ★ Le read-set **d'shadow** : les fichiers rapportes par une recherche.
    ///
    /// Il ne participe a **aucun** verdict. Il sert a compter ce qu'on aurait dit si les hits
    /// `Grep` comptaient (ADR 0027). Separe du reel a dessein : une mesure qui modifie ce
    /// qu'elle mesure ne mesure rien.
    ///
    /// Chemin -> (empreinte, instant, taille du resultat du `Grep` d'origine).
    shadow_read_set: HashMap<PathBuf, (ContentHash, Timestamp, usize)>,
    write_set: Vec<PathBuf>,
}

/// L'state du registre d'un projet.
#[derive(Debug)]
pub(crate) struct RegistryState {
    project: ProjectId,
    /// La root du working directory. Toute key de file passe par elle : sans ca, la
    /// meme lecture et la meme ecriture peuvent produire deux cles differentes et
    /// `StaleRead` cesse de se declencher en silence.
    root: ProjectRoot,
    seq: Seq,
    files: HashMap<PathBuf, FileState>,
    sessions: HashMap<SessionId, SessionState>,
    ttl: TimeDelta,
    /// Les compteurs du mode shadow. **Cumulatifs, jamais remis a zero.**
    shadow: ShadowStats,
}

/// Ce qu'une observation hors-bande produit, quand elle n'est pas un echo.
#[derive(Debug, Clone)]
pub(crate) struct Observation {
    pub seq: Seq,
    /// Le path relatif a la root, normalise.
    pub key: PathBuf,
    pub hash_before: Option<ContentHash>,
    pub hash: ContentHash,
}

/// Ce qu'une admission produit : le verdict, plus ce qu'il faut journaliser.
///
/// L'acteur journalise **apres** avoir rendu le verdict : le journal est un puits, pas
/// une source, et aucune requete ne doit se trouver sur le path chaud.
#[derive(Debug, Clone)]
pub(crate) struct Admission {
    pub verdict: Verdict,
    pub seq: Seq,
    /// Le path **relatif a la root**, normalise. C'est la key du registre et du
    /// journal, pas le path que l'agent a formule.
    pub key: PathBuf,
    /// Le nom affichable de la session ecrivante, resolu ici parce que c'est le
    /// registre qui tient la table des noms. Il part denormalise dans le journal :
    /// une line d'audit doit se lire sans jointure.
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
        // Un path hors du projet n'entre pas dans le read-set : il ne sera jamais la
        // target d'une admission, donc il ne peut rien perimer.
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

    /// ★ Enregistre une lecture rapportee par une recherche, **en shadow**.
    ///
    /// Elle n'entre pas dans le read-set reel : elle ne peut donc produire aucun avis, et le
    /// comportement du produit est **exactement** le meme avec ou sans cet appel. C'est la
    /// condition pour que la mesure soit une mesure (ADR 0027).
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

    /// Les compteurs du mode shadow.
    pub(crate) fn shadow_stats(&self) -> ShadowStats {
        self.shadow.clone()
    }

    /// Compte ce que les lectures d'shadow **auraient** produit pour cette ecriture.
    ///
    /// Appele a l'admission, apres le verdict reel et sans l'influencer. Ne compte que ce que
    /// le verdict reel n'a **pas** deja dit : sinon on compterait deux fois un avis qui existe.
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
                // Meme regle que le verdict reel : une autre session, et un content different.
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

        // ★ Le mode shadow compte **apres** le verdict reel et ne le touche pas. Les fichiers
        // deja nommes par le vrai verdict sont exclus : un avis qui existe deja n'est pas un
        // avis potentiel.
        let deja_dits: Vec<PathBuf> = match &verdict {
            Verdict::StaleRead { stale } => stale.iter().map(|f| f.path.clone()).collect(),
            _ => Vec::new(),
        };
        self.count_shadow(session, &deja_dits, now);

        // Le numero de sequence est attribue ici, mais l'state n'est **pas** encore
        // modifie : c'est `commit_write` qui le fait, et seulement si l'ecriture a
        // reussi. Sinon le registre croirait le file modifie et perimerait a tort les
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

    /// Enregistre l'ecriture dans l'state. **A n'appeler qu'apres son succes sur disque.**
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
        // Ecrire un file vaut relecture : la session connait desormais son content.
        // Sans ca, sa propre ecriture la rendrait perimee a l'admission suivante.
        state
            .read_set
            .insert(admission.key.clone(), (admission.hash_after, now));
    }

    /// ★ Enregistre une ecriture **hors-bande**, constatee par le watcher.
    ///
    /// # Pourquoi c'est indispensable et pas un raffinement
    ///
    /// Sans ca, le registre devient **faux**. Si une session modifie `auth.rs` par
    /// `sed -i`, le `FileState` guard l'ancien hash — et la session qui avait lu `auth.rs`
    /// n'obtient **jamais** son `StaleRead`. Le mecanisme central echoue silencieusement,
    /// ce qui est pire que de ne pas exister : l'outil a l'air de fonctionner.
    ///
    /// # L'echo d'une ecriture admise
    ///
    /// Le registre ecrit lui-meme (ADR 0014), donc le watcher voit **aussi** ses propres
    /// ecritures. Regle de deduplication : **une observation dont l'empreinte est deja celle
    /// connue est un echo, pas un evenement.** Elle est ignoree.
    ///
    /// C'est robuste sans horodatage ni fenetre de tolerance, et ca traite au passage le cas
    /// d'une ecriture externe qui reproduit le content a l'identique — un formatter sans
    /// effet, par exemple. Rien n'a change, donc rien n'est signale.
    ///
    /// Rend `None` si l'observation a ete ignoree.
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
                "observation ignoree : empreinte identique, c'est l'echo d'une ecriture connue"
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
            "ecriture hors-bande observee — le registre constate, il n'a rien empeche"
        );
        Some(Observation {
            seq: self.seq,
            key,
            hash_before,
            hash,
        })
    }

    /// Le path absolu ou ecrire une key.
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
                // Une autre session, et un content different. Une reecriture a
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
            // A granularite file entier (ADR 0012), deux sessions qui ecrivent le
            // meme file sans recouvrement de lecture sont indistinguables : on ne
            // sait pas si les regions se recouvrent, donc on ne peut pas trancher entre
            // le niveau 2 et le niveau 3. Les deux variantes existent dans `Verdict`
            // pour que les ajouter soit un `match` a completer et non un changement de
            // type public — mais elles ne sont **jamais produites** en v0.1.
            //
            // Prerequis avant de les implementer : le tracked de hunks, donc la projection
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
        // Les ecritures hors-bande ont un nom fixe et parlant : un agent qui lit
        // « modifie par la session hors-bande » comprend qu'aucune session ne l'a fait.
        if session.is_external() {
            return "hors-bande".to_owned();
        }
        self.sessions
            .get(&session)
            .map(|state| state.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| session.to_string().chars().take(8).collect())
    }

    /// L'state courant. Le read-set expose exclut les entrees expirees : c'est ce que le
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
            .expect("path dans le projet");
        state.commit_write(session, &admission, now);
        admission
    }

    fn state() -> RegistryState {
        RegistryState::new(ProjectId::new(), ProjectRoot::from_canonical("/projet"))
    }

    /// Le scenario canonique, teste **sans acteur, sans tokio, sans journal**.
    /// La logique est une fonction pure de l'state : c'est ce qui la rend verifiable ici.
    #[test]
    fn le_scenario_canonique_au_niveau_de_la_logique_pure() {
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
        let mut state = state();
        let admission = admettre(&mut state, SessionId::new(), "x.rs", "x", clock.now());
        assert_eq!(
            admission.verdict,
            Verdict::Clean,
            "rien de lu, rien a signaler"
        );
    }

    #[test]
    fn le_nom_manquant_retombe_sur_l_identifiant_court() {
        let mut state = state();
        let session = SessionId::new();
        let name = state.session_name(session);
        assert_eq!(name.len(), 8, "forme courte de l'UUID, pas une panique");
        state.register_session(session, "nomme".into());
        assert_eq!(state.session_name(session), "nomme");
    }

    #[test]
    fn le_ttl_est_celui_de_la_constante() {
        let state = state();
        assert_eq!(state.ttl, TimeDelta::from_std(READ_SET_TTL).unwrap());
    }

    #[test]
    fn disjoint_write_et_overlap_ne_sont_jamais_produits_en_v0_1() {
        let clock = ManualClock::new();
        let mut state = state();
        let a = SessionId::new();
        let b = SessionId::new();

        // Deux sessions ecrivent le meme file sans qu'aucune ne l'ait lu : c'est le
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
