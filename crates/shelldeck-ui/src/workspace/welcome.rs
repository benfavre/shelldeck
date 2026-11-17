//! The pre-login welcome screen (SDUC-597), from the owner's mock-up: one
//! promise (« Tout Inklura. Sur votre bureau. »), sign in or create an account
//! in the browser, and two cards that say what the app gathers. Every colour
//! comes from the palette, so the screen follows the theme; the « Par
//! Inklura » foot replaces the former WD29 signature.
use super::*;
use crate::overlay::round_window_bottom;

/// The product documentation, also behind the title bar's « Aide ».
pub(super) const WELCOME_DOCS_URL: &str = "https://shelldeck.1clic.pro";

/// The sticker yellow of inklura.fr; dark ink on it in every theme.
fn sticker_yellow() -> Hsla {
    hsla(43.0 / 360.0, 0.96, 0.62, 1.0)
}

fn sticker_ink() -> Hsla {
    hsla(0.0, 0.0, 0.08, 1.0)
}

/// A rounded tile outline: the stroke of the small illustrations.
fn stroke_box(w: f32, h: f32, radius: f32) -> Div {
    div()
        .w(px(w))
        .h(px(h))
        .rounded(px(radius))
        .border_2()
        .border_color(ShellDeckColors::text_primary())
}

fn illustration(id: &'static str, drawing: impl IntoElement, label: String) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex()
        .flex_col()
        .items_center()
        .gap(px(12.0))
        .min_w(px(96.0))
        .child(
            div()
                .h(px(64.0))
                .flex()
                .items_center()
                .justify_center()
                .child(drawing),
        )
        .child(
            div()
                .text_size(px(15.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(ShellDeckColors::text_primary())
                .child(label),
        )
}

fn applications_drawing() -> Div {
    let square = |fill: Option<Hsla>| {
        let tile = stroke_box(24.0, 24.0, 5.0);
        match fill {
            Some(color) => tile.bg(color),
            None => tile,
        }
    };
    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(
            div()
                .flex()
                .gap(px(6.0))
                .child(square(Some(ShellDeckColors::primary())))
                .child(square(None)),
        )
        .child(
            div()
                .flex()
                .gap(px(6.0))
                .child(square(None))
                .child(square(Some(sticker_yellow()))),
        )
}

fn projects_drawing() -> Div {
    let column = |blocks: &[(f32, Hsla)]| {
        let mut column = div().flex().flex_col().gap(px(4.0)).w(px(16.0));
        for (height, color) in blocks {
            column = column.child(div().h(px(*height)).w_full().rounded(px(2.0)).bg(*color));
        }
        column
    };
    stroke_box(76.0, 56.0, 7.0)
        .flex()
        .gap(px(5.0))
        .p(px(6.0))
        .child(column(&[(10.0, ShellDeckColors::primary())]))
        .child(
            div()
                .w(px(2.0))
                .h_full()
                .bg(ShellDeckColors::text_primary()),
        )
        .child(column(&[
            (5.0, ShellDeckColors::primary()),
            (5.0, ShellDeckColors::primary()),
        ]))
        .child(
            div()
                .w(px(2.0))
                .h_full()
                .bg(ShellDeckColors::text_primary()),
        )
        .child(column(&[(10.0, sticker_yellow())]))
}

fn terminal_drawing() -> Div {
    let dot = |color: Hsla| div().size(px(7.0)).rounded_full().bg(color);
    stroke_box(80.0, 60.0, 8.0)
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .h(px(16.0))
                .px(px(7.0))
                .border_b_2()
                .border_color(ShellDeckColors::text_primary())
                .child(dot(ShellDeckColors::primary()))
                .child(dot(sticker_yellow()))
                .child(dot(ShellDeckColors::success())),
        )
        .child(
            div()
                .flex_1()
                .flex()
                .items_center()
                .px(px(9.0))
                .child(lucide_icon(
                    "terminal",
                    26.0,
                    ShellDeckColors::text_primary(),
                )),
        )
}

fn ssh_drawing() -> Div {
    let server = |color: Hsla| {
        stroke_box(80.0, 26.0, 8.0)
            .flex()
            .items_center()
            .justify_end()
            .pr(px(10.0))
            .child(div().size(px(9.0)).rounded_full().bg(color))
    };
    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(server(ShellDeckColors::primary()))
        .child(server(sticker_yellow()))
}

fn welcome_card(
    id: &'static str,
    title: String,
    caption: String,
    row: impl IntoElement,
    tint: Option<Hsla>,
) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex_1()
        .min_w(px(300.0))
        .flex()
        .flex_col()
        .gap(px(18.0))
        .px(px(32.0))
        .pt(px(22.0))
        .pb(px(20.0))
        .rounded(px(18.0))
        .border_1()
        .border_color(ShellDeckColors::border().opacity(0.45))
        .bg(match tint {
            Some(tint) => ShellDeckColors::bg_sidebar().blend(tint.opacity(0.13)),
            None => ShellDeckColors::bg_sidebar(),
        })
        .shadow(vec![use_theme().tokens.shadow_sm].into())
        .child(
            div()
                .text_size(px(24.0))
                .font_weight(FontWeight::BOLD)
                .text_color(ShellDeckColors::text_primary())
                .child(title),
        )
        .child(row)
        .child(
            div()
                .text_center()
                .text_size(px(15.0))
                .text_color(ShellDeckColors::text_muted())
                .child(caption),
        )
}

