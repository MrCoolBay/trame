//! Types et coutures partages de Trame.
//!
//! Ce crate est la fondation : il ne depend d'aucun autre crate du workspace et
//! ne fait aucune I/O. Il contient le vocabulaire (identifiants, verdicts, etats
//! de session) et les *coutures* — les traits qui marquent les frontieres ou
//! l'application grandira.
//!
//! # La these que ces types servent
//!
//! Quand l'agent A s'apprete a ecrire, si un fichier qu'il a **lu** a ete modifie
//! depuis par une autre session, A raisonne sur un monde qui n'existe plus.
//! Trame le detecte et l'en informe. [`Verdict::StaleRead`] est ce constat, et
//! [`PromptContributor`] est le canal par lequel il remonte a l'agent.
//!
//! # Les coutures
//!
//! Elles ne servent presque a rien en v0.1 et a tout dans six mois :
//!
//! - [`TaskSource`] — d'ou vient le travail (issue, thread de review, prompt manuel).
//! - [`Forge`] — ou va le resultat. Nommage neutre : `ChangeRequest`, jamais
//!   `PullRequest`, parce que GitLab self-hosted est la cible primaire.
//! - [`PromptContributor`] — pipeline de composition du prompt. **Celle-ci n'est
//!   pas speculative** : c'est par elle que l'avis de lecture perimee est injecte.
//! - [`BranchTarget`] — une session vise une branche neuve *ou* une branche
//!   existante. Sans ca, repondre aux commentaires d'une MR imposerait un refactor.

pub mod clock;
pub mod error;
pub mod forge;
pub mod hash;
pub mod ids;
pub mod notice;
pub mod paths;
pub mod project;
pub mod prompt;
pub mod session;
pub mod task_source;
pub mod verdict;

pub use clock::{Clock, SystemClock, Timestamp};
pub use error::{CoreError, Result};
pub use forge::{ChangeRequest, Forge, ReviewThread};
pub use hash::ContentHash;
pub use ids::{BranchId, BranchName, CrId, ProjectId, Seq, SessionId, ThreadId, WorkItemId};
pub use notice::{ConfigurableNotice, NoticeVariant};
pub use paths::ProjectRoot;
pub use project::{Project, Toolchain};
pub use prompt::{
    PromptContributor, PromptFragment, PromptPipeline, SessionContext, StaleReadNotice,
};
pub use session::{BranchTarget, Harness, Session, SessionState, WorkItemRef};
pub use task_source::{ManualTask, TaskFilter, TaskSource, TaskSourceKind, WorkItem};
pub use verdict::{StaleFile, Verdict};
