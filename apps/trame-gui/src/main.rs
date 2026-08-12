//! Point d'entree de l'application desktop.
//!
//! # Usage
//!
//! ```sh
//! trame-gui [chemin-du-projet] [--scenario]
//! trame-gui --smoke                     # test de fumee : ouvre, dessine une image, sort 0
//! ```
//!
//! `--scenario` fait passer le scenario canonique par le vrai registre, sans agent. Il ECRIT
//! dans le projet vise : le chemin est obligatoire et un depot est refuse.
//!
//! # Deux threads, et c'est structurel
//!
//! gpui garde le thread principal — sur macOS l'AppKit run loop y vit, ce n'est pas
//! negociable. Le daemon vit donc dans son propre runtime tokio, sur son propre thread, et les
//! deux ne se parlent que par le canal d'observation.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use gpui::{AppContext, Application, Bounds, Timer, WindowBounds, WindowOptions, px, size};
use tracing_subscriber::EnvFilter;
use trame_core::clock::SystemClock;
use trame_gui::vue::Fenetre;
use trame_view::App;
use trame_view::source;

fn main() -> Result<()> {
    // Les logs vont sur stderr : la fenetre appartient a gpui, et un `println!` dans une app
    // graphique est un log que personne ne lit.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "trame=info".into()))
        .with_writer(std::io::stderr)
        .init();

    let (racine, scenario, fumee) = arguments()?;
    if scenario {
        source::refuser_racine_dangereuse(&racine)?;
    }

    // Le canal d'abord : le daemon tourne derriere la fenetre, et l'interface n'obtient que
    // l'extremite de reception.
    let (envoi, reception) = std::sync::mpsc::channel();
    let racine_daemon = racine.clone();
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Runtime::new() {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = envoi.send(Err(format!("runtime tokio indisponible : {error}")));
                return;
            }
        };
        runtime.block_on(async move {
            match source::open(&racine_daemon, Arc::new(SystemClock), scenario).await {
                Ok(mut source) => {
                    // Le recepteur part vers l'interface ; le reste du `Source` — le watcher,
                    // les taches du journal et du registre — reste ici et doit VIVRE. Le
                    // relacher arreterait la surveillance, et le hors-bande cesserait
                    // d'apparaitre sans que rien ne le signale.
                    let (_, observations) = tokio::sync::mpsc::channel(1);
                    let observations = std::mem::replace(&mut source.observations, observations);
                    let _ = envoi.send(Ok((source.project.clone(), observations)));
                    std::future::pending::<()>().await;
                    drop(source);
                }
                Err(error) => {
                    let _ = envoi.send(Err(format!("{error:#}")));
                }
            }
        });
    });
    let (projet, observations) = reception
        .recv()
        .context("le daemon n'a pas repondu")?
        .map_err(anyhow::Error::msg)
        .context("ouverture du projet")?;

    Application::new().run(move |cx| {
        let bounds = Bounds::centered(None, size(px(1040.), px(680.)), cx);
        let fenetre = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some(format!("Trame — {projet}").into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_, cx| {
                let etat = App::new(projet.clone(), Arc::new(SystemClock));
                cx.new(|cx| Fenetre::new(etat, observations, cx))
            },
        );
        let fenetre = match fenetre {
            Ok(fenetre) => fenetre,
            Err(error) => {
                tracing::error!(%error, "impossible d'ouvrir la fenetre");
                cx.quit();
                return;
            }
        };
        cx.activate(true);

        if fumee {
            // ★ Test de fumee. On attend qu'une IMAGE ait reellement ete produite : c'est la
            // seule preuve que les shaders ont ete compiles au lancement, le risque silencieux
            // de `runtime_shaders`. Une compilation reussie ne prouve rien ici.
            cx.spawn(async move |cx| {
                for _ in 0..200 {
                    let rendu = cx
                        .update(|cx| {
                            fenetre
                                .update(cx, |vue, _, _| vue.premier_rendu)
                                .unwrap_or(false)
                        })
                        .unwrap_or(false);
                    if rendu {
                        tracing::info!("FUMEE_OK : une image a ete produite");
                        let _ = cx.update(|cx| cx.quit());
                        return;
                    }
                    Timer::after(std::time::Duration::from_millis(50)).await;
                }
                // Dix secondes sans image : echec BRUYANT. Un test de fumee qui sort 0 sans
                // avoir rien vu ne garde rien.
                tracing::error!("FUMEE_ECHEC : aucune image en 10 s");
                std::process::exit(1);
            })
            .detach();
        }
    });
    Ok(())
}

fn arguments() -> Result<(PathBuf, bool, bool)> {
    let mut racine = None;
    let mut scenario = false;
    let mut fumee = false;
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--scenario" => scenario = true,
            "--smoke" => fumee = true,
            "-h" | "--help" => {
                tracing::info!("usage : trame-gui [chemin-du-projet] [--scenario] [--smoke]");
                std::process::exit(0);
            }
            autre => racine = Some(PathBuf::from(autre)),
        }
    }
    // Meme regle que la TUI : le repertoire courant est un defaut acceptable pour observer, il
    // ne l'est pas pour un mode qui ECRIT.
    if scenario && racine.is_none() {
        anyhow::bail!(
            "--scenario ecrit dans le projet : le chemin doit etre donne explicitement.\n\
             usage : trame-gui <chemin-du-projet> --scenario"
        );
    }
    let racine = match racine {
        Some(chemin) => chemin,
        None => std::env::current_dir().context("repertoire courant illisible")?,
    };
    Ok((racine, scenario, fumee))
}
