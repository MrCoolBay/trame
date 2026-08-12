//! D'ou viennent les observations. **Rien n'est simule.**
//!
//! Le vrai journal SQLite, le vrai registre, le vrai watcher FSEvents. Ouvrir un projet
//! dans l'interface, c'est ouvrir ce que le daemon ouvrirait — et une ecriture hors-bande
//! faite a la main dans le repertoire apparait pour de vrai dans le flux.
//!
//! # Ce que la v0.1 ne fait pas ici
//!
//! **Elle ne lance pas d'agent.** Il n'y a pas encore de service de session dans le daemon,
//! donc aucune session ACP n'est ouverte par ce binaire. Les panneaux de session
//! apparaissent quand un [`trame_daemon::SessionPilot`] est observe — ce que fait
//! `--scenario`, en faisant passer le scenario canonique par le **vrai registre**, sans
//! agent.
//!
//! C'est une limite, pas un decor : le transport affiche est alors `aucun`, parce que
//! c'est la verite. Afficher `ACP` pour faire joli reviendrait a promettre une garantie que
//! rien ne fournit.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use trame_core::clock::Clock;
use trame_core::{ProjectId, ProjectRoot, SessionId};
use trame_daemon::{Observation, Observer, Transport, WatcherGuard, observe_channel};
use trame_journal::{Journal, spawn_journal};
use trame_registry::{ReadKind, RegistryHandle, spawn_registry};

/// Un projet ouvert, et le flux de ce qui s'y passe.
///
/// Tant que cette valeur vit, le watcher surveille. La relacher, c'est fermer le projet.
pub struct Source {
    /// Le flux d'observations. **Lecture seule : l'interface ne pilote pas.**
    pub observations: mpsc::Receiver<Observation>,
    /// Le nom affichable du projet.
    pub project: String,
    _watcher: WatcherGuard,
    _taches: Vec<JoinHandle<()>>,
}

/// Ouvre un projet : journal, registre, watcher.
///
/// # Erreurs
///
/// Echoue si la racine n'existe pas, si le journal SQLite est inouvrable, ou si FSEvents
/// refuse de surveiller le repertoire.
pub async fn open(root: &Path, clock: Arc<dyn Clock>, scenario: bool) -> Result<Source> {
    let root = ProjectRoot::new(root)
        .with_context(|| format!("racine de projet invalide : {}", root.display()))?;
    let nom = root.as_path().file_name().map_or_else(
        || root.as_path().display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );

    let journal = Journal::open_default().context("journal SQLite inouvrable")?;
    let (journal, tache_journal) = spawn_journal(journal);
    let project = ProjectId::new();
    let (registry, tache_registre) =
        spawn_registry(project, root.clone(), clock.clone(), journal.clone());

    let (observer, observations) = observe_channel();
    let (tache_watcher, garde) = trame_daemon::spawn_watcher_observed(
        root.clone(),
        registry.clone(),
        Some(observer.clone()),
    )
    .context("FSEvents refuse de surveiller cette racine")?;

    let mut taches = vec![tache_journal, tache_registre, tache_watcher];
    if scenario {
        taches.push(tokio::spawn(joue_scenario(registry, observer)));
    }

    Ok(Source {
        observations,
        project: nom,
        _watcher: garde,
        _taches: taches,
    })
}

/// Fait passer le scenario canonique par le **vrai** registre, sans agent.
///
/// A lit `auth.rs`, B ecrit `auth.rs` (→ `Clean`), A ecrit `handlers.rs`
/// (→ `StaleRead`). Deux fichiers differents, aucune collision d'ecriture : c'est le mode
/// d'echec que Trame existe pour attraper, et ce que l'interface doit rendre evident.
///
/// Les verdicts affiches sont ceux du registre. **Rien n'est fabrique ici** — sinon
/// l'interface validerait sa propre fiction, ce que ce projet a deja paye deux fois.
async fn joue_scenario(registry: RegistryHandle, mut observer: Observer) {
    let (a, b) = (SessionId::new(), SessionId::new());
    for (session, nom) in [(a, "scenario-a"), (b, "scenario-b")] {
        if registry.register_session(session, nom).await.is_err() {
            return;
        }
        observer.emit(Observation::SessionOpened {
            session,
            name: nom.to_owned(),
            // La verite : aucun agent n'est attache. Donc rien n'est intercepte.
            transport: Transport::Absent,
        });
    }

    let auth = PathBuf::from("auth.rs");
    let handlers = PathBuf::from("handlers.rs");
    let v1 = "pub fn verify_token(token: &str) -> bool { !token.is_empty() }\n";

    // 0. La fixture passe par l'admission, pas par un `fs::write` direct.
    //
    // Ecrire le fichier dans le dos du registre en ferait une vraie ecriture hors-bande,
    // que le watcher signalerait a juste titre — et le compteur « hors-bande » de
    // l'interface afficherait 1 sans que l'utilisateur ait rien fait. Selon la course entre
    // FSEvents et l'ecriture suivante, il afficherait 0 ou 1 : un affichage non
    // deterministe sur le compteur qui doit precisement rester croyable.
    if let Ok(verdict) = registry.admit(a, auth.clone(), v1).await {
        observer.emit(Observation::Write {
            session: a,
            path: auth.clone(),
            verdict,
        });
    }

    // 1. A lit auth.rs. C'est ce qui remplit le read-set.
    if registry
        .record_read(a, auth.clone(), v1, ReadKind::FullFile)
        .await
        .is_ok()
    {
        observer.emit(Observation::Read {
            session: a,
            path: auth.clone(),
        });
    }

    // 2. B ecrit auth.rs. Personne n'a lu ce que B ecrase : Clean.
    if let Ok(verdict) = registry
        .admit(
            b,
            auth.clone(),
            "pub fn validate_token(t: &str) -> bool { !t.is_empty() }\n",
        )
        .await
    {
        observer.emit(Observation::Write {
            session: b,
            path: auth,
            verdict,
        });
    }

    // 3. A ecrit handlers.rs. Fichier different, et pourtant StaleRead.
    if let Ok(verdict) = registry
        .admit(a, handlers.clone(), "verify_token(t)\n")
        .await
    {
        observer.emit(Observation::Write {
            session: a,
            path: handlers,
            verdict,
        });
    }
}
