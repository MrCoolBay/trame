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
use trame_core::{ProjectId, SessionId, Verdict};
use trame_journal::{JournalHandle, ReadRecord, WriteRecord};

use crate::error::RegistryGone;
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
                    let hash = self
                        .state
                        .record_read(session, path.clone(), &content, kind, now);
                    let _ = reply.send(());

                    // Journalisation apres reponse : le journal est un puits.
                    if let Some(hash) = hash {
                        let record = ReadRecord {
                            project: self.state.project(),
                            session,
                            path,
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
                    let now = self.clock.now();
                    let admission = self.state.admit(session, &path, &content, now);
                    let verdict = admission.verdict.clone();

                    // Le verdict part avant l'ecriture au journal : c'est ce qui garde
                    // l'admission en microsecondes plutot qu'en millisecondes.
                    let _ = reply.send(verdict.clone());

                    let record = WriteRecord {
                        project: self.state.project(),
                        session,
                        seq: admission.seq,
                        path,
                        hash_before: admission.hash_before,
                        hash_after: admission.hash_after,
                        verdict: verdict.label().to_owned(),
                        ts: now,
                    };
                    if self.journal.record_write(record).await.is_err() {
                        tracing::error!("journal injoignable : ecriture non enregistree");
                    }
                }

                RegistryMsg::Snapshot(reply) => {
                    let _ = reply.send(self.state.snapshot(self.clock.now()));
                }
            }
        }
        tracing::info!(project = %self.state.project(), "registre arrete");
    }
}

/// La poignee du registre. Clonable, c'est le seul acces.
#[derive(Debug, Clone)]
pub struct RegistryHandle {
    tx: mpsc::Sender<RegistryMsg>,
}

/// Demarre le registre d'un projet.
///
/// L'horloge est injectee : le TTL du read-set serait intestable autrement, et les
/// tests du projet n'ont pas le droit de dormir. Un `Arc` sur une horloge n'est pas de
/// l'etat metier — il n'y a pas de mutation, donc pas d'ordre a garantir.
pub fn spawn_registry(
    project: ProjectId,
    clock: Arc<dyn Clock>,
    journal: JournalHandle,
) -> (RegistryHandle, JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
    let actor = RegistryActor {
        state: RegistryState::new(project),
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

    /// ★ Soumet une ecriture a l'admission, et rend le verdict.
    ///
    /// **Rien n'est bloque en v0.1** : le registre observe, journalise et informe. Le
    /// blocage se decidera apres mesure du taux reel de faux positifs.
    pub async fn admit(
        &self,
        session: SessionId,
        path: impl Into<PathBuf>,
        content: impl Into<String>,
    ) -> Result<Verdict, RegistryGone> {
        let (path, content) = (path.into(), content.into());
        self.ask(|reply| RegistryMsg::Admit {
            session,
            path,
            content,
            reply,
        })
        .await
    }

    /// L'etat courant.
    pub async fn snapshot(&self) -> Result<RegistrySnapshot, RegistryGone> {
        self.ask(RegistryMsg::Snapshot).await
    }
}
