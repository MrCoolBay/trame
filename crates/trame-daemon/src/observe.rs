//! Le canal d'observation. **Un sens unique : le daemon parle, l'interface ecoute.**
//!
//! # Pourquoi un canal et pas un accesseur
//!
//! L'interface pourrait interroger [`trame_registry::RegistryHandle::snapshot`] en run_loop.
//! Deux raisons de ne pas le faire, et la seconde est la vraie :
//!
//! 1. Un snapshot donne l'**state**, pas les **evenements**. Il ne dit pas qu'un verdict
//!    `StaleRead` a ete rendu, seulement qu'un file a un dernier ecrivain.
//! 2. Un accesseur invite a l'ecriture. Si l'interface tient un `RegistryHandle`, rien ne
//!    l'empeche d'appeler `admit`. **La TUI observe, elle ne pilot pas** — et la facon de
//!    le garantir est structurelle : elle ne recoit qu'un `Receiver`.
//!
//! # Perdre une observation est acceptable. Perdre une admission ne l'est pas.
//!
//! L'ADR 0015 dit qu'un canal limit sature et qu'on attend, parce qu'une saturation du
//! path d'admission est un bug qu'il faut voir. **Ce canal-ci est l'exception, et
//! l'exception est justifiee par la direction du feed** : si l'interface prend du retard,
//! faire wait_for le registre reviendrait a laisser l'affichage ralentir les ecritures d'un
//! agent. Le remede serait pire que le mal.
//!
//! Donc [`Observer::emit`] ne bloque jamais — il compte ce qu'il perd, et le dit a la
//! premiere occasion via [`Observation::Lost`]. Une perte silencieuse afficherait un feed
//! incomplet en le presentant comme complet, ce qui est exactement le mode d'echec que ce
//! projet refuse partout ailleurs.

use std::path::PathBuf;

use tokio::sync::mpsc;
use trame_agent::Capabilities;
use trame_core::{SessionId, SessionState, Verdict};

/// Capacite du canal d'observation.
///
/// Genereuse a dessein : la limit n'est pas la pour appliquer une contre-pression — on ne
/// veut pas en appliquer ici — mais pour empecher une interface bloquee de faire grossir
/// une file sans fin.
pub const OBSERVE_CAPACITY: usize = 256;

/// Par quel transport une session est pilotee, **et donc ce qui est garanti**.
///
/// Vit ici plutot que dans `trame-agent` pour que l'interface puisse la nommer sans
/// dependre du crate agent : la direction de dependance reste
/// `core <- journal <- registry <- {agent, vcs} <- daemon <- tui`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Transport {
    /// ACP. Les ecritures passent par l'admission avant le disque.
    Acp,
    /// PTY. **Mode degrade** : rien n'est interceptable avant le disque.
    Pty,
    /// Aucun agent attache. L'state d'une session que personne ne pilot.
    Absent,
}

impl Transport {
    /// Vrai si les ecritures de cette session sont interceptables avant le disque.
    #[must_use]
    pub const fn can_intercept_writes(self) -> bool {
        matches!(self, Self::Acp)
    }

    /// Vrai si l'interface **doit** afficher une banniere de degradation.
    ///
    /// Un utilisateur qui croit avoir la garantie d'admission sans l'avoir est dans une
    /// situation pire que sans outil.
    #[must_use]
    pub const fn is_degraded(self) -> bool {
        !self.can_intercept_writes()
    }

    /// Le libelle affichable.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Acp => "ACP",
            Self::Pty => "PTY",
            Self::Absent => "aucun",
        }
    }
}

impl From<Capabilities> for Transport {
    /// Deduit le transport des capacites **reelles** du backend.
    ///
    /// On ne devine pas depuis le type du backend au point d'appel : c'est
    /// `can_intercept_writes` qui decide, et lui seul.
    fn from(capabilities: Capabilities) -> Self {
        if capabilities.can_intercept_writes {
            Self::Acp
        } else {
            Self::Pty
        }
    }
}

