//! L'acteur du registre. **Un par projet.**
//!
//! Il possede son etat ; personne ne le partage. La communication passe par `mpsc` en
//! entree et `oneshot` en retour, ce qui donne la **serialisation et l'ordre total par
//! construction** — sans verrou, et sans aucun interleaving a raisonner.
//!
//! C'est necessaire, pas cosmetique : le verdict repond a « ce fichier a-t-il change
//! **depuis** que cette session l'a lu », ce qui suppose un ordre total sur les lectures
//! et les ecritures du projet. Un `Mutex` donnerait l'exclusion mutuelle, pas l'ordre.
//!
//! Deux registres ne se parlent jamais : deux projets sont independants par
//! construction, donc aucun interblocage n'est possible entre eux.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use trame_core::clock::Clock;
use trame_core::{ContentHash, ProjectId, ProjectRoot, SessionId, Verdict};
use trame_journal::{JournalHandle, ReadRecord, WriteOrigin, WriteRecord};

use crate::error::{RegistryError, RegistryGone};
use crate::msg::{ReadKind, RegistryMsg, RegistrySnapshot};
use crate::state::RegistryState;

/// Capacite de la file. A deux a cinq sessions par projet et une admission traitee en
/// microsecondes, 64 messages en attente est deja large. Bornee, **jamais
/// `unbounded_channel`** : une file non bornee transforme une surcharge en fuite de
/// memoire silencieuse.
const CHANNEL_CAPACITY: usize = 64;

struct RegistryActor {
    state: RegistryState,
    clock: Arc<dyn Clock>,
    journal: JournalHandle,
    rx: mpsc::Receiver<RegistryMsg>,
}

impl RegistryActor {
    async fn run(mut self) {
        while let Some(msg) = self.rx.recv().await {
            match msg {
                RegistryMsg::RegisterSession {
                    session,
                    name,
                    reply,
                } => {
                    self.state.register_session(session, name);
                    // L'appelant a pu abandonner : son `oneshot` est ferme. Ce n'est pas
                    // une erreur, on l'ignore. `let _ =` et jamais `.unwrap()`.
                    let _ = reply.send(());
                }

                RegistryMsg::RecordRead {
                    session,
                    path,
                    content,
                    kind,
                    reply,
                } => {
                    let now = self.clock.now();
                    let retenue = self.state.record_read(session, &path, &content, kind, now);
                    let _ = reply.send(());

                    // Journalisation apres reponse : le journal est un puits. Le chemin
                    // journalise est la cle normalisee, pas celle que l'agent a formulee.
                    if let Some((key, hash)) = retenue {
                        let record = ReadRecord {
                            project: self.state.project(),
                            session,
                            path: key,
                            hash,
                            ts: now,
                        };
                        if self.journal.record_read(record).await.is_err() {
                            tracing::error!("journal injoignable : lecture non enregistree");
                        }
                    }
                }

                RegistryMsg::Admit {
                    session,
                    path,
                    content,
                    reply,
                } => {
                    let outcome = self.admit(session, &path, &content).await;
                    let _ = reply.send(outcome);
                }

                RegistryMsg::ObserveExternalWrite { path, hash, reply } => {
                    let now = self.clock.now();
                    let observation = self.state.observe_external_write(&path, hash, now);
                    let _ = reply.send(());

                    // Une observation ignoree — l'echo d'une ecriture qu'on a faite
                    // nous-memes — ne laisse aucune ligne. Sinon le journal compterait
                    // chaque admission deux fois.
                    if let Some(observation) = observation {
                        let record = WriteRecord {
                            project: self.state.project(),
                            session: SessionId::EXTERNAL,
                            session_name: "hors-bande".to_owned(),
                            seq: observation.seq,
                            path: observation.key,
                            hash_before: observation.hash_before,
                            hash_after: observation.hash,
                            // Aucun verdict : personne n'a admis cette ecriture.
                            verdict: None,
                            origin: WriteOrigin::Observed,
                            ts: now,
                        };
                        if self.journal.record_write(record).await.is_err() {
                            tracing::error!(
                                "journal injoignable : ecriture hors-bande non enregistree"
                            );
                        }
                    }
                }

                RegistryMsg::Snapshot(reply) => {
                    let _ = reply.send(self.state.snapshot(self.clock.now()));
                }
            }
        }
        tracing::info!(project = %self.state.project(), "registre arrete");
    }

