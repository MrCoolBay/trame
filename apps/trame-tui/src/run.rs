//! La boucle d'affichage. Partagee entre le binaire et les exemples.
//!
//! Elle vit ici et non dans `main.rs` pour une raison concrete : l'exemple
//! `experience_avis --tui` doit afficher exactement la meme interface que le binaire. Une
//! seconde boucle recopiee divergerait, et c'est l'affichage du produit qui divergerait —
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
/// Le flux est evenementiel ; ce tick ne sert qu'a redessiner quand rien n'arrive, et a
/// laisser la touche `q` repondre vite.
pub const TICK: Duration = Duration::from_millis(100);

/// Lit le clavier dans un thread dedie.
///
/// `event::read` est bloquant. L'appeler depuis une tache tokio bloquerait l'ordonnanceur,
/// donc le flux d'observations avec lui.
#[must_use]
pub fn spawn_touches() -> mpsc::Receiver<Event> {
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
/// **Ne s'arrete pas quand le flux d'observations se ferme.** Quand la source a fini son
/// travail — un run termine, un daemon parti — l'ecran doit rester lisible : c'est le
/// moment ou on regarde ce qui s'est passe. `tokio::select!` desactive simplement la
/// branche epuisee.
///
/// # Erreurs
///
/// Echoue si le terminal refuse un rendu.
pub async fn afficher(
    terminal: &mut DefaultTerminal,
    etat: &mut App,
    observations: &mut mpsc::Receiver<Observation>,
    touches: &mut mpsc::Receiver<Event>,
) -> std::io::Result<()> {
    let mut tick = tokio::time::interval(TICK);
    loop {
        terminal.draw(|frame| ui::render(frame, etat))?;
        if etat.quit {
            return Ok(());
        }
        tokio::select! {
            // Les observations en premier : c'est le sujet.
            Some(observation) = observations.recv() => etat.apply(observation),
            Some(evenement) = touches.recv() => applique_touche(etat, &evenement),
            _ = tick.tick() => {}
        }
    }
}

/// Traduit une touche. `q`, `Echap` et `Ctrl-C` quittent ; rien d'autre n'agit.
///
/// L'interface observe : aucune touche ne doit pouvoir modifier un projet.
pub fn applique_touche(etat: &mut App, evenement: &Event) {
    if let Event::Key(touche) = evenement
        && touche.kind == KeyEventKind::Press
    {
        let quitte = matches!(touche.code, KeyCode::Char('q') | KeyCode::Esc)
            || (touche.modifiers.contains(KeyModifiers::CONTROL)
                && touche.code == KeyCode::Char('c'));
        if quitte {
            etat.quit = true;
        }
    }
}
