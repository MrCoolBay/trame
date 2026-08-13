//! Les couleurs et les symboles. **Un seul endroit, et il est petit.**
//!
//! Pas de bibliothèque de composants : `gpui-component` target une autre version de `gpui`
//! que la nôtre (ADR 0023), et le périmètre v0.1 n'en a pas besoin. Ce qu'il faut tient dans un file —
//! des couleurs nommées et deux fonctions.
//!
//! **La règle qui gouverne ce file** : la distinction visuelle ne repose jamais sur la seule
//! color. Chaque nature de line a une color **et** un marker, parce qu'une color
//! disparaît en niveaux de gris, dans une capture d'écran, et pour une partie des utilisateurs.

use gpui::{Hsla, Rgba, rgb};
use trame_view::Kind;

/// Le fond de la fenêtre.
pub const BG: u32 = 0x0f1419;
/// Le fond des bandeaux et des panels.
pub const BG_PANEL: u32 = 0x1a212b;
/// La bordure au repos.
pub const BORDER: u32 = 0x2c3542;
/// Le texte principal.
pub const TEXT: u32 = 0xe6edf3;
/// Le texte secondaire — ce qui est propre reste discret.
pub const TEXT_DIM: u32 = 0x6e7d8f;
/// ★ Ce qui compte : une lecture périmée.
pub const STALE: u32 = 0xf2cc60;
/// Un refus.
pub const REFUS: u32 = 0xf85149;
/// Un avis injecté.
pub const NOTICE: u32 = 0x39c5cf;
/// Le hors-bande : une color à soi, parce que ce n'est pas une admission.
pub const OBSERVED: u32 = 0xd2a8ff;
/// Une session qui écrit.
pub const WRITING: u32 = 0xf2cc60;
/// Une session qui réfléchit.
pub const THINKING: u32 = 0x39c5cf;
/// Le mode shadow : une **mesure**, pas un avis. Terne et froid, pour qu'on ne le confonde
/// jamais avec un `StaleRead` (ADR 0027).
pub const OMBRE: u32 = 0x6a9fea;

/// La color d'une nature de line.
///
/// Publique et sans état : les tests s'appuient dessus pour vérifier que deux natures
/// **différentes** ne se peignent pas pareil.
#[must_use]
pub fn color(kind: Kind) -> Rgba {
    rgb(match kind {
        // Le propre est terne, à dessein : ~95 % du trafic doit passer sans un mot.
        Kind::Read => TEXT_DIM,
        Kind::Clean => 0x8b98a5,
        Kind::Stale => STALE,
        Kind::Refused => REFUS,
        Kind::Notice => NOTICE,
        Kind::Observed => OBSERVED,
        Kind::Lost => REFUS,
        Kind::Shadow => OMBRE,
    })
}

/// Le marker textuel d'une nature de line.
///
/// **Le second axe de la distinction.** `▲` sur ce qui mérite l'attention, `~` sur ce qui a été
/// constaté après coup et que personne n'a admis, deux espaces sur le reste.
#[must_use]
pub const fn marker(kind: Kind) -> &'static str {
    match kind {
        Kind::Observed => "~ ",
        _ if kind.is_notable() => "▲ ",
        _ => "  ",
    }
}

/// Une color avec son opacité, pour les fonds discrets.
#[must_use]
pub fn tint(color: u32, opacite: f32) -> Hsla {
    Hsla::from(rgb(color)).opacity(opacite)
}
