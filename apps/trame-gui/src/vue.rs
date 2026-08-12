//! La vue. **Les trois distinctions à ne jamais perdre**, en fenêtre cette fois.
//!
//! 1. Un `StaleRead` ne ressemble pas à un `Clean` — couleur **et** marqueur `▲`.
//! 2. Une écriture **observée** ne ressemble pas à une écriture **admise**. Le watcher constate
//!    après coup ; laisser croire l'inverse serait promettre une garantie qu'on n'a pas.
//! 3. Une session **dégradée** le dit, et dit **ce qui** manque.
//!
//! Le reste reste discret : ~95 % du trafic passe sans un mot.
//!
//! # Ce qui vient de gpui, et rien de plus
//!
//! `div`, du texte, `rgb`, un `ScrollHandle`. Aucune bibliothèque de composants — le périmètre
//! v0.1 n'en a pas besoin, et `gpui-component` cible une version de `gpui` differente de la
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
/// défilement. **Aucun `RegistryHandle`** — elle observe, elle ne pilote pas (ADR 0022).
pub struct Fenetre {
    etat: App,
    defilement: ScrollHandle,
    /// Vrai dès que la vue a été dessinée au moins une fois.
    ///
    /// Sert au test de fumée : un premier rendu prouve que les shaders ont été compilés au
    /// lancement, ce qui est le seul risque silencieux de `runtime_shaders`.
    pub premier_rendu: bool,
}

impl Fenetre {
    /// Construit la vue et branche le flux d'observations.
    ///
    /// ★ **Le point technique de la phase 4** : le `Receiver` tokio est attendu depuis
    /// l'exécuteur de gpui. Aucune passerelle, aucune sérialisation — le canal n'a besoin ni du
    /// réacteur I/O ni des timers, seulement d'un waker.
    pub fn new(etat: App, mut observations: Receiver<Observation>, cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |cette_vue, cx| {
            while let Some(observation) = observations.recv().await {
                let suite = cette_vue.update(cx, |vue: &mut Self, cx| {
                    vue.etat.apply(observation);
                    // Coller à la queue : on regarde ce qui arrive, pas ce qui est arrivé il y
                    // a une minute.
                    vue.defilement.scroll_to_bottom();
                    cx.notify();
                });
                if suite.is_err() {
                    break; // la fenêtre est fermée
                }
            }
        })
        .detach();
        Self {
            etat,
            defilement: ScrollHandle::new(),
            premier_rendu: false,
        }
    }

    /// L'état affiché, pour les tests.
    #[must_use]
    pub const fn etat(&self) -> &App {
        &self.etat
    }
}

impl Render for Fenetre {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.premier_rendu = true;
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(theme::FOND))
            .font_family("SF Mono")
            .text_size(px(12.5))
            .text_color(rgb(theme::TEXTE))
            .child(bandeau(&self.etat))
            .children(self.etat.is_degraded().then(|| degradation(&self.etat)))
            .child(panneaux(&self.etat))
            .child(flux(&self.etat, &self.defilement))
    }
}

/// Le bandeau de tête : projet, compteurs, et les trous nommés.
fn bandeau(etat: &App) -> Div {
    let mut ligne = div()
        .flex()
        .flex_none()
        .items_center()
        .gap_3()
        .px_4()
        .py_2()
        .bg(rgb(theme::FOND_PANNEAU))
        .border_b_1()
        .border_color(rgb(theme::BORDURE))
        .child(
            div()
                .px_2()
                .rounded_sm()
                .bg(rgb(theme::AVIS))
                .text_color(rgb(0x0f1419))
                .child("Trame"),
        )
        .child(div().child(etat.project.clone()))
        .child(
            div()
                .text_color(rgb(theme::TEXTE_TERNE))
                .child(format!("{} session(s)", etat.panels.len())),
        );
    // Les deux compteurs qui doivent rester lisibles : ce qui a échappé à l'admission, et ce
    // qu'on n'a pas pu afficher. Un trou nommé vaut mieux qu'un trou ignoré.
    if etat.observed_writes > 0 {
        ligne = ligne.child(
            div()
                .text_color(rgb(theme::OBSERVE))
                .child(format!("{} hors-bande", etat.observed_writes)),
        );
    }
    if etat.lost > 0 {
        ligne = ligne.child(
            div()
                .text_color(rgb(theme::REFUS))
                .child(format!("{} observations perdues", etat.lost)),
        );
    }
    ligne
}

