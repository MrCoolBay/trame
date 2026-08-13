//! Identifiants. Newtypes systematiques : un `SessionId` n'est jamais
//! interchangeable avec un `ProjectId`, meme si les deux portent un UUID.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Declare un identifiant opaque adosse a un UUID v4.
macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Tire un nouvel identifiant.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// L'UUID sous-jacent. Utile au moment de persister.
            #[must_use]
            pub fn as_uuid(&self) -> &Uuid {
                &self.0
            }

            /// Reconstruit un identifiant depuis un UUID connu (lecture du journal).
            #[must_use]
            pub fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        /// Relecture depuis une colonne `TEXT` du journal.
        impl std::str::FromStr for $name {
            type Err = uuid::Error;

            fn from_str(raw: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(raw)?))
            }
        }
    };
}

uuid_id! {
    /// Un projet : un dossier, un depot git, un working directory unique.
    ProjectId
}

uuid_id! {
    /// Une session : un agent et un objectif, dans un projet.
    ///
    /// Les sessions `human` (l'utilisateur dans son editeur) et `external`
    /// (build, formatter, script) sont des sessions comme les autres. Cette
    /// uniformite supprime une categorie entiere de cas particuliers.
    SessionId
}

impl SessionId {
    /// La session conventionnelle des ecritures **hors-bande**.
    ///
    /// `sed -i`, un hook git, un formatter, un build, ou l'utilisateur dans son editeur :
    /// tout ce qui touche l'arbre sans passer par l'admission. Le watcher FSEvents attribue
    /// ces ecritures a cet identifiant.
    ///
    /// # Pourquoi une session et pas une absence de session
    ///
    /// Parce que le registre doit pouvoir dire « ce file a change, et pas par toi ». Une
    /// ecriture sans auteur ne perimerait rien : la comparaison `last_writer == session`
    /// n'aurait pas de sens. Les handle comme une session comme les autres supprime une
    /// categorie entiere de cas particuliers — c'est le meme choix que `Harness::External`.
    ///
    /// L'UUID est fixe et documente : il doit etre reconnaissable dans le journal, et stable
    /// entre les executions.
    pub const EXTERNAL: Self = Self(Uuid::from_u128(0x7242_414d_4500_0000_0000_0000_0000_0001));

    /// Vrai si cet identifiant designe les ecritures hors-bande.
    #[must_use]
    pub fn is_external(&self) -> bool {
        *self == Self::EXTERNAL
    }
}

/// Le numero de sequence d'une ecriture admise.
///
/// **Local au projet, jamais global.** Un compteur global serait un point de
/// contention entre projets qui, par construction, ne peuvent pas entrer en
/// collision — et il rendrait le journal illisible en transverse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Seq(u64);

impl Seq {
    /// Le premier numero de sequence d'un projet.
    pub const FIRST: Self = Self(1);

    /// Le numero suivant.
    #[must_use]
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// La valeur brute, pour la persistance.
    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }

    /// Reconstruit depuis le journal.
    #[must_use]
    pub fn from_u64(value: u64) -> Self {
        Self(value)
    }
}

impl fmt::Display for Seq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Le nom lisible d'une branche virtuelle. Ce que l'utilisateur voit.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BranchName(String);

impl BranchName {
    /// Construit un nom de branche.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Le nom brut.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BranchName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Declare un identifiant opaque adosse a une chaine fournie par un tiers.
macro_rules! opaque_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Construit l'identifiant depuis la valeur fournie par le tiers.
            #[must_use]
            pub fn new(raw: impl Into<String>) -> Self {
                Self(raw.into())
            }

            /// La valeur brute.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

opaque_id! {
    /// L'identifiant stable d'une branche cote GitButler, tel que rendu par
    /// `but status --format json`. Opaque : on ne le construit jamais soi-meme.
    BranchId
}

opaque_id! {
    /// Une *change request* : merge request GitLab, pull request GitHub.
    ///
    /// Le nom est neutre a dessein. GitLab est la target primaire, pas un
    /// citoyen de seconde zone.
    CrId
}

opaque_id! {
    /// Un thread de discussion sur une change request.
    ThreadId
}

opaque_id! {
    /// Un element de travail chez sa source : numero d'issue, id de thread,
    /// key de ticket. La forme depend de la source, d'ou l'opacite.
    WorkItemId
}
