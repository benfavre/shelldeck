//! Sheet component for slide-in side panels.

use gpui::{prelude::FluentBuilder as _, *};
use std::rc::Rc;

use crate::components::button::{Button, ButtonSize, ButtonVariant};
use crate::icon_config::resolve_icon_path;
use crate::theme::use_theme;

actions!(sheet, [SheetClose]);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SheetSide {
    Left,
    #[default]
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SheetSize {
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
    Assistant,
    Custom,
}

impl SheetSize {
    fn size(&self) -> Pixels {
        match self {
            Self::Sm => px(320.0),
            Self::Md => px(400.0),
            Self::Lg => px(500.0),
            Self::Xl => px(640.0),
            Self::Assistant => px(600.0),
            Self::Custom => px(400.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SheetVariant {
    #[default]
    Default,
    Assistant,
}

pub struct Sheet {
    focus_handle: FocusHandle,
    side: SheetSide,
    size: SheetSize,
    custom_width: Option<Pixels>,
    custom_height: Option<Pixels>,
    variant: SheetVariant,
    title: Option<SharedString>,
    description: Option<SharedString>,
    content: Option<AnyElement>,
    content_factory: Option<Rc<dyn Fn() -> AnyElement>>,
    footer: Option<AnyElement>,
    show_close_button: bool,
    close_on_backdrop_click: bool,
    on_close: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    style: StyleRefinement,
}

impl Sheet {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            side: SheetSide::default(),
            size: SheetSize::default(),
            custom_width: None,
            custom_height: None,
            variant: SheetVariant::Default,
            title: None,
            description: None,
            content: None,
            content_factory: None,
            footer: None,
            show_close_button: true,
            close_on_backdrop_click: true,
            on_close: None,
            style: StyleRefinement::default(),
        }
    }

    pub fn side(mut self, side: SheetSide) -> Self {
        self.side = side;
        self
    }

    pub fn size(mut self, size: SheetSize) -> Self {
        self.size = size;
        self
    }

    pub fn width(mut self, width: impl Into<Pixels>) -> Self {
        self.custom_width = Some(width.into());
        self.size = SheetSize::Custom;
        self
    }

    pub fn height(mut self, height: impl Into<Pixels>) -> Self {
        self.custom_height = Some(height.into());
        self.size = SheetSize::Custom;
        self
    }

    pub fn variant(mut self, variant: SheetVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn content(mut self, content: impl IntoElement) -> Self {
        self.content = Some(content.into_any_element());
        self
    }

    // ShellDeck patch: SDPATCH-013 — persistent Sheet entities re-render, but
    // `AnyElement` content is consumed with `take()`. A factory lets live child
    // entities be mounted again on every Sheet render instead of going blank.
    pub fn dynamic_content<F, E>(mut self, factory: F) -> Self
    where
        F: Fn() -> E + 'static,
        E: IntoElement,
    {
        self.content_factory = Some(Rc::new(move || factory().into_any_element()));
        self
    }

    pub fn footer(mut self, footer: impl IntoElement) -> Self {
        self.footer = Some(footer.into_any_element());
        self
    }

    pub fn show_close_button(mut self, show: bool) -> Self {
        self.show_close_button = show;
        self
    }

    pub fn close_on_backdrop_click(mut self, close: bool) -> Self {
        self.close_on_backdrop_click = close;
        self
    }

    pub fn on_close<F>(mut self, handler: F) -> Self
    where
        F: Fn(&mut Window, &mut App) + 'static,
    {
        self.on_close = Some(Rc::new(handler));
        self
    }

    fn handle_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handler) = &self.on_close {
            handler(window, cx);
        }
    }

    fn handle_escape(&mut self, _: &SheetClose, window: &mut Window, cx: &mut Context<Self>) {
        self.handle_close(window, cx);
    }

    fn get_sheet_size(&self) -> Pixels {
        if let Some(width) = self.custom_width {
            return width;
        }
        if let Some(height) = self.custom_height {
            return height;
        }
        self.size.size()
    }
}

pub fn init_sheet(_cx: &mut App) {}

impl Styled for Sheet {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl Render for Sheet {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = use_theme();
        let has_header =
            self.title.is_some() || self.description.is_some() || self.show_close_button;
        let sheet_size = self.get_sheet_size();
        let user_style = self.style.clone();
        let assistant = self.variant == SheetVariant::Assistant;

        div()
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::handle_escape))
            .absolute()
            .inset_0()
            .flex()
            .bg(hsla(0.0, 0.0, 0.0, 0.5))
            .when(self.close_on_backdrop_click, |this: Div| {
                this.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        this.handle_close(window, cx);
                    }),
                )
            })
            .child(
                div()
                    .occlude()
                    .relative()
                    .flex()
                    .flex_col()
                    .bg(theme.tokens.background)
                    .border_color(if assistant {
                        theme.tokens.primary.opacity(0.55)
                    } else {
                        theme.tokens.border
                    })
                    .shadow(smallvec::smallvec![BoxShadow {
                        color: hsla(0.0, 0.0, 0.0, 0.2),
                        offset: point(px(0.0), px(0.0)),
                        blur_radius: px(16.0),
                        spread_radius: px(0.0),
                        inset: false,
                    }])
                    .on_mouse_down(MouseButton::Left, |_, _, _| {})
                    .when(self.side == SheetSide::Right, |this: Div| {
                        this.absolute()
                            .right_0()
                            .top_0()
                            .bottom_0()
                            .w(sheet_size)
                            .border_l_1()
                    })
                    .when(self.side == SheetSide::Left, |this: Div| {
                        this.absolute()
                            .left_0()
                            .top_0()
                            .bottom_0()
                            .w(sheet_size)
                            .border_r_1()
                    })
                    .when(self.side == SheetSide::Top, |this: Div| {
                        this.absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .h(sheet_size)
                            .border_b_1()
                    })
                    .when(self.side == SheetSide::Bottom, |this: Div| {
                        this.absolute()
                            .bottom_0()
                            .left_0()
                            .right_0()
                            .h(sheet_size)
                            .border_t_1()
                    })
                    .when(has_header, |this: Div| {
                        this.child(
                            div()
                                .flex()
                                .items_start()
                                .justify_between()
                                .gap(px(16.0))
                                .px(px(24.0))
                                .pt(px(24.0))
                                .pb(px(20.0))
                                .when(assistant, |header| {
                                    header.bg(theme.tokens.primary.opacity(0.06))
                                })
                                .border_b_1()
                                .border_color(if assistant {
                                    theme.tokens.primary.opacity(0.25)
                                } else {
                                    theme.tokens.border
                                })
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .flex_1()
                                        .min_w_0()
                                        .gap(px(4.0))
                                        .when_some(self.title.clone(), |this: Div, title| {
                                            this.child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap(px(8.0))
                                                    .min_w_0()
                                                    .text_size(px(18.0))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(theme.tokens.foreground)
                                                    .when(assistant, |title_row| {
                                                        title_row.child(
                                                            svg()
                                                                .path(resolve_icon_path("sparkles"))
                                                                .size(px(18.0))
                                                                .flex_shrink_0()
                                                                .text_color(theme.tokens.primary),
                                                        )
                                                    })
                                                    .child(title),
                                            )
                                        })
                                        .when_some(self.description.clone(), |this: Div, desc| {
                                            this.child(
                                                div()
                                                    .w_full()
                                                    .min_w_0()
                                                    .text_size(px(14.0))
                                                    .text_color(theme.tokens.muted_foreground)
                                                    .whitespace_normal()
                                                    .child(desc),
                                            )
                                        }),
                                )
                                .when(self.show_close_button, |this: Div| {
                                    this.child(
                                        Button::new("sheet-close-btn", "×")
                                            .variant(ButtonVariant::Ghost)
                                            .size(ButtonSize::Sm)
                                            .flex_shrink_0()
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.handle_close(window, cx);
                                            })),
                                    )
                                }),
                        )
                    })
                    .when_some(self.content.take(), |this: Div, content| {
                        this.child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_h(px(0.0))
                                .min_w_0()
                                .overflow_hidden()
                                .when(assistant, |body| {
                                    body.bg(theme.tokens.primary.opacity(0.015))
                                })
                                .child(content),
                        )
                    })
                    .when_some(self.content_factory.as_ref(), |this: Div, factory| {
                        this.child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_h(px(0.0))
                                .min_w_0()
                                .overflow_hidden()
                                .when(assistant, |body| {
                                    body.bg(theme.tokens.primary.opacity(0.015))
                                })
                                .child(factory()),
                        )
                    })
                    .map(|this| {
                        let mut div = this;
                        div.style().refine(&user_style);
                        div
                    })
                    .when_some(self.footer.take(), |this: Div, footer| {
                        this.child(
                            div()
                                .px(px(24.0))
                                .py(px(16.0))
                                .when(assistant, |footer| {
                                    footer.bg(theme.tokens.primary.opacity(0.04))
                                })
                                .border_t_1()
                                .border_color(theme.tokens.border)
                                .child(footer),
                        )
                    }),
            )
    }
}
