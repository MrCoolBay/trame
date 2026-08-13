//! La run_loop d'affichage. Partagee entre le binaire et les exemples.
//!
//! Elle vit ici et non dans `main.rs` pour une raison concrete : l'exemple
//! `notice_experiment --tui` doit afficher exactement la meme interface que le binaire. Une
//! seconde run_loop recopiee divergerait, et c'est l'affichage du produit qui divergerait —
//! celui dont on affirme qu'il ne mele pas admis et observe.

use std::time::Duration;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use tokio::sync::mpsc;
use trame_daemon::Observation;

use crate::ui;
use trame_view::App;

/// Periode de rafraichissement.
///
/// Le feed est evenementiel ; ce tick ne sert qu'a redessiner quand rien n'arrive, et a
/// laisser la touche `q` repondre vite.
pub const TICK: Duration = Duration::from_millis(100);

/// Lit le clavier dans un thread dedie.
///
/// `event::read` est bloquant. L'appeler depuis une tache tokio bloquerait l'ordonnanceur,
/// donc le feed d'observations avec lui.
#[must_use]
pub fn spawn_keys() -> mpsc::Receiver<Event> {
    let (tx, rx) = mpsc::channel(32);
    std::thread::spawn(move || {
        loop {
            match event::poll(TICK) {
                Ok(true) => match event::read() {
                    Ok(evenement) => {
                        if tx.blocking_send(evenement).is_err() {
                            break; // l'interface est fermee
                        }
                    }
                    Err(_) => break,
                },
                Ok(false) => {}
                Err(_) => break,
            }
        }
    });
    rx
}

/// Affiche jusqu'a ce que l'utilisateur quitte.
///
/// **Ne s'arrete pas quand le feed d'observations se ferme.** Quand la source a fini son
/// travail — un run termine, un daemon parti — l'ecran doit rester lisible : c'est le
/// moment ou on regarde ce qui s'est passe. `tokio::select!` desactive simplement la
/// branche epuisee.
///
/// # Erreurs
///
/// Echoue si le terminal refuse un rendu.
pub async fn render_loop(
    terminal: &mut DefaultTerminal,
    state: &mut App,
    observations: &mut mpsc::Receiver<Observation>,
    touches: &mut mpsc::Receiver<Event>,
) -> std::io::Result<()> {
    let mut tick = tokio::time::interval(TICK);
    loop {
        terminal.draw(|frame| ui::render(frame, state))?;
        if state.quit {
            return Ok(());
        }
        tokio::select! {
            // Les observations en premier : c'est le sujet.
            Some(observation) = observations.recv() => state.apply(observation),
            Some(evenement) = touches.recv() => apply_key(state, &evenement),
            _ = tick.tick() => {}
        }
    }
}

/// Traduit une touche. `q`, `Echap` et `Ctrl-C` quittent ; rien d'autre n'agit.
///
/// L'interface observe : aucune touche ne doit pouvoir modifier un projet.
pub fn apply_key(state: &mut App, evenement: &Event) {
    if let Event::Key(touche) = evenement
        && touche.kind == KeyEventKind::Press
    {
        let quitte = matches!(touche.code, KeyCode::Char('q') | KeyCode::Esc)
            || (touche.modifiers.contains(KeyModifiers::CONTROL)
                && touche.code == KeyCode::Char('c'));
        if quitte {
            state.quit = true;
        }
    }
}