/// La bannière de dégradation. **Elle nomme la session et ce qui n'est pas garanti.**
///
/// « Mode dégradé » ne dit rien à personne.
fn degradation(etat: &App) -> Div {
    let noms: Vec<&str> = etat
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

/// Un panneau par session, à largeur égale : aucune n'est plus importante qu'une autre.
fn panneaux(etat: &App) -> Div {
    let conteneur = div()
        .flex()
        .flex_none()
        .gap_2()
        .p_2()
        .border_b_1()
        .border_color(rgb(theme::BORDURE));
    if etat.panels.is_empty() {
        // Le dire plutôt que d'afficher un cadre muet, qui se lit comme une panne.
        return conteneur.child(
            div()
                .flex_1()
                .p_3()
                .text_color(rgb(theme::TEXTE_TERNE))
                .child("aucune session — en attente d'observations"),
        );
    }
    conteneur.children(etat.panels.iter().map(panneau))
}

fn panneau(panel: &Panel) -> Div {
    let couleur_etat = match panel.state {
        SessionState::Writing => theme::ECRIT,
        SessionState::Thinking => theme::PENSE,
        SessionState::Failed(_) => theme::REFUS,
        _ => theme::TEXTE_TERNE,
    };
    // La bordure porte la même information que le contenu : périmé, dégradé, ou rien.
    let bordure = if panel.is_degraded() {
        theme::REFUS
    } else if panel.stale > 0 {
        theme::PERIME
    } else {
        theme::BORDURE
    };
    div()
        .flex_1()
        .flex()
        .flex_col()
        .gap_1()
        .p_3()
        .rounded_md()
        .bg(rgb(theme::FOND_PANNEAU))
        .border_1()
        .border_color(rgb(bordure))
        .child(
            div()
                .text_color(rgb(theme::TEXTE))
                .child(panel.name.clone()),
        )
        .child(
            div()
                .flex()
                .gap_2()
                .text_color(rgb(couleur_etat))
                .child(panel.state_symbol())
                .child(panel.state.label().to_owned()),
        )
        .child(
            div()
                .text_color(rgb(if panel.is_degraded() {
                    theme::REFUS
                } else {
                    theme::TEXTE_TERNE
                }))
                .child(if panel.is_degraded() {
                    format!("transport {} — non intercepté", panel.transport.label())
                } else {
                    format!("transport {}", panel.transport.label())
                }),
        )
        .child(
            div()
                .text_color(rgb(theme::TEXTE_TERNE))
                .child(format!("{} lues · {} écrites", panel.reads, panel.writes)),
        )
        .child(if panel.stale > 0 {
            div().text_color(rgb(theme::PERIME)).child(format!(
                "▲ {} périmée(s) · {} avis",
                panel.stale, panel.notices
            ))
        } else {
            div()
                .text_color(rgb(theme::TEXTE_TERNE))
                .child("aucune lecture périmée")
        })
}

/// Le flux, du plus ancien au plus récent, défilant et collé à sa queue.
fn flux(etat: &App, defilement: &ScrollHandle) -> impl IntoElement {
    div()
        .id("flux")
        .flex_1()
        .flex()
        .flex_col()
        .px_3()
        .py_2()
        .overflow_y_scroll()
        .track_scroll(defilement)
        .children(etat.feed.iter().map(ligne))
}

/// Une ligne de flux.
///
/// Publique pour que les tests vérifient la mise en forme sur la vraie fonction, pas sur une
/// réimplémentation.
#[must_use]
pub fn ligne(line: &Line) -> Div {
    let couleur = theme::couleur(line.kind);
    let mut rendu = div()
        .flex()
        .gap_2()
        .when(line.kind.is_notable(), |d| {
            // Un fond très discret en plus du marqueur et de la couleur : dans un flux long,
            // c'est ce qui permet de retrouver la ligne d'un coup d'œil.
            d.bg(theme::voile(theme::PERIME, 0.08)).rounded_sm()
        })
        .child(
            div()
                .w(px(18.0))
                .text_color(couleur)
                .child(theme::marqueur(line.kind)),
        )
        .child(
            div()
                .w(px(64.0))
                .flex_none()
                .text_color(rgb(theme::TEXTE_TERNE))
                .child(line.at.format("%H:%M:%S").to_string()),
        )
        .child(
            div()
                .w(px(120.0))
                .flex_none()
                .text_color(rgb(theme::TEXTE_TERNE))
                // Le hors-bande n'a pas de session : personne ne l'a demandé.
                .child(line.session.clone().unwrap_or_else(|| "—".to_owned())),
        )
        .child(
            div()
                .w(px(64.0))
                .flex_none()
                .text_color(couleur)
                .child(line.kind.verb()),
        );
    if !line.path.is_empty() {
        rendu = rendu.child(
            div()
                .w(px(200.0))
                .flex_none()
                .text_color(couleur)
                .child(line.path.clone()),
        );
    }
    if !line.detail.is_empty() {
        rendu = rendu.child(
            div()
                .flex_1()
                .text_color(couleur)
                .child(line.detail.clone()),
        );
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
    /// vérifiable ici — que couleur et marqueur diffèrent — et le rendu réel est constaté à
    /// l'écran. Ce qui n'est pas testé est nommé dans le rapport, pas passé sous silence.
    #[test]
    fn un_stale_read_se_distingue_d_un_clean_sur_deux_axes() {
        assert_ne!(
            theme::couleur(Kind::Stale),
            theme::couleur(Kind::Clean),
            "un Clean et un StaleRead peints pareil rendent le mecanisme central invisible"
        );
        assert_eq!(theme::marqueur(Kind::Stale), "▲ ");
        assert_eq!(
            theme::marqueur(Kind::Clean),
            "  ",
            "une ligne propre doit rester discrete"
        );
    }

    /// Une écriture observée ne se peint ni ne se marque comme une admission.
    #[test]
    fn observe_ne_ressemble_pas_a_admis() {
        assert_ne!(theme::couleur(Kind::Observed), theme::couleur(Kind::Clean));
        assert_ne!(theme::couleur(Kind::Observed), theme::couleur(Kind::Stale));
        assert_eq!(
            theme::marqueur(Kind::Observed),
            "~ ",
            "le hors-bande a son propre marqueur : ce n'est pas une admission"
        );
    }

    /// Chaque nature notable a un marqueur, et aucune nature discrète n'en a.
    #[test]
    fn le_marqueur_suit_la_gravite() {
        for kind in [Kind::Stale, Kind::Refused, Kind::Lost] {
            assert_eq!(theme::marqueur(kind), "▲ ", "{kind:?} merite l'attention");
        }
        for kind in [Kind::Read, Kind::Clean, Kind::Notice] {
            assert_eq!(theme::marqueur(kind), "  ", "{kind:?} doit rester discret");
        }
    }
}