/// Height of the « Par Inklura » foot of the welcome and loading screens.
pub(super) const FOOTER_H: f32 = 46.0;

/// The foot of the welcome and loading screens: the product, « Par Inklura »,
/// the documentation link.
pub(super) fn welcome_foot(is_maximized: bool) -> Div {
    let docs = div()
        .id("welcome-docs")
        .debug_selector(|| "welcome-docs".into())
        .flex()
        .items_center()
        .gap(px(8.0))
        .cursor_pointer()
        .hover(|style| style.text_color(ShellDeckColors::primary()))
        .on_click(|_, _, _| {
            let _ = cloud_account::open_in_browser(WELCOME_DOCS_URL);
        })
        .child(t!("welcome.docs").to_string())
        .child(lucide_icon(
            "arrow-up-right",
            15.0,
            ShellDeckColors::text_muted(),
        ));
    round_window_bottom(
        div()
            .debug_selector(|| "welcome-foot".into())
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .h(px(FOOTER_H))
            .flex()
            .items_center()
            .px(px(24.0))
            .bg(ShellDeckColors::bg_sidebar())
            .border_t_1()
            .border_color(ShellDeckColors::border().opacity(0.6))
            .text_size(px(13.5))
            .text_color(ShellDeckColors::text_muted()),
        is_maximized,
    )
    .child(div().flex_1().child(shelldeck_core::APP_NAME))
    .child(
        div()
            .debug_selector(|| "welcome-by".into())
            .flex_1()
            .text_center()
            .child(t!("welcome.by").to_string()),
    )
    .child(div().flex_1().flex().justify_end().child(docs))
}

/// The page of the welcome and loading screens: the palette gradient with the
/// circuit lines, doodles, rings and crosses above the foot.
pub(super) fn welcome_backdrop(is_maximized: bool) -> Div {
    let line_color = ShellDeckColors::primary().opacity(0.45);
    // The decoration: circuit lines and two doodles on the sides, the
    // little yellow rings and crosses of the site.
    let ring = |size: f32| {
        div()
            .absolute()
            .size(px(size))
            .rounded_full()
            .border_2()
            .border_color(sticker_yellow())
    };
    let cross = || {
        div()
            .absolute()
            .child(lucide_icon("plus", 20.0, sticker_yellow()))
    };
    let decoration = div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom(px(FOOTER_H))
        .child(
            svg()
                .absolute()
                .left(px(10.0))
                .top(px(20.0))
                .w(px(300.0))
                .h(px(640.0))
                .path("images/welcome/lines-left.svg")
                .text_color(line_color),
        )
        .child(
            svg()
                .absolute()
                .right(px(10.0))
                .top(px(20.0))
                .w(px(300.0))
                .h(px(640.0))
                .path("images/welcome/lines-right.svg")
                .text_color(line_color),
        )
        .child(
            div().absolute().left(px(150.0)).top(px(150.0)).child(
                svg()
                    .path(lucide_path("file-check"))
                    .size(px(58.0))
                    .text_color(ShellDeckColors::text_primary().opacity(0.7))
                    .with_transformation(Transformation::rotate(radians(-0.25))),
            ),
        )
        .child(
            div().absolute().right(px(170.0)).top(px(110.0)).child(
                svg()
                    .path(lucide_path("plug"))
                    .size(px(48.0))
                    .text_color(ShellDeckColors::primary())
                    .with_transformation(Transformation::rotate(radians(0.5))),
            ),
        )
        .child(ring(18.0).left(px(60.0)).top(px(440.0)))
        .child(ring(20.0).right(px(250.0)).top(px(60.0)))
        .child(cross().left(px(280.0)).top(px(150.0)))
        .child(cross().right(px(80.0)).top(px(540.0)));

    round_window_bottom(
        div()
            .relative()
            .size_full()
            .bg(linear_gradient(
                120.0,
                linear_color_stop(
                    ShellDeckColors::bg_primary().blend(ShellDeckColors::primary().opacity(0.07)),
                    0.0,
                ),
                linear_color_stop(
                    ShellDeckColors::bg_primary().blend(ShellDeckColors::success().opacity(0.09)),
                    1.0,
                ),
            ))
            .overflow_hidden(),
        is_maximized,
    )
    .child(decoration)
}

