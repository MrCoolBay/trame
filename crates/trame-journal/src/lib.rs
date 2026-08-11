//! Journal SQLite **append-only**, global au workspace.
//!
//! # Emplacement
//!
//! `~/Library/Application Support/Trame/trame.sqlite`. Une base unique pour tous les
//! projets, avec une colonne `project_id` — **jamais** une base dans le depot : ca ne
//! pollue pas les projets, ca survit a leur suppression, et ca permet la question
//! transverse (« qu'est-ce que j'ai fait cette semaine, tous projets confondus »).
//! Cette derniere est la raison principale.
//!
//! # Append-only
//!
//! On n'`UPDATE` pas, on n'efface pas. C'est ce qui rend l'outil auditable : la reponse
//! a « qui a ecrit cette ligne, dans quelle session, en reponse a quel prompt » est une
//! requete, pas une reconstruction.
//!
//! # Ce module a de la valeur tout seul
//!
//! Meme sans aucune detection de conflit, un outil qui repond a la question ci-dessus
//! est immediatement utile. C'est aussi l'angle auditabilite du produit.
//!
//! # Architecture
//!
//! - [`Journal`] — la connexion et les operations, **synchrones**. Testable sans tokio.
//! - [`spawn_journal`] / [`JournalHandle`] — l'acteur qui possede le `Journal`. Une
//!   `Connection` est `Send` mais pas `Sync` : la partager derriere un `Arc<Mutex<_>>`
//!   serait la solution evidente et la mauvaise, c'est de l'etat metier.
//!
//! # Exemple
//!
//! ```no_run
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! use trame_journal::{Journal, spawn_journal};
//!
//! let (journal, _join) = spawn_journal(Journal::open_default()?);
//! // ... record_read / record_write ...
//! let report = journal.flush().await?;
//! assert_eq!(report.errors, 0);
//! # Ok(())
//! # }
//! ```

mod actor;
mod error;
mod records;
mod schema;
mod store;

pub use actor::{FlushReport, JournalHandle, spawn_journal};
pub use error::{JournalError, JournalGone, Result};
pub use records::{
    ProjectRecord, PromptRecord, ReadRecord, ResourceClaimRecord, SessionRecord, WriteRecord,
};
pub use schema::TARGET_VERSION;
pub use store::{
    APPLICATION_SUPPORT_DIR, DATABASE_FILE_NAME, Journal, data_dir, default_database_path,
};

/// Re-export : construire un [`ProjectRecord`] ne doit pas forcer a dependre de
/// `trame-core` explicitement.
pub use trame_core::Toolchain;
