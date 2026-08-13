//! Normalisation des chemins autour de la root d'un projet.
//!
//! # Pourquoi ce module existe
//!
//! Il a ete ecrit apres un constat de la validation live du transport ACP, et il corrige
//! un mode d'echec **silencieux** qui aurait rendu le produit inoperant sans qu'aucun
//! test ne le voie.
//!
//! L'agent renvoie des chemins **absolus**, et **resolus** : la root passee etait
//! `/var/folders/…/projet`, l'agent a repondu `/private/var/folders/…/projet/auth.rs`.
//! Sur macOS, `/var` est un lien symbolique vers `/private/var`.
//!
//! Consequence si l'on se contentait de retirer le prefixe tel quel : le prefixe ne
//! correspond pas, la relativisation echoue, et deux formes du **meme file**
//! deviennent deux cles differentes dans le read-set. Le scenario canonique cesserait de
//! fonctionner sans rien casser de visible :
//!
//! ```text
//! A lit  /var/folders/…/auth.rs          -> key « /var/folders/…/auth.rs »
//! B ecrit /private/var/folders/…/auth.rs -> key « /private/var/folders/…/auth.rs »
//! A ecrit handlers.rs                    -> Clean, alors qu'il devrait etre StaleRead
//! ```
//!
//! Le registre se tairait exactement quand il devrait parler. C'est le pire mode d'echec
//! possible pour cet outil, et il est invisible : les tests, qui utilisent des chemins
//! relatifs, passent tous.
//!
//! **Toute key de file du registre et du journal passe donc par [`ProjectRoot`].**

use std::path::{Component, Path, PathBuf};

use crate::error::CoreError;

/// La root canonique d'un projet, et le seul moyen d'en deriver des chemins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRoot {
    canonical: PathBuf,
}

impl ProjectRoot {
    /// Canonicalise la root du projet.
    ///
    /// Fait a l'ouverture du projet, une seule fois : c'est la seule resolution de liens
    /// symboliques qui touche le disque.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, CoreError> {
        let path = path.as_ref();
        let canonical = path
            .canonicalize()
            .map_err(|source| CoreError::Backend(Box::new(source)))?;
        Ok(Self { canonical })
    }

    /// Construit sans toucher au disque. Pour les tests, et pour un projet dont la root
    /// est deja connue canonique.
    #[must_use]
    pub fn from_canonical(path: impl Into<PathBuf>) -> Self {
        Self {
            canonical: path.into(),
        }
    }

    /// La root, sous sa forme canonique.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.canonical
    }

    /// Ramene un path quelconque a sa forme **relative a la root**, qui est la key
    /// utilisee partout ailleurs.
    ///
    /// Accepte un path relatif ou absolu, resolu ou non. Refuse tout ce qui sort de la
    /// root : le registre ne peut rien garantir sur ce qu'il ne voit pas.
    ///
    /// N'exige pas que le file existe — une ecriture cree souvent un file neuf.
    /// Seule la partie existante du path est resolue.
    pub fn relativize(&self, path: impl AsRef<Path>) -> Result<PathBuf, CoreError> {
        let path = path.as_ref();
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.canonical.join(path)
        };

        let resolved = resolve_existing_prefix(&absolute);
        let normalized = lexical_normalize(&resolved);

        normalized
            .strip_prefix(&self.canonical)
            .map(Path::to_path_buf)
            .map_err(|_| CoreError::PathOutsideProject(path.to_path_buf()))
    }

    /// L'inverse : d'une key relative vers le path absolu a ouvrir.
    #[must_use]
    pub fn resolve(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.canonical.join(relative)
    }

    /// Vrai si ce path appartient au projet.
    #[must_use]
    pub fn contains(&self, path: impl AsRef<Path>) -> bool {
        self.relativize(path).is_ok()
    }
}

/// Canonicalise le plus long prefixe **existant** du path, puis rejoue la queue.
///
/// `canonicalize` exige que la target existe, ce qui est faux pour une creation de
/// file. On resout donc ce qui existe — la ou vivent les liens symboliques — et on
/// laisse le reste tel quel.
fn resolve_existing_prefix(path: &Path) -> PathBuf {
    let mut ancestor = path;
    let mut tail: Vec<&std::ffi::OsStr> = Vec::new();

    loop {
        if let Ok(canonical) = ancestor.canonicalize() {
            let mut out = canonical;
            for segment in tail.iter().rev() {
                out.push(segment);
            }
            return out;
        }
        match (ancestor.parent(), ancestor.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name);
                ancestor = parent;
            }
            // Plus d'ancetre existant : rien a resoudre, on rend le path tel quel.
            _ => return path.to_path_buf(),
        }
    }
}

