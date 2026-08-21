//! Chrome partagé des calques qui couvrent toute la fenêtre.
//!
//! Une modale, une feuille ou la palette peignent un fond qui atteint les
//! quatre coins de la fenêtre. Dans GPUI, un enfant `absolute().inset_0()`
//! n'est **pas** rogné de façon fiable par un ancêtre arrondi : la couche qui
//! peint réellement le fond doit donc porter elle-même le rayon de la fenêtre.
//! Sans cela, la fenêtre flottante redevient carrée dès qu'un calque s'ouvre.
//!
//! Voir `.agents/window-rounding.md` — règle 2 et règle 10.
//!
//! Ce helper existe parce que la même forme était recopiée dans neuf fichiers
//! et que sept d'entre eux avaient oublié le rayon. Tout nouveau calque plein
//! cadre passe par ici plutôt que de recomposer la chaîne à la main.

use crate::theme::ShellDeckColors;
use adabraka_ui::theme::use_theme;
use gpui::prelude::*;
use gpui::{div, Div, Stateful};

/// Fond plein cadre d'un calque, déjà rogné à la forme de la fenêtre.
///
/// Rend un `div` absolu qui couvre les quatre bords, occlut la souris et
/// porte le rayon de la fenêtre tant qu'elle n'est pas maximisée — une
/// fenêtre maximisée doit garder ses coins carrés, sinon le bureau
/// transparaît aux angles de l'écran.
///
/// L'appelant enchaîne ensuite ce dont il a besoin : `track_focus`,
/// `on_key_down`, `on_mouse_down` pour la fermeture au clic, la disposition
/// de son contenu.
///
/// ```ignore
/// window_backdrop("login-form-overlay", window.is_maximized())
///     .track_focus(&self.focus_handle)
///     .flex()
///     .items_center()
///     .justify_center()
///     .child(card)
/// ```
/// Arrondit les deux coins bas d'une couche **opaque** qui atteint le bord
/// inférieur de la fenêtre.
///
/// En mode Dev, c'est la barre d'état qui touche ce bord et qui porte déjà ses
/// coins. UX-004 a retiré cette barre des modes Utilisateur et Support sans
/// transférer la propriété des coins : la surface du mode est devenue la couche
/// du bas et a carré la fenêtre. Un ancêtre arrondi ne suffit pas — c'est la
/// règle 10 de `.agents/window-rounding.md`, « la couche opaque qui atteint un
/// coin le possède ».
///
/// À appliquer à la racine de toute surface plein cadre rendue sans barre
/// d'état sous elle.
pub fn round_window_bottom<E: Styled>(element: E, is_maximized: bool) -> E {
    if is_maximized {
        // Fenêtre maximisée : bord à bord, coins carrés. Un rayon ici
        // laisserait le bureau transparaître aux angles de l'écran.
        return element;
    }
    let radius = use_theme().tokens.radius_xl;
    element.rounded_bl(radius).rounded_br(radius)
}

pub fn window_backdrop(id: &'static str, is_maximized: bool) -> Stateful<Div> {
    let backdrop = div()
        .id(id)
        .occlude()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .bg(ShellDeckColors::backdrop())
        .overflow_hidden();

    if is_maximized {
        backdrop
    } else {
        backdrop.rounded(use_theme().tokens.radius_xl)
    }
}
