//! Point d'entree du daemon.
//!
//! `anyhow` est autorise ici : c'est un binaire, personne ne fait de pattern
//! matching sur ses erreurs. Les bibliotheques, elles, utilisent `thiserror`.

use anyhow::Result;
use tracing::info;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "trame=info".into()))
        .with_writer(std::io::stderr)
        .init();

    info!(
        version = env!("CARGO_PKG_VERSION"),
        "trame-daemon : phase 0. Ni Supervisor, ni registre, ni session : \
         la phase 0 ne pose que les frontieres."
    );

    Ok(())
}
