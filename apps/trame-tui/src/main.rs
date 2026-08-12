//! TUI Trame. **Elle observe, elle ne pilote pas.**
//!
//! # Perimetre v0.1, et rien de plus
//!
//! - un panneau par session, avec son etat — `Idle` / `Thinking` / `Writing`
//! - le flux d'evenements en direct, verdicts mis en evidence
//! - les `StaleRead` visuellement distincts des `Clean`
//! - la distinction **admis / observe** visible : le watcher constate apres coup, et
//!   l'interface ne doit pas laisser croire l'inverse
//! - un indicateur de degradation quand `can_intercept_writes` est faux
//!
//! Pas de vue multi-projet, pas de gestion de branches, pas de diffs, pas de
//! configuration. Ce qui n'est pas dans cette liste n'est pas dans la v0.1.
//!
//! # Pourquoi elle ne peut structurellement pas piloter
//!
//! Elle ne recoit qu'un `Receiver<Observation>` (`trame_daemon::observe`). Elle ne tient
//! aucun `RegistryHandle`, donc `admit` ne lui est pas accessible. La garantie n'est pas
//! une convention de revue, c'est le typage.
//!
//! # Usage
//!
//! ```sh
//! trame-tui [chemin-du-projet] [--scenario]
//! ```
//!
//! `--scenario` fait passer le scenario canonique par le vrai registre, sans agent, pour
//! voir un `StaleRead` reel a l'ecran. Les verdicts affiches sont ceux du registre.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

use trame_core::clock::SystemClock;
use trame_tui::app::App;
use trame_tui::{run, source};

#[tokio::main]
async fn main() -> Result<()> {
    // Les logs vont sur stderr : stdout appartient au terminal alternatif de ratatui, et y
    // ecrire corromprait l'affichage.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "trame=info".into()))
        .with_writer(std::io::stderr)
        .init();

    let (racine, scenario) = arguments()?;
    if scenario {
        source::refuser_racine_dangereuse(&racine)?;
    }
    let clock = Arc::new(SystemClock);

    // **Le terminal d'abord, le projet ensuite.** Ouvrir le projet demarre le watcher et,
    // en mode scenario, ecrit dans le repertoire de travail. Une interface qui ne peut pas
    // s'afficher — pas de TTY, sortie redirigee — ne doit rien avoir touche.
    let mut terminal = ratatui::try_init().context("terminal indisponible")?;
    let resultat = async {
        let mut source = source::open(&racine, clock.clone(), scenario)
            .await
            .context("ouverture du projet")?;
        let mut etat = App::new(source.project.clone(), clock);
        let mut touches = run::spawn_touches();
        run::afficher(
            &mut terminal,
            &mut etat,
            &mut source.observations,
            &mut touches,
        )
        .await
        .context("rendu interrompu")
    }
    .await;
    // Restaurer avant de propager : une erreur affichee dans le terminal alternatif est
    // une erreur que personne ne lit.
    ratatui::try_restore().ok();
    resultat
}

fn arguments() -> Result<(PathBuf, bool)> {
    let mut racine = None;
    let mut scenario = false;
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--scenario" => scenario = true,
            "-h" | "--help" => {
                // Pas de `println!` — deni par clippy, et stdout appartient a ratatui.
                tracing::info!("usage : trame-tui [chemin-du-projet] [--scenario]");
                std::process::exit(0);
            }
            autre => racine = Some(PathBuf::from(autre)),
        }
    }
    // Le repertoire courant est un defaut acceptable pour **observer**. Il ne l'est pas pour
    // `--scenario`, qui ecrit : voir `refuser_racine_dangereuse`.
    if scenario && racine.is_none() {
        anyhow::bail!(
            "--scenario ecrit dans le projet : le chemin doit etre donne explicitement.\n\
             usage : trame-tui <chemin-du-projet> --scenario"
        );
    }
    let racine = match racine {
        Some(chemin) => chemin,
        None => std::env::current_dir().context("repertoire courant illisible")?,
    };
    Ok((racine, scenario))
}