/// Ce que le daemon donne a voir. **Rien de plus que ce que l'interface affiche.**
///
/// Deliberement pauvre : chaque variante ajoutee ici est une chose que l'interface pourra
/// montrer, donc une promesse faite a l'utilisateur.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Observation {
    /// Une session apparait, avec le transport qui la pilot.
    SessionOpened {
        /// Son identifiant.
        session: SessionId,
        /// Son nom affichable.
        name: String,
        /// Ce qui est garanti pour elle.
        transport: Transport,
    },
    /// Son state a change.
    StateChanged {
        /// La session concernee.
        session: SessionId,
        /// Son nouvel state.
        state: SessionState,
    },
    /// Une lecture est entree dans le read-set.
    Read {
        /// La session qui a lu.
        session: SessionId,
        /// Le path lu, relatif a la root du projet.
        path: PathBuf,
    },
    /// Une ecriture **admise**, avec le verdict rendu.
    Write {
        /// La session qui a ecrit.
        session: SessionId,
        /// Le path ecrit, relatif a la root du projet.
        path: PathBuf,
        /// Le verdict. `StaleRead` est le seul qui merite d'etre vu.
        verdict: Verdict,
    },
    /// Une ecriture **refusee**, avec le reason transmis a l'agent.
    Refused {
        /// La session qui a demande.
        session: SessionId,
        /// Le path refuse.
        path: PathBuf,
        /// Le reason, tel que l'agent l'a recu.
        reason: String,
    },
    /// Un avis pose devant le prochain message de la session.
    Notice {
        /// La session avertie.
        session: SessionId,
        /// Le texte exact injecte.
        text: String,
    },
    /// Une ecriture **hors-bande**, constatee par le watcher apres coup.
    ///
    /// **Sans verdict, et l'interface ne doit pas en inventer un** : personne ne l'a
    /// admise. Le watcher constate, il n'empeche rien.
    ExternalWrite {
        /// Le path observe, relatif a la root du projet.
        path: PathBuf,
    },
    /// ★ Des avis que les lectures `Grep` **auraient** produits, si elles comptaient.
    ///
    /// **Ce ne sont pas des avis.** Rien n'a ete injecte, aucun agent n'a ete averti. C'est la
    /// donnee manquante pour decider si le trou lecture peut se fermer sans crier au loup
    /// (ADR 0027), et l'interface doit l'afficher **distinctement** des avis reels — sinon elle
    /// annonce une couverture qui n'existe pas.
    PotentialNotices {
        /// Le cumul depuis le demarrage du projet.
        total: u64,
    },
    /// Des observations ont ete perdues faute de place.
    ///
    /// L'interface l'affiche : un feed troue presente comme complet serait un mensonge.
    Lost {
        /// Combien.
        count: u64,
    },
}

/// L'extremite d'emission du canal d'observation.
///
/// Se clone : chaque pilot de session et le watcher en tiennent un.
#[derive(Debug)]
pub struct Observer {
    tx: mpsc::Sender<Observation>,
    /// Ce qui n'a pas pu etre transmis, en attente d'etre signale.
    dropped: u64,
}

impl Clone for Observer {
    /// Le compteur de pertes **ne se clone pas** : il appartient a l'emetteur qui a perdu.
    /// Le dupliquer ferait compter deux fois la meme perte.
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            dropped: 0,
        }
    }
}

/// Cree le canal d'observation.
///
/// Rend l'emetteur au daemon et le recepteur a l'interface. **Le recepteur ne permet
/// rien d'autre que d'ecouter**, et c'est la garantie que la TUI ne pilot pas.
#[must_use]
pub fn observe_channel() -> (Observer, mpsc::Receiver<Observation>) {
    let (tx, rx) = mpsc::channel(OBSERVE_CAPACITY);
    (Observer { tx, dropped: 0 }, rx)
}

