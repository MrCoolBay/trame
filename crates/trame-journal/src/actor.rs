//! L'acteur du journal : il possede la connexion SQLite.
//!
//! Une `rusqlite::Connection` est `Send` mais pas `Sync`. La mettre derriere un
//! `Arc<Mutex<_>>` serait la solution evidente et la mauvaise : c'est de l'etat metier,
//! donc il appartient a un acteur. Le journal est aussi la brique qui doit **serialiser
//! l'ordre d'insertion**, et un acteur le donne par construction.
//!
//! # Le journal est un puits, pas une source
//!
//! Les methodes d'ajout ne portent **pas** de `oneshot` de reponse : leur `await`
//! n'attend que la place dans la file, jamais l'ecriture SQLite. C'est ce qui garde
//! l'admission du registre en microsecondes. Une erreur d'ecriture est journalisee par
//! l'acteur et comptee ; elle ne remonte pas a chaque appel.
//!
//! Pour les tests, [`JournalHandle::flush`] est une **barriere deterministe** : la file
//! est FIFO, donc quand la reponse au `Flush` arrive, tous les messages precedents sont
//! traites. Aucun `sleep` nulle part.

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use trame_core::{ProjectId, SessionId};

use crate::error::{JournalGone, Result};
use crate::records::{
    ProjectRecord, PromptRecord, ReadRecord, ResourceClaimRecord, SessionRecord, WriteRecord,
};
use crate::store::Journal;

/// Capacite de la file. Le journal traite en dizaines de microsecondes ; 256 messages
/// en attente est deja beaucoup. Bornee, **jamais `unbounded_channel`** : une file non
/// bornee transforme une surcharge en fuite de memoire silencieuse.
const CHANNEL_CAPACITY: usize = 256;

/// Ce qu'on peut demander au journal.
enum JournalMsg {
    Project(Box<ProjectRecord>),
    Session(Box<SessionRecord>),
    Prompt(Box<PromptRecord>),
    Read(Box<ReadRecord>),
    Write(Box<WriteRecord>),
    ResourceClaim(Box<ResourceClaimRecord>),
    WritesForProject {
        project: ProjectId,
        reply: oneshot::Sender<Result<Vec<WriteRecord>>>,
    },
    ReadsForSession {
        session: SessionId,
        reply: oneshot::Sender<Result<Vec<ReadRecord>>>,
    },
    Count {
        table: &'static str,
        reply: oneshot::Sender<Result<u64>>,
    },
    Flush(oneshot::Sender<FlushReport>),
}

/// Ce que la barriere [`JournalHandle::flush`] rapporte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlushReport {
    /// Nombre d'ajouts traites depuis le demarrage.
    pub appended: u64,
    /// Nombre d'ajouts qui ont echoue. **Doit rester a zero** : un test qui l'assert
    /// attrape les erreurs SQLite que les ajouts sans reponse avaleraient sinon.
    pub errors: u64,
}

struct JournalActor {
    journal: Journal,
    appended: u64,
    errors: u64,
    rx: mpsc::Receiver<JournalMsg>,
}

impl JournalActor {
    async fn run(mut self) {
        while let Some(msg) = self.rx.recv().await {
            match msg {
                JournalMsg::Project(record) => self.append(self.journal.insert_project(&record)),
                JournalMsg::Session(record) => self.append(self.journal.insert_session(&record)),
                JournalMsg::Prompt(record) => self.append(self.journal.insert_prompt(&record)),
                JournalMsg::Read(record) => self.append(self.journal.insert_read(&record)),
                JournalMsg::Write(record) => self.append(self.journal.insert_write(&record)),
                JournalMsg::ResourceClaim(record) => {
                    self.append(self.journal.insert_resource_claim(&record));
                }
                JournalMsg::WritesForProject { project, reply } => {
                    let _ = reply.send(self.journal.writes_for_project(project));
                }
                JournalMsg::ReadsForSession { session, reply } => {
                    let _ = reply.send(self.journal.reads_for_session(session));
                }
                JournalMsg::Count { table, reply } => {
                    let _ = reply.send(self.journal.count(table));
                }
                JournalMsg::Flush(reply) => {
                    let _ = reply.send(FlushReport {
                        appended: self.appended,
                        errors: self.errors,
                    });
                }
            }
        }
        // Sortie de boucle : tous les handles sont tombes. Arret propre, sans signal
        // dedie ni token d'annulation — une seconde facon de mourir serait un bug de plus.
        tracing::info!(
            appended = self.appended,
            errors = self.errors,
            "journal arrete"
        );
    }

