//! Les couleurs et les symboles. **Un seul endroit, et il est petit.**
//!
//! Pas de bibliothèque de composants : `gpui-component` est incompatible avec l'épinglage
//! (ADR 0023), et le périmètre v0.1 n'en a pas besoin. Ce qu'il faut tient dans un fichier —
//! des couleurs nommées et deux fonctions.
//!
//! **La règle qui gouverne ce fichier** : la distinction visuelle ne repose jamais sur la seule
//! couleur. Chaque nature de ligne a une couleur **et** un marqueur, parce qu'une couleur
//! disparaît en niveaux de gris, dans une capture d'écran, et pour une partie des utilisateurs.

use gpui::{Hsla, Rgba, rgb};
use trame_view::Kind;

/// Le fond de la fenêtre.
pub const FOND: u32 = 0x0f1419;
/// Le fond des bandeaux et des panneaux.
pub const FOND_PANNEAU: u32 = 0x1a212b;
/// La bordure au repos.
pub const BORDURE: u32 = 0x2c3542;
/// Le texte principal.
pub const TEXTE: u32 = 0xe6edf3;
/// Le texte secondaire — ce qui est propre reste discret.
pub const TEXTE_TERNE: u32 = 0x6e7d8f;
/// ★ Ce qui compte : une lecture périmée.
pub const PERIME: u32 = 0xf2cc60;
/// Un refus.
pub const REFUS: u32 = 0xf85149;
/// Un avis injecté.
pub const AVIS: u32 = 0x39c5cf;
/// Le hors-bande : une couleur à soi, parce que ce n'est pas une admission.
pub const OBSERVE: u32 = 0xd2a8ff;
/// Une session qui écrit.
pub const ECRIT: u32 = 0xf2cc60;
/// Une session qui réfléchit.
pub const PENSE: u32 = 0x39c5cf;

/// La couleur d'une nature de ligne.
///
/// Publique et sans état : les tests s'appuient dessus pour vérifier que deux natures
/// **différentes** ne se peignent pas pareil.
#[must_use]
pub fn couleur(kind: Kind) -> Rgba {
    rgb(match kind {
        // Le propre est terne, à dessein : ~95 % du trafic doit passer sans un mot.
        Kind::Read => TEXTE_TERNE,
        Kind::Clean => 0x8b98a5,
        Kind::Stale => PERIME,
        Kind::Refused => REFUS,
        Kind::Notice => AVIS,
        Kind::Observed => OBSERVE,
        Kind::Lost => REFUS,
    })
}

/// Le marqueur textuel d'une nature de ligne.
///
/// **Le second axe de la distinction.** `▲` sur ce qui mérite l'attention, `~` sur ce qui a été
/// constaté après coup et que personne n'a admis, deux espaces sur le reste.
#[must_use]
pub const fn marqueur(kind: Kind) -> &'static str {
    match kind {
        Kind::Observed => "~ ",
        _ if kind.is_notable() => "▲ ",
        _ => "  ",
    }
}

/// Une couleur avec son opacité, pour les fonds discrets.
#[must_use]
pub fn voile(couleur: u32, opacite: f32) -> Hsla {
    Hsla::from(rgb(couleur)).opacity(opacite)
}