    /// ★ Admission **et** ecriture, dans cet ordre, dans le meme acteur (ADR 0014).
    ///
    /// L'ordre importe : evaluer, ecrire, puis enregistrer. Enregistrer avant d'ecrire
    /// ferait croire au registre que le fichier a change alors qu'il a peut-etre echoue,
    /// et il perimerait a tort les lectures des autres sessions.
    async fn admit(
        &mut self,
        session: SessionId,
        path: &std::path::Path,
        content: &str,
    ) -> Result<Verdict, RegistryError> {
        let now = self.clock.now();
        let admission = self.state.evaluate_write(session, path, content, now)?;

        // L'ecriture. `tokio::fs` pour ne pas bloquer le runtime ; la serialisation par
        // l'acteur est voulue — deux ecritures du meme fichier dans un ordre indetermine
        // seraient un bug.
        let cible = self.state.resolve(&admission.key);
        if let Some(parent) = cible.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|source| RegistryError::Write {
                    path: admission.key.clone(),
                    source,
                })?;
        }
        tokio::fs::write(&cible, content)
            .await
            .map_err(|source| RegistryError::Write {
                path: admission.key.clone(),
                source,
            })?;

        // L'ecriture a reussi : l'etat peut refleter le disque.
        self.state.commit_write(session, &admission, now);

        let verdict = admission.verdict.clone();
        let record = WriteRecord {
            project: self.state.project(),
            session,
            session_name: admission.session_name,
            seq: admission.seq,
            path: admission.key,
            hash_before: admission.hash_before,
            hash_after: admission.hash_after,
            verdict: Some(verdict.label().to_owned()),
            origin: WriteOrigin::Admitted,
            ts: now,
        };
        if self.journal.record_write(record).await.is_err() {
            tracing::error!("journal injoignable : ecriture non enregistree");
        }
        Ok(verdict)
    }
}

/// La poignee du registre. Clonable, c'est le seul acces.
#[derive(Debug, Clone)]
pub struct RegistryHandle {
    tx: mpsc::Sender<RegistryMsg>,
}

/// Demarre le registre d'un projet.
///
/// `root` est la racine du working directory : le registre y ecrit, et **toute cle de
/// fichier passe par elle**. Sans cette normalisation, la meme lecture et la meme ecriture
/// peuvent produire deux cles differentes et `StaleRead` cesse de se declencher en silence.
///
/// L'horloge est injectee : le TTL du read-set serait intestable autrement, et les
/// tests du projet n'ont pas le droit de dormir. Un `Arc` sur une horloge n'est pas de
/// l'etat metier — il n'y a pas de mutation, donc pas d'ordre a garantir.
pub fn spawn_registry(
    project: ProjectId,
    root: ProjectRoot,
    clock: Arc<dyn Clock>,
    journal: JournalHandle,
) -> (RegistryHandle, JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
    let actor = RegistryActor {
        state: RegistryState::new(project, root),
        clock,
        journal,
        rx,
    };
    (RegistryHandle { tx }, tokio::spawn(actor.run()))
}

impl RegistryHandle {
    async fn ask<T>(
        &self,
        make: impl FnOnce(oneshot::Sender<T>) -> RegistryMsg,
    ) -> Result<T, RegistryGone> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(make(reply)).await.map_err(|_| RegistryGone)?;
        rx.await.map_err(|_| RegistryGone)
    }

    /// Fait connaitre une session et son nom affichable.
    pub async fn register_session(
        &self,
        session: SessionId,
        name: impl Into<String>,
    ) -> Result<(), RegistryGone> {
        let name = name.into();
        self.ask(|reply| RegistryMsg::RegisterSession {
            session,
            name,
            reply,
        })
        .await
    }

    /// Enregistre une lecture. Seul [`ReadKind::FullFile`] entre dans le read-set.
    pub async fn record_read(
        &self,
        session: SessionId,
        path: impl Into<PathBuf>,
        content: impl Into<String>,
        kind: ReadKind,
    ) -> Result<(), RegistryGone> {
        let (path, content) = (path.into(), content.into());
        self.ask(|reply| RegistryMsg::RecordRead {
            session,
            path,
            content,
            kind,
            reply,
        })
        .await
    }

    /// ★ Soumet une ecriture a l'admission. Le registre **evalue, ecrit, journalise**,
    /// et rend le verdict (ADR 0014).
    ///
    /// Faillible : l'admission inclut l'ecriture, donc elle peut echouer. Rendre un
    /// verdict sans avoir ecrit serait un mensonge — l'appelant repondrait « admis » a un
    /// agent qui croirait son fichier ecrit.
    ///
    /// **Rien n'est bloque en v0.1** : le registre observe, journalise et informe. Le
    /// blocage se decidera apres mesure du taux reel de faux positifs.
    pub async fn admit(
        &self,
        session: SessionId,
        path: impl Into<PathBuf>,
        content: impl Into<String>,
    ) -> Result<Verdict, RegistryError> {
        let (path, content) = (path.into(), content.into());
        self.ask(|reply| RegistryMsg::Admit {
            session,
            path,
            content,
            reply,
        })
        .await?
    }

    /// Signale une ecriture **hors-bande** constatee par le watcher.
    ///
    /// Le watcher constate apres coup : ce message n'empeche rien, il empeche seulement le
    /// registre de devenir faux. Une observation dont l'empreinte est deja connue est un
    /// echo d'une ecriture admise et sera ignoree.
    pub async fn observe_external_write(
        &self,
        path: impl Into<PathBuf>,
        hash: ContentHash,
    ) -> Result<(), RegistryGone> {
        let path = path.into();
        self.ask(|reply| RegistryMsg::ObserveExternalWrite { path, hash, reply })
            .await
    }

    /// L'etat courant.
    pub async fn snapshot(&self) -> Result<RegistrySnapshot, RegistryGone> {
        self.ask(RegistryMsg::Snapshot).await
    }
}
