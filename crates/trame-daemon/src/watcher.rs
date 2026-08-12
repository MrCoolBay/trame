//! Le watcher FSEvents. **Il constate, il n'empeche rien.**
//!
//! # Pourquoi il existe
//!
//! Ce n'est pas une question de couverture du journal, c'est une question de **justesse**.
//! Une session peut ecrire hors admission — `sed -i` dans un `Bash`, un hook git, un build,
//! l'utilisateur dans son editeur — et la sonde l'a confirme. Sans watcher :
//!
//! ```text
//! A lit auth.rs                    -> read-set : hash v1
//! B fait `sed -i` sur auth.rs       -> le disque a v2, le registre croit encore v1
//! A ecrit handlers.rs               -> Clean, alors qu'il devrait etre StaleRead
//! ```
//!
//! Le registre devient **faux**, et le mecanisme central echoue **silencieusement**. C'est
//! le pire mode d'echec possible : l'outil a l'air de fonctionner.
//!
//! # Ce qu'il ne fait pas
//!
//! Il **n'intercepte rien**. Quand FSEvents remonte l'evenement, le fichier est deja ecrit.
//! Il n'y a pas de verdict a rendre, pas d'avis a injecter au bon moment, rien a refuser.
//! Le watcher rattrape l'etat du registre pour que les *prochaines* admissions soient
//! justes — et c'est tout ce qu'on peut en attendre.
//!
//! # Deux pieges evites
//!
//! **L'echo.** Le registre ecrit lui-meme (ADR 0014), donc FSEvents voit aussi ses propres
//! ecritures. Le registre les reconnait a leur empreinte et les ignore : une observation dont
//! le hash est deja celui connu est un echo, pas un evenement. Pas d'horodatage, pas de
//! fenetre de tolerance, pas de course.
//!
//! **Le bruit.** Un `cargo build` produit des milliers d'evenements dans `target/`. Sans
//! filtre, le registre serait noye et le journal illisible. Le filtre respecte les regles
//! `.gitignore` du projet — un fichier ignore par git n'est pas du travail d'agent.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify::{Event, EventKind, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use trame_core::{ContentHash, ProjectRoot};
use trame_registry::RegistryHandle;

/// Repertoires toujours exclus, meme absents du `.gitignore`.
///
/// `.git` change a chaque commande git et ne contient aucun travail d'agent. Les autres sont
/// les etats partages usuels : les avoir en dur evite de dependre d'un `.gitignore` bien
/// tenu pour ne pas se noyer.
const TOUJOURS_EXCLUS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".venv",
    "__pycache__",
    ".next",
    "dist",
];

/// Decide ce qui merite d'atteindre le registre.
pub struct PathFilter {
    root: ProjectRoot,
    gitignore: Gitignore,
}

impl std::fmt::Debug for PathFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PathFilter")
            .field("root", &self.root.as_path())
            .finish_non_exhaustive()
    }
}

impl PathFilter {
    /// Construit le filtre a partir du `.gitignore` du projet, s'il y en a un.
    ///
    /// Un `.gitignore` absent ou illisible n'est pas une erreur : on retombe sur la liste
    /// d'exclusions en dur. Refuser de demarrer parce qu'un projet n'a pas de `.gitignore`
    /// serait disproportionne.
    #[must_use]
    pub fn new(root: ProjectRoot) -> Self {
        let mut builder = GitignoreBuilder::new(root.as_path());
        let fichier = root.as_path().join(".gitignore");
        if fichier.is_file()
            && let Some(error) = builder.add(&fichier)
        {
            tracing::warn!(%error, "gitignore partiellement illisible, exclusions en dur seules");
        }
        let gitignore = builder.build().unwrap_or_else(|error| {
            tracing::warn!(%error, "gitignore inutilisable, exclusions en dur seules");
            Gitignore::empty()
        });
        Self { root, gitignore }
    }

    /// Vrai si ce chemin doit etre signale au registre.
    #[must_use]
    pub fn retient(&self, path: &Path) -> bool {
        let Ok(key) = self.root.relativize(path) else {
            return false; // hors du projet : le registre ne suit que son arbre
        };
        if key.as_os_str().is_empty() {
            return false;
        }
        if key
            .components()
            .any(|c| TOUJOURS_EXCLUS.contains(&c.as_os_str().to_string_lossy().as_ref()))
        {
            return false;
        }
        !self
            .gitignore
            .matched_path_or_any_parents(&key, false)
            .is_ignore()
    }
}