    /// Compte l'ajout et journalise l'echec. Une erreur d'ecriture ne doit jamais tuer
    /// le daemon : elle ferait perdre toutes les sessions du processus.
    fn append(&mut self, outcome: Result<()>) {
        match outcome {
            Ok(()) => self.appended += 1,
            Err(error) => {
                self.errors += 1;
                tracing::error!(%error, "ecriture au journal echouee");
            }
        }
    }
}

/// La poignee du journal. Clonable, c'est le seul acces.
#[derive(Debug, Clone)]
pub struct JournalHandle {
    tx: mpsc::Sender<JournalMsg>,
}

/// Demarre l'acteur du journal.
pub fn spawn_journal(journal: Journal) -> (JournalHandle, JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
    let join = tokio::spawn(
        JournalActor {
            journal,
            appended: 0,
            errors: 0,
            rx,
        }
        .run(),
    );
    (JournalHandle { tx }, join)
}

impl JournalHandle {
    async fn send(&self, msg: JournalMsg) -> Result<(), JournalGone> {
        self.tx.send(msg).await.map_err(|_| JournalGone)
    }

    async fn ask<T>(
        &self,
        make: impl FnOnce(oneshot::Sender<T>) -> JournalMsg,
    ) -> Result<T, JournalGone> {
        let (reply, rx) = oneshot::channel();
        self.send(make(reply)).await?;
        rx.await.map_err(|_| JournalGone)
    }

    /// Ajoute un projet.
    pub async fn record_project(&self, record: ProjectRecord) -> Result<(), JournalGone> {
        self.send(JournalMsg::Project(Box::new(record))).await
    }

    /// Ajoute une session.
    pub async fn record_session(&self, record: SessionRecord) -> Result<(), JournalGone> {
        self.send(JournalMsg::Session(Box::new(record))).await
    }

    /// Ajoute un prompt.
    pub async fn record_prompt(&self, record: PromptRecord) -> Result<(), JournalGone> {
        self.send(JournalMsg::Prompt(Box::new(record))).await
    }

    /// Ajoute une lecture.
    pub async fn record_read(&self, record: ReadRecord) -> Result<(), JournalGone> {
        self.send(JournalMsg::Read(Box::new(record))).await
    }

    /// Ajoute une ecriture admise.
    pub async fn record_write(&self, record: WriteRecord) -> Result<(), JournalGone> {
        self.send(JournalMsg::Write(Box::new(record))).await
    }

    /// Ajoute une reservation de ressource.
    pub async fn record_resource_claim(
        &self,
        record: ResourceClaimRecord,
    ) -> Result<(), JournalGone> {
        self.send(JournalMsg::ResourceClaim(Box::new(record))).await
    }

    /// Attend que tous les messages deja envoyes soient traites, et rapporte le
    /// compteur d'ajouts et d'erreurs.
    ///
    /// La file etant FIFO, c'est une barriere exacte : pas besoin de `sleep`.
    pub async fn flush(&self) -> Result<FlushReport, JournalGone> {
        self.ask(JournalMsg::Flush).await
    }

    /// Les ecritures d'un projet, dans l'ordre de sequence.
    pub async fn writes_for_project(
        &self,
        project: ProjectId,
    ) -> Result<Vec<WriteRecord>, JournalGone> {
        self.ask(|reply| JournalMsg::WritesForProject { project, reply })
            .await?
            .map_err(|error| {
                tracing::error!(%error, "lecture des ecritures echouee");
                JournalGone
            })
    }

    /// Les lectures d'une session.
    pub async fn reads_for_session(
        &self,
        session: SessionId,
    ) -> Result<Vec<ReadRecord>, JournalGone> {
        self.ask(|reply| JournalMsg::ReadsForSession { session, reply })
            .await?
            .map_err(|error| {
                tracing::error!(%error, "lecture des lectures echouee");
                JournalGone
            })
    }

    /// Le nombre de lignes d'une table du schema.
    pub async fn count(&self, table: &'static str) -> Result<u64, JournalGone> {
        self.ask(|reply| JournalMsg::Count { table, reply })
            .await?
            .map_err(|error| {
                tracing::error!(%error, "comptage echoue");
                JournalGone
            })
    }
}
