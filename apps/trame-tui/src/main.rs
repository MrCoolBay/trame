//! TUI Trame.
//!
//! Cible de la phase 3 : deux panneaux de session avec leur etat
//! (Idle / Thinking / Writing), le flux d'evenements en direct, et les verdicts
//! mis en evidence — les `StaleRead` visuellement distincts des autres.
//!
//! L'UI est interchangeable. Le core est le produit.

use anyhow::Result;
use tracing::info;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    // Les logs vont sur stderr : stdout appartient au terminal alternatif de
    // ratatui, et y ecrire corromprait l'affichage.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "trame=info".into()))
        .with_writer(std::io::stderr)
        .init();

    info!("trame-tui : phase 0. Aucune interface avant la phase 3.");

    Ok(())
}
