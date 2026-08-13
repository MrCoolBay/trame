//! Le projet : un dossier, un depot git, **un working directory unique**.
//!
//! Il n'y a pas de worktree, pas de copie, pas de copy-on-write. C'est la
//! condition de possibilite de la coordination : deux agents qui ne partagent
//! pas de repertoire n'ont rien a se coordonner.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;
use crate::ids::ProjectId;

/// Un projet ouvert ou connu du workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// Son identifiant.
    pub id: ProjectId,
    /// La root du working directory. Unique, partagee par toutes ses sessions.
    pub path: PathBuf,
    /// Le nom affiche. Par defaut, le dernier segment du path.
    pub name: String,
    /// La toolchain detectee. Elle determine ce qui constitue l'state partage du
    /// projet, donc les ressources a reserver globalement.
    pub toolchain: Toolchain,
    /// Quand le projet a ete ajoute au workspace.
    pub added_at: Timestamp,
    /// La derniere ouverture. `None` si jamais ouvert depuis l'ajout.
    pub last_opened_at: Option<Timestamp>,
}

/// La toolchain d'un projet, deduite des fichiers presents a la root.
///
/// L'interet n'est pas de savoir compiler le projet — Trame ne compile rien —
/// mais de savoir **quel state partage** ses sessions se disputent : `node_modules`
/// et les ports pour Node, `target/` pour Cargo, `.venv` pour Python.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Toolchain {
    /// `Cargo.toml`.
    Cargo,
    /// `package.json`.
    Node,
    /// `pyproject.toml`.
    Python,
    /// `go.mod`.
    Go,
    /// Rien de reconnu. Trame fonctionne quand meme, il ne reserve juste rien.
    Unknown,
}

impl Toolchain {
    /// Le file marker qui trahit cette toolchain.
    #[must_use]
    pub fn marker(self) -> Option<&'static str> {
        match self {
            Self::Cargo => Some("Cargo.toml"),
            Self::Node => Some("package.json"),
            Self::Python => Some("pyproject.toml"),
            Self::Go => Some("go.mod"),
            Self::Unknown => None,
        }
    }

    /// Les repertoires a exclure du watcher et du read-set.
    ///
    /// Sans ca, une seule commande `cargo build` noierait le journal sous des
    /// milliers d'ecritures qu'aucun agent n'a demandees.
    #[must_use]
    pub fn shared_state_dirs(self) -> &'static [&'static str] {
        match self {
            Self::Cargo => &["target"],
            Self::Node => &["node_modules", ".next", "dist"],
            Self::Python => &[".venv", "__pycache__", ".pytest_cache"],
            Self::Go => &["vendor"],
            Self::Unknown => &[],
        }
    }

    /// L'ordre de detection. Le premier marker trouve gagne.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[Self::Cargo, Self::Node, Self::Python, Self::Go]
    }

    /// Le libelle stable stocke en base. Ne jamais le changer sans migration.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Cargo => "cargo",
            Self::Node => "node",
            Self::Python => "python",
            Self::Go => "go",
            Self::Unknown => "unknown",
        }
    }

    /// L'inverse de [`Toolchain::label`]. Un libelle inconnu — ecrit par une version
    /// plus recente de Trame — se relit en [`Toolchain::Unknown`] plutot que d'echouer :
    /// le journal est append-only, on ne peut pas reecrire le passe.
    #[must_use]
    pub fn from_label(label: &str) -> Self {
        match label {
            "cargo" => Self::Cargo,
            "node" => Self::Node,
            "python" => Self::Python,
            "go" => Self::Go,
            _ => Self::Unknown,
        }
    }
}

impl Project {
    /// Le nom par defaut d'un projet : le dernier segment de son path.
    #[must_use]
    pub fn default_name(path: &Path) -> String {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_project_name_is_the_last_path_segment() {
        assert_eq!(
            Project::default_name(Path::new("/Users/x/dev/portailfcd")),
            "portailfcd"
        );
    }

    #[test]
    fn every_known_toolchain_has_a_marker_file() {
        for toolchain in Toolchain::all() {
            assert!(
                toolchain.marker().is_some(),
                "{toolchain:?} doit avoir un marker"
            );
        }
        assert!(Toolchain::Unknown.marker().is_none());
    }
}