/// Supprime les `.` et resout les `..` **lexicalement**.
///
/// Indispensable pour la partie non existante du path : sans ca, `projet/neuf/../../..`
/// sortirait de la root sans que `strip_prefix` s'en apercoive.
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // `pop` sur une root ne fait rien, ce qui est le comportement voulu :
                // on ne remonte jamais au-dessus de `/`.
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cree un repertoire temporaire dont le path passe par un lien symbolique, comme
    /// `/var` sur macOS. C'est la situation exacte de la validation live.
    fn temp_root() -> (PathBuf, ProjectRoot) {
        let brut = std::env::temp_dir().join(format!("trame-paths-{}", crate::ProjectId::new()));
        std::fs::create_dir_all(&brut).unwrap();
        let root = ProjectRoot::new(&brut).unwrap();
        (brut, root)
    }

    /// ★ Le test qui justifie ce module.
    ///
    /// Sur macOS, `std::env::temp_dir()` rend `/var/folders/…` et `canonicalize` rend
    /// `/private/var/folders/…`. Les deux formes doivent donner **la meme key**.
    #[test]
    fn both_forms_of_the_same_path_give_the_same_key() {
        let (brut, root) = temp_root();

        let via_brut = root.relativize(brut.join("auth.rs")).unwrap();
        let via_canonique = root.relativize(root.as_path().join("auth.rs")).unwrap();
        let via_relatif = root.relativize("auth.rs").unwrap();

        assert_eq!(via_brut, PathBuf::from("auth.rs"));
        assert_eq!(via_canonique, PathBuf::from("auth.rs"));
        assert_eq!(via_relatif, PathBuf::from("auth.rs"));

        std::fs::remove_dir_all(&brut).ok();
    }

    #[test]
    fn a_path_that_does_not_exist_still_relativizes() {
        let (brut, root) = temp_root();
        // Cas d'une creation : le file n'existe pas encore, et son repertoire non plus.
        let key = root.relativize(brut.join("src/neuf/file.rs")).unwrap();
        assert_eq!(key, PathBuf::from("src/neuf/file.rs"));
        std::fs::remove_dir_all(&brut).ok();
    }

    #[test]
    fn a_path_outside_the_project_is_refused() {
        let root = ProjectRoot::from_canonical("/projet");
        assert!(matches!(
            root.relativize("/etc/passwd"),
            Err(CoreError::PathOutsideProject(_))
        ));
        assert!(!root.contains("/autre/projet/auth.rs"));
    }

    #[test]
    fn dot_dot_cannot_climb_out_of_the_project_root() {
        let root = ProjectRoot::from_canonical("/projet");
        // Lexicalement, ceci sort du projet : ca doit etre refuse et non pas normalise
        // en silence vers quelque chose d'admissible.
        assert!(root.relativize("../../etc/passwd").is_err());
        assert!(root.relativize("/projet/../autre/x.rs").is_err());
        // En revanche, un aller-retour interne reste dans le projet.
        assert_eq!(
            root.relativize("/projet/src/../auth.rs").unwrap(),
            PathBuf::from("auth.rs")
        );
    }

    #[test]
    fn redundant_path_components_are_dropped() {
        let root = ProjectRoot::from_canonical("/projet");
        assert_eq!(
            root.relativize("/projet/./src/./auth.rs").unwrap(),
            PathBuf::from("src/auth.rs")
        );
    }

    #[test]
    fn resolve_is_the_inverse_of_relativize() {
        let root = ProjectRoot::from_canonical("/projet");
        let key = root.relativize("/projet/src/auth.rs").unwrap();
        assert_eq!(root.resolve(&key), PathBuf::from("/projet/src/auth.rs"));
    }

    #[test]
    fn the_root_itself_relativizes_to_the_empty_path() {
        let root = ProjectRoot::from_canonical("/projet");
        assert_eq!(root.relativize("/projet").unwrap(), PathBuf::from(""));
    }
}
