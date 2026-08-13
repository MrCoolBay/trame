//! D'ou viennent les observations. **Rien n'est simule.**
//!
//! Le vrai journal SQLite, le vrai registre, le vrai watcher FSEvents. Ouvrir un projet
//! dans l'interface, c'est ouvrir ce que le daemon ouvrirait — et une ecriture hors-bande
//! faite a la main dans le repertoire apparait pour de vrai dans le feed.
//!
//! # Ce que la v0.1 ne fait pas ici
//!
//! **Elle ne lance pas d'agent.** Il n'y a pas encore de service de session dans le daemon,
//! donc aucune session ACP n'est opened par ce binaire. Les panels de session
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

/// Un projet ouvert, et le feed de ce qui s'y passe.
///
/// Tant que cette valeur vit, le watcher surveille. La relacher, c'est fermer le projet.
pub struct Source {
    /// Le feed d'observations. **Lecture seule : l'interface ne pilot pas.**
    pub observations: mpsc::Receiver<Observation>,
    /// Le nom affichable du projet.
    pub project: String,
    _watcher: WatcherGuard,
    _taches: Vec<JoinHandle<()>>,
}

/// Refuse une root dans laquelle le scenario n'a rien a ecrire.
///
/// **Le scenario ecrit dans le projet vise** — c'est tout son interet, les verdicts affiches
/// sont ceux d'ecritures reelles. Le corollaire est qu'une root choisie par defaut est
/// dangereuse : lancer le mode scenario depuis un depot y depose `auth.rs` et `handlers.rs`.
///
/// C'est arrive. Deux fichiers du scenario se sont retrouves a la root de ce depot, et je
/// n'ai pas pu reproduire l'invocation exacte — raison de plus pour fermer la classe
/// d'accident au lieu de chercher le coupable. Un depot qui contient `.git` n'est pas un
/// bac a sable.
///
/// # Erreurs
///
/// Echoue si la root contient un `.git`.
pub fn refuse_dangerous_root(root: &Path) -> Result<()> {
    if root.join(".git").exists() {
        anyhow::bail!(
            "{} contient un .git : le mode scenario y ecrirait auth.rs et handlers.rs.\n\
             Passe un repertoire jetable, par exemple /tmp/trame-demo.",
            root.display()
        );
    }
    Ok(())
}

/// Ouvre un projet : journal, registre, watcher.
///
/// # Erreurs
///
/// Echoue si la root n'existe pas, si le journal SQLite est inouvrable, ou si FSEvents
/// refuse de surveiller le repertoire.
pub async fn open(root: &Path, clock: Arc<dyn Clock>, scenario: bool) -> Result<Source> {
    // Le mode scenario **cree** son bac a sable. Il ecrit de toute facon dedans, et exiger
    // qu'il existe deja n'apporte aucune securite : la guard utile est ailleurs, dans
    // `refuse_dangerous_root`, qui refuse un depot. Sans ca, un `--scenario /tmp/demo`
    // echoue sur « root de projet invalide » — ce qui est vrai et inutile.
    //
    // L'observation, elle, continue d'exiger un projet **existant** : observer un repertoire
    // qu'on vient de creer, c'est observer le vide, et c'est plus probablement une faute de
    // frappe dans le path.
    if scenario && !root.exists() {
        std::fs::create_dir_all(root)
            .with_context(|| format!("bac a sable impossible a creer : {}", root.display()))?;
        tracing::info!(root = %root.display(), "bac a sable cree pour le scenario");
    }
    let root = ProjectRoot::new(root)
        .with_context(|| format!("root de projet invalide : {}", root.display()))?;
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
    let (tache_watcher, guard) = trame_daemon::spawn_watcher_observed(
        root.clone(),
        registry.clone(),
        Some(observer.clone()),
    )
    .context("FSEvents refuse de surveiller cette root")?;

    let mut tasks = vec![tache_journal, tache_registre, tache_watcher];
    if scenario {
        tasks.push(tokio::spawn(play_scenario(registry, observer)));
    }

    Ok(Source {
        observations,
        project: nom,
        _watcher: guard,
        _taches: tasks,
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
async fn play_scenario(registry: RegistryHandle, mut observer: Observer) {
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
    // Ecrire le file dans le dos du registre en ferait une vraie ecriture hors-bande,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Un depot n'est pas un bac a sable. Le scenario doit deny d'y ecrire.
    #[test]
    fn the_scenario_mode_refuses_a_git_repository() {
        let root = std::env::temp_dir().join(format!("trame-guard-{}", ProjectId::new()));
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let erreur = refuse_dangerous_root(&root).unwrap_err().to_string();
        assert!(
            erreur.contains(".git"),
            "le reason doit dire pourquoi : {erreur}"
        );
        assert!(
            erreur.contains("auth.rs"),
            "et ce qui serait ecrit : {erreur}"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// Le mode scenario cree son bac a sable.
    ///
    /// Ce test existe parce que le cas a echoue en vrai : `just gui-scenario /tmp/trame-demo`
    /// sur un path qui n'existait pas encore rendait « root de projet invalide », alors
    /// que l'ADR conseillait justement ce path-la.
    #[tokio::test]
    async fn the_scenario_mode_creates_its_own_sandbox() {
        let root = std::env::temp_dir().join(format!("trame-bac-{}", ProjectId::new()));
        assert!(!root.exists());
        // On n'ouvre pas le projet entier ici — le journal reel et le watcher n'ont rien a
        // voir avec la question. On verifie la seule chose qui a casse : le repertoire.
        let echec = open(&root, Arc::new(trame_core::clock::SystemClock), true)
            .await
            .err()
            .map(|e| format!("{e:#}"));
        assert!(
            root.is_dir(),
            "le bac a sable doit exister apres l'ouverture (erreur : {echec:?})"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// Controle negatif : sans `--scenario`, un repertoire absent reste une erreur.
    ///
    /// Observer un repertoire qu'on vient de creer, c'est observer le vide — et c'est plus
    /// probablement une faute de frappe dans le path.
    #[tokio::test]
    async fn observing_a_directory_that_does_not_exist_stays_an_error() {
        let root = std::env::temp_dir().join(format!("trame-absent-{}", ProjectId::new()));
        let erreur = open(&root, Arc::new(trame_core::clock::SystemClock), false).await;
        assert!(erreur.is_err(), "observer le vide doit echouer");
        assert!(!root.exists(), "et ne doit rien creer");
    }

    /// Controle negatif : un repertoire jetable passe, sinon la guard bloquerait l'usage
    /// normal et serait contournee dans la semaine.
    #[test]
    fn a_throwaway_directory_is_accepted() {
        let root = std::env::temp_dir().join(format!("trame-guard-{}", ProjectId::new()));
        std::fs::create_dir_all(&root).unwrap();
        assert!(refuse_dangerous_root(&root).is_ok());
        std::fs::remove_dir_all(&root).ok();
    }
}
