//! Journal SQLite **append-only**, global au workspace.
//!
//! # Emplacement
//!
//! `~/Library/Application Support/Trame/trame.sqlite`. Une base unique pour tous
//! les projets, jamais une base dans le depot : ca ne pollue pas les projets, ca
//! survit a leur suppression, et ca permet la question transverse — « qu'est-ce
//! que j'ai fait cette semaine, tous projets confondus ».
//!
//! # Append-only
//!
//! On n'`UPDATE` pas, on n'efface pas. L'etat courant d'une session est le
//! dernier evenement la concernant. C'est ce qui rend l'outil auditable : la
//! reponse a « qui a ecrit cette ligne, dans quelle session, en reponse a quel
//! prompt » est une requete, pas une reconstruction.
//!
//! # Schema vise (phase 1)
//!
//! ```sql
//! projects(id, path, name, toolchain, added_at, last_opened_at)
//! sessions(id, project_id, name, harness, target_branch, work_item, state, created_at)
//! prompts(id, session_id, content, ts)
//! reads(id, project_id, session_id, path, hash, ts)
//! writes(id, project_id, session_id, seq, path, hash_before, hash_after, verdict, ts)
//! resource_claims(resource, project_id, session_id, claimed_at)
//!
//! UNIQUE(project_id, seq)   -- la sequence est locale au projet, jamais globale
//! ```
//!
//! Ce crate est vide en phase 0. Les frontieres de crates *sont* l'architecture :
//! les poser maintenant coute une journee, les retrofitter coute une reecriture.

/// Le nom du fichier de base, sous le repertoire de support de l'application.
pub const DATABASE_FILE_NAME: &str = "trame.sqlite";

/// Le sous-repertoire de `~/Library/Application Support/`.
pub const APPLICATION_SUPPORT_DIR: &str = "Trame";