impl Observer {
    /// Transmet une observation. **Ne bloque jamais, ne rend pas d'erreur.**
    ///
    /// Si le canal est plein ou ferme, l'observation est perdue et comptee. Le compte est
    /// transmis des qu'une place se libere, via [`Observation::Lost`].
    pub fn emit(&mut self, observation: Observation) {
        // Les pertes passent d'abord : sinon un compteur monterait sans jamais s'afficher,
        // et le trou resterait invisible — precisement ce qu'on veut eviter.
        if self.dropped > 0
            && self
                .tx
                .try_send(Observation::Lost {
                    count: self.dropped,
                })
                .is_ok()
        {
            self.dropped = 0;
        }
        if self.tx.try_send(observation).is_err() {
            self.dropped = self.dropped.saturating_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_transport_pty_est_degrade_et_acp_ne_l_est_pas() {
        assert!(Transport::Pty.is_degraded(), "PTY n'intercepte rien");
        assert!(
            Transport::Absent.is_degraded(),
            "sans agent, rien n'est garanti"
        );
        assert!(!Transport::Acp.is_degraded());
        assert!(Transport::Acp.can_intercept_writes());
    }

    #[test]
    fn le_transport_se_deduit_des_capacites_reelles() {
        assert_eq!(Transport::from(Capabilities::acp()), Transport::Acp);
        assert_eq!(Transport::from(Capabilities::pty()), Transport::Pty);
    }

    #[tokio::test]
    async fn une_saturation_ne_bloque_pas_et_se_declare() {
        let (mut observer, mut rx) = observe_channel();
        let path = PathBuf::from("auth.rs");

        // On remplit exactement le canal, puis on depasse de trois.
        for _ in 0..OBSERVE_CAPACITY + 3 {
            observer.emit(Observation::ExternalWrite { path: path.clone() });
        }
        assert_eq!(observer.dropped, 3, "trois observations perdues, comptees");

        // On libere une seule place. Elle sert a **declarer la perte**, pas a transmettre
        // l'observation suivante — qui est donc perdue a son turn. C'est le bon ordre :
        // mieux vaut savoir qu'on ne sait pas.
        rx.recv().await.unwrap();
        observer.emit(Observation::ExternalWrite { path: path.clone() });
        let mut vues = Vec::new();
        while let Ok(observation) = rx.try_recv() {
            vues.push(observation);
        }
        assert!(
            vues.contains(&Observation::Lost { count: 3 }),
            "une perte silencieuse presenterait un feed troue comme complet"
        );
        assert_eq!(
            observer.dropped, 1,
            "la nouvelle perte est comptee a son turn"
        );

        // Canal vide : l'emission suivante passe, et le compteur se solde.
        observer.emit(Observation::ExternalWrite { path });
        assert_eq!(rx.try_recv().unwrap(), Observation::Lost { count: 1 });
        assert!(matches!(
            rx.try_recv().unwrap(),
            Observation::ExternalWrite { .. }
        ));
        assert_eq!(
            observer.dropped, 0,
            "le compteur se solde quand la place revient"
        );
    }

    /// Un `Observer` clone ne doit pas heriter des pertes de son parent : la meme perte
    /// serait signalee deux fois, et l'interface afficherait un trou qui n'existe pas.
    #[test]
    fn un_clone_ne_herite_pas_des_pertes() {
        let (mut observer, _rx) = observe_channel();
        for _ in 0..OBSERVE_CAPACITY + 1 {
            observer.emit(Observation::Lost { count: 1 });
        }
        assert_eq!(observer.dropped, 1);
        assert_eq!(observer.clone().dropped, 0);
    }

    /// Un canal ferme ne doit pas faire paniquer l'emetteur : l'interface peut se fermer
    /// pendant qu'une session tourne, et ce n'est pas une erreur du daemon.
    #[tokio::test]
    async fn un_recepteur_ferme_ne_casse_rien() {
        let (mut observer, rx) = observe_channel();
        drop(rx);
        observer.emit(Observation::Lost { count: 1 });
        assert_eq!(observer.dropped, 1);
    }
}
