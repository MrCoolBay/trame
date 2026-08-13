//! La vue. **Les trois distinctions à ne jamais perdre**, en fenêtre cette fois.
//!
//! 1. Un `StaleRead` ne ressemble pas à un `Clean` — color **et** marker `▲`.
//! 2. Une écriture **observée** ne ressemble pas à une écriture **admise**. Le watcher constate
//!    après coup ; laisser croire l'inverse serait promettre une garantie qu'on n'a pas.
//! 3. Une session **dégradée** le dit, et dit **ce qui** manque.
//!
//! Le reste reste discret : ~95 % du trafic passe sans un mot.
//!
//! # Ce qui vient de gpui, et rien de plus
//!
//! `div`, du texte, `rgb`, un `ScrollHandle`. Aucune bibliothèque de composants — le périmètre
//! v0.1 n'en a pas besoin, et `gpui-component` target une version de `gpui` differente de la
//! notre (ADR 0023).

use gpui::{Context, Div, ScrollHandle, Window, div, prelude::*, px, rgb};
use tokio::sync::mpsc::Receiver;
use trame_core::SessionState;
use trame_daemon::Observation;
use trame_view::{App, Line, Panel};

use crate::theme;

/// La fenêtre Trame.
///
/// Elle ne tient que deux choses : l'état d'affichage, partagé avec la TUI, et une poignée de
/// défilement. **Aucun `RegistryHandle`** — elle observe, elle ne pilot pas (ADR 0022).
pub struct Screen {
    state: App,
    scroll: ScrollHandle,
    /// Vrai dès que la vue a été dessinée au moins une fois.
    ///
    /// Sert au test de fumée : un premier rendu prouve que les shaders ont été compilés au
    /// lancement, ce qui est le seul risque silencieux de `runtime_shaders`.
    pub first_render: bool,
}

impl Screen {
    /// Construit la vue et branche le feed d'observations.
    ///
    /// ★ **Le point technique de la phase 4** : le `Receiver` tokio est attendu depuis
    /// l'exécuteur de gpui. Aucune passerelle, aucune sérialisation — le canal n'a besoin ni du
    /// réacteur I/O ni des timers, seulement d'un waker.
    pub fn new(
        state: App,
        mut observations: Receiver<Observation>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.spawn(async move |cette_vue, cx| {
            while let Some(observation) = observations.recv().await {
                let suite = cette_vue.update(cx, |vue: &mut Self, cx| {
                    vue.state.apply(observation);
                    // Coller à la queue : on regarde ce qui arrive, pas ce qui est arrivé il y
                    // a une minute.
                    vue.scroll.scroll_to_bottom();
                    cx.notify();
                });
                if suite.is_err() {
                    break; // la fenêtre est fermée
                }
            }
        })
        .detach();
        Self {
            state,
            scroll: ScrollHandle::new(),
            first_render: false,
        }
    }

    /// L'état affiché, pour les tests.
    #[must_use]
    pub const fn state(&self) -> &App {
        &self.state
    }
}

impl Render for Screen {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.first_render = true;
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(theme::BG))
            .font_family("SF Mono")
            .text_size(px(12.5))
            .text_color(rgb(theme::TEXT))
            .child(header(&self.state))
            .children(
                self.state
                    .is_degraded()
                    .then(|| degraded_banner(&self.state)),
            )
            .child(panels(&self.state))
            .child(feed(&self.state, &self.scroll))
    }
}

