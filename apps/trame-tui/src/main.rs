//! TUI Trame. **Elle observe, elle ne pilot pas.**
//!
//! # Perimetre v0.1, et rien de plus
//!
//! - un panel par session, avec son state — `Idle` / `Thinking` / `Writing`
//! - le feed d'evenements en direct, verdicts mis en evidence
//! - les `StaleRead` visuellement distincts des `Clean`
//! - la distinction **admis / observe** visible : le watcher constate apres coup, et
//!   l'interface ne doit pas laisser croire l'inverse
//! - un indicateur de degraded_banner quand `can_intercept_writes` est faux
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
//! trame-tui [path-du-projet] [--scenario]
//! ```
//!
//! `--scenario` fait passer le scenario canonique par le vrai registre, sans agent, pour
//! voir un `StaleRead` reel a l'ecran. Les verdicts affiches sont ceux du registre.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

use trame_core::clock::SystemClock;
use trame_tui::{run, source};
use trame_view::App;

#[tokio::main]
async fn main() -> Result<()> {
    // Les logs vont sur stderr : stdout appartient au terminal alternatif de ratatui, et y
    // ecrire corromprait l'affichage.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "trame=info".into()))
        .with_writer(std::io::stderr)
        .init();

    let (root, scenario) = parse_args()?;
    if scenario {
        source::refuse_dangerous_root(&root)?;
    }
    let clock = Arc::new(SystemClock);

    // **Le terminal d'abord, le projet ensuite.** Ouvrir le projet demarre le watcher et,
    // en mode scenario, ecrit dans le repertoire de travail. Une interface qui ne peut pas
    // s'afficher — pas de TTY, sortie redirigee — ne doit rien avoir touche.
    let mut terminal = ratatui::try_init().context("terminal indisponible")?;
    let resultat = async {
        let mut source = source::open(&root, clock.clone(), scenario)
            .await
            .context("ouverture du projet")?;
        let mut state = App::new(source.project.clone(), clock);
        let mut touches = run::spawn_keys();
        run::render_loop(
            &mut terminal,
            &mut state,
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

fn parse_args() -> Result<(PathBuf, bool)> {
    let mut root = None;
    let mut scenario = false;
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--scenario" => scenario = true,
            "-h" | "--help" => {
                // Pas de `println!` — deni par clippy, et stdout appartient a ratatui.
                tracing::info!("usage : trame-tui [path-du-projet] [--scenario]");
                std::process::exit(0);
            }
            autre => root = Some(PathBuf::from(autre)),
        }
    }
    // Le repertoire courant est un defaut acceptable pour **observer**. Il ne l'est pas pour
    // `--scenario`, qui ecrit : voir `refuse_dangerous_root`.
    if scenario && root.is_none() {
        anyhow::bail!(
            "--scenario ecrit dans le projet : le path doit etre donne explicitement.\n\
             usage : trame-tui <path-du-projet> --scenario"
        );
    }
    let root = match root {
        Some(path) => path,
        None => std::env::current_dir().context("repertoire courant illisible")?,
    };
    Ok((root, scenario))
}