impl Workspace {
    /// `is_maximized` only rounds the two bottom corners: this is the lowest
    /// opaque layer, with no status bar under it.
    pub(super) fn render_welcome_screen(
        &self,
        is_maximized: bool,
        compact_height: bool,
        logical_height: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use crate::ludique::{chunky_button, ChunkyKind};

        let entity = cx.entity();
        let title_size = if compact_height { 40.0 } else { 56.0 };
        let title_line = |label: String| {
            div()
                .text_center()
                .text_size(px(title_size))
                .line_height(px(title_size * 1.08))
                .font_weight(FontWeight::EXTRA_BOLD)
                .text_color(ShellDeckColors::text_primary())
                .child(label)
        };
        let lead = t!("welcome.lead").to_string();
        let mut lead_lines = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(4.0))
            .text_size(px(17.0))
            .text_color(ShellDeckColors::text_muted());
        for line in lead.lines() {
            lead_lines = lead_lines.child(div().text_center().child(line.to_string()));
        }

        let hero = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(14.0))
            .px(px(24.0))
            .child(
                div()
                    .debug_selector(|| "welcome-sticker".into())
                    .px(px(16.0))
                    .py(px(6.0))
                    .rounded_full()
                    .bg(sticker_yellow())
                    .text_size(px(14.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(sticker_ink())
                    .child(t!("welcome.sticker").to_string()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .pt(px(4.0))
                    .child(title_line(t!("welcome.title_line1").to_string()))
                    .child(title_line(t!("welcome.title_line2").to_string())),
            )
            .child(lead_lines)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_center()
                    .gap(px(16.0))
                    .mt(px(14.0))
                    // Room for the hard edge the buttons stand on.
                    .pb(px(6.0))
                    .child(
                        chunky_button("welcome-sign-in", ChunkyKind::Primary)
                            .h(px(50.0))
                            .px(px(26.0))
                            .text_size(px(16.0))
                            .child(t!("welcome.sign_in").to_string())
                            .child(
                                svg()
                                    .path(lucide_path("arrow-up-right"))
                                    .size(px(17.0))
                                    .text_color(crate::theme::StreetColors::paper()),
                            )
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    entity.update(cx, |this, cx| this.show_login_form(cx));
                                }
                            }),
                    )
                    .child(
                        chunky_button("welcome-signup", ChunkyKind::Paper)
                            .h(px(50.0))
                            .px(px(24.0))
                            .text_size(px(16.0))
                            .child(t!("welcome.create_account_short").to_string())
                            .child(
                                svg()
                                    .path(lucide_path("arrow-up-right"))
                                    .size(px(17.0))
                                    .text_color(crate::theme::StreetColors::line()),
                            )
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    entity.update(cx, |this, cx| this.open_signup(cx));
                                }
                            }),
                    ),
            )
            .child(
                div()
                    .text_size(px(14.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("welcome.browser_note").to_string()),
            );

        let cards = div()
            .flex()
            .flex_wrap()
            .justify_center()
            .gap(px(24.0))
            .w_full()
            .max_w(px(1180.0))
            .px(px(24.0))
            .child(welcome_card(
                "welcome-card-spaces",
                t!("welcome.spaces_title").to_string(),
                t!("welcome.spaces_caption").to_string(),
                div()
                    .flex()
                    .justify_around()
                    .items_end()
                    .gap(px(16.0))
                    .child(illustration(
                        "welcome-spaces",
                        lucide_icon("folder", 60.0, ShellDeckColors::text_primary()),
                        t!("welcome.spaces").to_string(),
                    ))
                    .child(illustration(
                        "welcome-applications",
                        applications_drawing(),
                        t!("welcome.applications").to_string(),
                    ))
                    .child(illustration(
                        "welcome-projects",
                        projects_drawing(),
                        t!("welcome.projects").to_string(),
                    )),
                None,
            ))
            .child(welcome_card(
                "welcome-card-tools",
                t!("welcome.tools_title").to_string(),
                t!("welcome.tools_caption").to_string(),
                div()
                    .flex()
                    .justify_center()
                    .items_center()
                    .gap(px(36.0))
                    .child(illustration(
                        "welcome-terminal",
                        terminal_drawing(),
                        t!("welcome.terminal").to_string(),
                    ))
                    .child(div().pb(px(30.0)).child(lucide_icon(
                        "arrow-right",
                        30.0,
                        ShellDeckColors::success(),
                    )))
                    .child(illustration(
                        "welcome-ssh",
                        ssh_drawing(),
                        t!("welcome.ssh").to_string(),
                    )),
                Some(ShellDeckColors::success()),
            ));

        let foot = welcome_foot(is_maximized);

        let body = div()
            .id("welcome-body")
            .flex()
            .flex_col()
            .items_center()
            // Centred in the room above the foot; percentages do not resolve
            // inside the scroll area, so the height is stated. A short window
            // keeps the content at the top and scrolls.
            .justify_center()
            .w_full()
            .min_h(px((logical_height - FOOTER_H).max(0.0)))
            .gap(px(if compact_height { 24.0 } else { 34.0 }))
            .py(px(if compact_height { 20.0 } else { 34.0 }))
            .child(hero)
            .child(cards);

        let content = scrollable_vertical(body).into_any_element();

        welcome_backdrop(is_maximized).child(foot).child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom(px(FOOTER_H))
                .child(content),
        )
    }
}