/// Le header de tête : projet, compteurs, et les trous nommés.
fn header(state: &App) -> Div {
    let mut line = div()
        .flex()
        .flex_none()
        .items_center()
        .gap_3()
        .px_4()
        .py_2()
        .bg(rgb(theme::BG_PANEL))
        .border_b_1()
        .border_color(rgb(theme::BORDER))
        .child(
            div()
                .px_2()
                .rounded_sm()
                .bg(rgb(theme::NOTICE))
                .text_color(rgb(0x0f1419))
                .child("Trame"),
        )
        .child(div().child(state.project.clone()))
        .child(
            div()
                .text_color(rgb(theme::TEXT_DIM))
                .child(format!("{} session(s)", state.panels.len())),
        );
    // Les deux compteurs qui doivent rester lisibles : ce qui a échappé à l'admission, et ce
    // qu'on n'a pas pu afficher. Un trou nommé vaut mieux qu'un trou ignoré.
    if state.observed_writes > 0 {
        line = line.child(
            div()
                .text_color(rgb(theme::OBSERVED))
                .child(format!("{} hors-bande", state.observed_writes)),
        );
    }
    if state.potential_notices > 0 {
        line = line.child(
            div()
                .text_color(rgb(theme::OMBRE))
                .child(format!("{} potential (shadow)", state.potential_notices)),
        );
    }
    if state.lost > 0 {
        line = line.child(
            div()
                .text_color(rgb(theme::REFUS))
                .child(format!("{} observations perdues", state.lost)),
        );
    }
    line
}

/// La bannière de dégradation. **Elle nomme la session et ce qui n'est pas garanti.**
///
/// « Mode dégradé » ne dit rien à personne.
fn degraded_banner(state: &App) -> Div {
    let noms: Vec<&str> = state
        .panels
        .iter()
        .filter(|p| p.is_degraded())
        .map(|p| p.name.as_str())
        .collect();
    div()
        .flex_none()
        .px_4()
        .py_1()
        .bg(rgb(theme::REFUS))
        .text_color(rgb(0x0f1419))
        .child(format!(
            "⚠ DÉGRADÉ — {} : les écritures ne sont PAS interceptées avant le disque",
            noms.join(", ")
        ))
}

/// Un panel par session, à largeur égale : aucune n'est plus importante qu'une autre.
fn panels(state: &App) -> Div {
    let conteneur = div()
        .flex()
        .flex_none()
        .gap_2()
        .p_2()
        .border_b_1()
        .border_color(rgb(theme::BORDER));
    if state.panels.is_empty() {
        // Le dire plutôt que d'afficher un cadre muet, qui se lit comme une panne.
        return conteneur.child(
            div()
                .flex_1()
                .p_3()
                .text_color(rgb(theme::TEXT_DIM))
                .child("aucune session — en attente d'observations"),
        );
    }
    conteneur.children(state.panels.iter().map(panel))
}

fn panel(panel: &Panel) -> Div {
    let state_color = match panel.state {
        SessionState::Writing => theme::WRITING,
        SessionState::Thinking => theme::THINKING,
        SessionState::Failed(_) => theme::REFUS,
        _ => theme::TEXT_DIM,
    };
    // La bordure porte la même information que le content : périmé, dégradé, ou rien.
    let bordure = if panel.is_degraded() {
        theme::REFUS
    } else if panel.stale > 0 {
        theme::STALE
    } else {
        theme::BORDER
    };
    div()
        .flex_1()
        .flex()
        .flex_col()
        .gap_1()
        .p_3()
        .rounded_md()
        .bg(rgb(theme::BG_PANEL))
        .border_1()
        .border_color(rgb(bordure))
        .child(div().text_color(rgb(theme::TEXT)).child(panel.name.clone()))
        .child(
            div()
                .flex()
                .gap_2()
                .text_color(rgb(state_color))
                .child(panel.state_symbol())
                .child(panel.state.label().to_owned()),
        )
        .child(
            div()
                .text_color(rgb(if panel.is_degraded() {
                    theme::REFUS
                } else {
                    theme::TEXT_DIM
                }))
                .child(if panel.is_degraded() {
                    format!("transport {} — non intercepté", panel.transport.label())
                } else {
                    format!("transport {}", panel.transport.label())
                }),
        )
        .child(
            div()
                .text_color(rgb(theme::TEXT_DIM))
                .child(format!("{} lues · {} écrites", panel.reads, panel.writes)),
        )
        .child(if panel.stale > 0 {
            div().text_color(rgb(theme::STALE)).child(format!(
                "▲ {} périmée(s) · {} avis",
                panel.stale, panel.notices
            ))
        } else {
            div()
                .text_color(rgb(theme::TEXT_DIM))
                .child("aucune lecture périmée")
        })
}