/// Demarre le watcher d'un projet.
///
/// Un watcher par projet ouvert, comme le registre. Il vit tant que son `JoinHandle` n'est
/// pas abandonne ; fermer un projet, c'est le relacher.
///
/// # Erreurs
///
/// Echoue si FSEvents refuse de surveiller la racine — droits, chemin disparu.
pub fn spawn_watcher(
    root: ProjectRoot,
    registry: RegistryHandle,
) -> notify::Result<(JoinHandle<()>, WatcherGuard)> {
    let filtre = Arc::new(PathFilter::new(root.clone()));
    // Borne : une rafale de build peut produire des milliers d'evenements. Une file non
    // bornee transformerait `cargo build` en fuite de memoire (ADR 0015).
    let (tx, mut rx) = mpsc::channel::<PathBuf>(1024);

    let mut observateur = notify::recommended_watcher(move |resultat: notify::Result<Event>| {
        let Ok(event) = resultat else { return };
        // Seuls les evenements qui changent un contenu nous interessent. Un acces en
        // lecture, un changement de droits ou un simple `touch` ne perime rien.
        if !matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
            return;
        }
        for path in event.paths {
            // `try_send` et non `send` : ici on est dans le thread de notify, on ne peut
            // pas attendre. Un evenement perdu sous rafale est acceptable — le fichier
            // sera de toute facon rehashe a la prochaine ecriture qui le concerne, et
            // saturer signifie qu'on est deja dans du bruit de build.
            if tx.try_send(path).is_err() {
                tracing::warn!("file du watcher saturee, evenement perdu");
            }
        }
    })?;
    observateur.watch(root.as_path(), RecursiveMode::Recursive)?;
    tracing::info!(racine = %root.as_path().display(), "watcher FSEvents demarre");

    let join = tokio::spawn(async move {
        while let Some(path) = rx.recv().await {
            if !filtre.retient(&path) {
                continue;
            }
            // Le hash est calcule ici, pas dans le registre : c'est de l'I/O, et le
            // registre est un acteur dont la latence compte.
            let Ok(contenu) = tokio::fs::read(&path).await else {
                // Fichier supprime entre l'evenement et la lecture : cas normal.
                continue;
            };
            let hash = ContentHash::of(&contenu);
            if registry
                .observe_external_write(path.clone(), hash)
                .await
                .is_err()
            {
                tracing::info!("registre arrete, le watcher s'arrete aussi");
                break;
            }
        }
    });

    Ok((
        join,
        WatcherGuard {
            _inner: Box::new(observateur),
        },
    ))
}

/// Tient le watcher FSEvents en vie.
///
/// `notify` arrete la surveillance quand son observateur est abandonne. Le garder dans un
/// type nomme rend cette propriete visible plutot que subtile : abandonner ce garde, c'est
/// arreter de surveiller.
pub struct WatcherGuard {
    _inner: Box<dyn Watcher + Send>,
}

impl std::fmt::Debug for WatcherGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WatcherGuard")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projet_temporaire(gitignore: Option<&str>) -> (PathBuf, PathFilter) {
        let racine =
            std::env::temp_dir().join(format!("trame-filtre-{}", trame_core::ProjectId::new()));
        std::fs::create_dir_all(racine.join("src")).unwrap();
        std::fs::create_dir_all(racine.join("target/debug")).unwrap();
        if let Some(contenu) = gitignore {
            std::fs::write(racine.join(".gitignore"), contenu).unwrap();
        }
        let filtre = PathFilter::new(ProjectRoot::new(&racine).unwrap());
        (racine, filtre)
    }

    #[test]
    fn le_bruit_de_build_est_exclu_meme_sans_gitignore() {
        let (racine, filtre) = projet_temporaire(None);
        assert!(filtre.retient(&racine.join("src/auth.rs")));
        assert!(!filtre.retient(&racine.join("target/debug/trame")));
        assert!(!filtre.retient(&racine.join(".git/index")));
        std::fs::remove_dir_all(&racine).ok();
    }

    #[test]
    fn les_regles_du_gitignore_sont_respectees() {
        let (racine, filtre) = projet_temporaire(Some("*.log\n/secrets/\n"));
        assert!(filtre.retient(&racine.join("src/auth.rs")));
        assert!(!filtre.retient(&racine.join("build.log")));
        assert!(!filtre.retient(&racine.join("secrets/cle.pem")));
        std::fs::remove_dir_all(&racine).ok();
    }

    #[test]
    fn un_chemin_hors_du_projet_est_exclu() {
        let (racine, filtre) = projet_temporaire(None);
        assert!(!filtre.retient(Path::new("/etc/passwd")));
        assert!(!filtre.retient(&racine));
        std::fs::remove_dir_all(&racine).ok();
    }

    /// Un `.gitignore` absent ne doit pas empecher le watcher de fonctionner : on retombe
    /// sur les exclusions en dur.
    #[test]
    fn un_gitignore_absent_n_est_pas_une_erreur() {
        let (racine, filtre) = projet_temporaire(None);
        assert!(filtre.retient(&racine.join("src/main.rs")));
        std::fs::remove_dir_all(&racine).ok();
    }
}
