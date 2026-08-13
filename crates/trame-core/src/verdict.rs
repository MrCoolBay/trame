//! Les verdicts d'admission.
//!
//! Le type vit ici, dans le crate fondation, parce que trois crates en ont
//! besoin : `trame-registry` le produit, `trame-journal` le persiste,
//! `trame-tui` l'affiche. La *logique* qui le calcule, elle, n'existe qu'a un
//! seul endroit — l'acteur du registre.
//!
//! # Rien n'est bloque en v0.1
//!
//! Le registre observe, journalise et informe. Le blocage viendra quand on aura
//! mesure le taux reel de faux positifs sur du vrai usage. Un outil qui crie au
//! loup est desactive en une semaine.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;
use crate::ids::{Seq, SessionId};

/// Le resultat d'une demande d'admission en ecriture.
///
/// Quatre niveaux, pas un booleen : la bonne reponse a une collision n'est pas
/// toujours « non ».
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Verdict {
    /// Niveau 0. Aucun recouvrement. ~95 % du trafic. Silencieux.
    Clean,

    /// Niveau 1. Un file du read-set de cette session a change depuis sa
    /// lecture, par une autre session.
    ///
    /// **Admis**, et un avis est injecte dans le contexte de l'agent. C'est le
    /// seul mecanisme du produit qui n'existe nulle part ailleurs : on ne sait
    /// pas *si* ca casse, mais on sait que l'agent raisonne sur un monde qui
    /// n'existe plus.
    StaleRead {
        /// Les fichiers perimes, du plus recemment modifie au plus ancien.
        stale: Vec<StaleFile>,
    },

    /// Niveau 2. Meme file, regions disjointes. Admis.
    ///
    /// **Non implemente en v0.1** : la granularite est le file entier, donc
    /// ce cas n'est jamais produit. La variante existe pour que l'ajouter en
    /// v0.4 soit un `match` a completer et non un changement de type public.
    DisjointWrite {
        /// La session qui a ecrit l'autre region.
        other: SessionId,
    },

    /// Niveau 3. Regions qui se recouvrent. Bloque, on demande a l'humain via le
    /// mecanisme de permission ACP existant.
    ///
    /// **Non implemente en v0.1**, meme raison que [`Verdict::DisjointWrite`].
    Overlap {
        /// La session avec laquelle il y a recouvrement.
        other: SessionId,
    },
}

impl Verdict {
    /// Le libelle stable stocke dans la colonne `writes.verdict`.
    /// Ne jamais le changer sans migration : le journal est append-only.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::StaleRead { .. } => "stale_read",
            Self::DisjointWrite { .. } => "disjoint_write",
            Self::Overlap { .. } => "overlap",
        }
    }

    /// Le niveau numerique, 0 a 3.
    #[must_use]
    pub fn level(&self) -> u8 {
        match self {
            Self::Clean => 0,
            Self::StaleRead { .. } => 1,
            Self::DisjointWrite { .. } => 2,
            Self::Overlap { .. } => 3,
        }
    }

    /// Vrai si l'ecriture est admise.
    ///
    /// En v0.1 : toujours vrai, y compris pour [`Verdict::Overlap`], qui n'est
    /// de toute facon jamais produit. Le blocage se decidera apres mesure.
    #[must_use]
    pub fn is_admitted(&self) -> bool {
        !matches!(self, Self::Overlap { .. })
    }

    /// Vrai si l'agent doit etre informe. C'est le seul verdict qui declenche
    /// une injection de contexte.
    #[must_use]
    pub fn needs_notice(&self) -> bool {
        matches!(self, Self::StaleRead { .. })
    }
}

/// Un file lu par une session, modifie depuis par une autre.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaleFile {
    /// Son path, relatif a la root du projet.
    pub path: PathBuf,
    /// La session qui l'a modifie.
    pub last_writer: SessionId,
    /// Le nom affichable de cette session. Un UUID ne dit rien a un agent ;
    /// « refacto-api » lui dit quelque chose.
    pub last_writer_name: String,
    /// Quand la session courante l'avait lu.
    pub read_at: Timestamp,
    /// Quand l'autre session l'a modifie.
    pub written_at: Timestamp,
    /// Le numero de sequence de cette modification, local au projet.
    pub seq: Seq,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_levels_are_ordered() {
        let session = SessionId::new();
        assert_eq!(Verdict::Clean.level(), 0);
        assert_eq!(Verdict::StaleRead { stale: vec![] }.level(), 1);
        assert_eq!(Verdict::DisjointWrite { other: session }.level(), 2);
        assert_eq!(Verdict::Overlap { other: session }.level(), 3);
    }

    #[test]
    fn stale_read_is_admitted_and_notified() {
        let verdict = Verdict::StaleRead { stale: vec![] };
        assert!(
            verdict.is_admitted(),
            "le niveau 1 informe, il ne bloque pas"
        );
        assert!(verdict.needs_notice());
    }

    #[test]
    fn clean_says_nothing() {
        assert!(Verdict::Clean.is_admitted());
        assert!(
            !Verdict::Clean.needs_notice(),
            "95 % du trafic doit passer sans un mot"
        );
    }
}