/// Le feed, du plus ancien au plus récent, défilant et collé à sa queue.
fn feed(state: &App, scroll: &ScrollHandle) -> impl IntoElement {
    div()
        .id("feed")
        .flex_1()
        .flex()
        .flex_col()
        .px_3()
        .py_2()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .children(state.feed.iter().map(line))
}

/// Une line de feed.
///
/// Publique pour que les tests vérifient la mise en forme sur la vraie fonction, pas sur une
/// réimplémentation.
#[must_use]
pub fn line(line: &Line) -> Div {
    let color = theme::color(line.kind);
    let mut rendu = div()
        .flex()
        .gap_2()
        .when(line.kind.is_notable(), |d| {
            // Un fond très discret en plus du marker et de la color : dans un feed long,
            // c'est ce qui permet de retrouver la line d'un coup d'œil.
            d.bg(theme::tint(theme::STALE, 0.08)).rounded_sm()
        })
        .child(
            div()
                .w(px(18.0))
                .text_color(color)
                .child(theme::marker(line.kind)),
        )
        .child(
            div()
                .w(px(64.0))
                .flex_none()
                .text_color(rgb(theme::TEXT_DIM))
                .child(line.at.format("%H:%M:%S").to_string()),
        )
        .child(
            div()
                .w(px(120.0))
                .flex_none()
                .text_color(rgb(theme::TEXT_DIM))
                // Le hors-bande n'a pas de session : personne ne l'a demandé.
                .child(line.session.clone().unwrap_or_else(|| "—".to_owned())),
        )
        .child(
            div()
                .w(px(64.0))
                .flex_none()
                .text_color(color)
                .child(line.kind.verb()),
        );
    if !line.path.is_empty() {
        rendu = rendu.child(
            div()
                .w(px(200.0))
                .flex_none()
                .text_color(color)
                .child(line.path.clone()),
        );
    }
    if !line.detail.is_empty() {
        rendu = rendu.child(div().flex_1().text_color(color).child(line.detail.clone()));
    }
    rendu
}

#[cfg(test)]
mod tests {
    use trame_view::Kind;

    use super::*;

    /// ★ La propriété centrale, vérifiée sur les deux axes **sans ouvrir de fenêtre**.
    ///
    /// gpui ne permet pas de relire un buffer comme `TestBackend` : on vérifie donc ce qui est
    /// vérifiable ici — que color et marker diffèrent — et le rendu réel est constaté à
    /// l'écran. Ce qui n'est pas testé est nommé dans le rapport, pas passé sous silence.
    #[test]
    fn un_stale_read_se_distingue_d_un_clean_sur_deux_axes() {
        assert_ne!(
            theme::color(Kind::Stale),
            theme::color(Kind::Clean),
            "un Clean et un StaleRead peints pareil rendent le mecanisme central invisible"
        );
        assert_eq!(theme::marker(Kind::Stale), "▲ ");
        assert_eq!(
            theme::marker(Kind::Clean),
            "  ",
            "une line propre doit rester discrete"
        );
    }

    /// Une écriture observée ne se peint ni ne se marque comme une admission.
    #[test]
    fn observe_ne_ressemble_pas_a_admis() {
        assert_ne!(theme::color(Kind::Observed), theme::color(Kind::Clean));
        assert_ne!(theme::color(Kind::Observed), theme::color(Kind::Stale));
        assert_eq!(
            theme::marker(Kind::Observed),
            "~ ",
            "le hors-bande a son propre marker : ce n'est pas une admission"
        );
    }

    /// Chaque nature notable a un marker, et aucune nature discrète n'en a.
    #[test]
    fn le_marqueur_suit_la_gravite() {
        for kind in [Kind::Stale, Kind::Refused, Kind::Lost] {
            assert_eq!(theme::marker(kind), "▲ ", "{kind:?} merite l'attention");
        }
        for kind in [Kind::Read, Kind::Clean, Kind::Notice] {
            assert_eq!(theme::marker(kind), "  ", "{kind:?} doit rester discret");
        }
    }
}
