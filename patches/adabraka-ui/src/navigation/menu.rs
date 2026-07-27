//! Menu system for dropdown and context menus.

use crate::{
    components::{
        icon::Icon,
        icon_source::IconSource,
        text::{body, caption},
    },
    theme::use_theme,
};
use gpui::{prelude::FluentBuilder as _, InteractiveElement, *};
use std::rc::Rc;

#[derive(Clone, Debug)]
pub enum MenuItemKind {
    Action,
    Checkbox { checked: bool },
    Radio { checked: bool },
    Submenu,
    Separator,
}

#[derive(Clone)]
pub struct MenuItem {
    pub id: SharedString,
    pub label: SharedString,
    pub icon: Option<IconSource>,
    pub shortcut: Option<SharedString>,
    pub kind: MenuItemKind,
    pub disabled: bool,
    pub on_click: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    pub children: Vec<MenuItem>,
}

impl MenuItem {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            shortcut: None,
            kind: MenuItemKind::Action,
            disabled: false,
            on_click: None,
            children: Vec::new(),
        }
    }

    pub fn separator() -> Self {
        Self {
            id: SharedString::from("separator"),
            label: SharedString::from(""),
            icon: None,
            shortcut: None,
            kind: MenuItemKind::Separator,
            disabled: false,
            on_click: None,
            children: Vec::new(),
        }
    }

    pub fn checkbox(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        checked: bool,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            shortcut: None,
            kind: MenuItemKind::Checkbox { checked },
            disabled: false,
            on_click: None,
            children: Vec::new(),
        }
    }

    pub fn submenu(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            shortcut: None,
            kind: MenuItemKind::Submenu,
            disabled: false,
            on_click: None,
            children: Vec::new(),
        }
    }

    pub fn with_icon(mut self, icon: impl Into<IconSource>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn with_shortcut(mut self, shortcut: impl Into<SharedString>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_click<F>(mut self, handler: F) -> Self
    where
        F: Fn(&mut Window, &mut App) + 'static,
    {
        self.on_click = Some(Rc::new(handler));
        self
    }

    pub fn with_children(mut self, children: Vec<MenuItem>) -> Self {
        self.children = children;
        self
    }
}

#[derive(IntoElement)]
pub struct Menu {
    items: Vec<MenuItem>,
    min_width: Pixels,
    max_height: Option<Pixels>,
    style: StyleRefinement,
}

impl Menu {
    pub fn new(items: Vec<MenuItem>) -> Self {
        Self {
            items,
            min_width: px(200.0),
            max_height: Some(px(400.0)),
            style: StyleRefinement::default(),
        }
    }

    pub fn min_width(mut self, width: Pixels) -> Self {
        self.min_width = width;
        self
    }

    pub fn max_height(mut self, height: Option<Pixels>) -> Self {
        self.max_height = height;
        self
    }
}

impl Styled for Menu {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Menu {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = use_theme();
        let user_style = self.style;

        div()
            .min_w(self.min_width)
            .when_some(self.max_height, |div, h| div.max_h(h))
            .flex()
            .flex_col()
            .bg(theme.tokens.popover)
            .border_1()
            .border_color(theme.tokens.border)
            .rounded(theme.tokens.radius_md)
            .shadow_lg()
            .p(px(4.0))
            .children(self.items.into_iter().map(render_menu_item))
            .map(|this| {
                let mut div = this;
                div.style().refine(&user_style);
                div
            })
    }
}

fn render_menu_item(item: MenuItem) -> impl IntoElement {
    let theme = use_theme();

    match item.kind {
        MenuItemKind::Separator => div()
            .h(px(1.0))
            .bg(theme.tokens.border)
            .my(px(4.0))
            .mx(px(8.0)),
        _ => {
            let is_checked = matches!(
                item.kind,
                MenuItemKind::Checkbox { checked: true } | MenuItemKind::Radio { checked: true }
            );
            let has_submenu = matches!(item.kind, MenuItemKind::Submenu);

            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .px(px(12.0))
                .py(px(8.0))
                .rounded(theme.tokens.radius_sm)
                .cursor(if item.disabled {
                    CursorStyle::Arrow
                } else {
                    CursorStyle::PointingHand
                })
                .when(item.disabled, |div| div.opacity(0.5))
                .when(!item.disabled, |div| {
                    div.hover(|style| style.bg(theme.tokens.accent))
                })
                .when_some(item.on_click.filter(|_| !item.disabled), |div, handler| {
                    div.on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                        handler(window, cx);
                    })
                })
                // ShellDeck patch: SDPATCH-026 — use one leading gutter for
                // either a checkmark or an icon. The upstream layout always
                // inserted an empty check gutter before a separate icon,
                // wasting 28px at the left of every ordinary menu command.
                .child(
                    div()
                        .w(px(16.0))
                        .h(px(16.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(is_checked, |div| {
                            div.child(
                                Icon::new("check")
                                    .size(px(12.0))
                                    .color(theme.tokens.foreground),
                            )
                        })
                        .when_some(item.icon.filter(|_| !is_checked), |div, icon| {
                            div.child(Icon::new(icon).size(px(16.0)).color(if item.disabled {
                                theme.tokens.muted_foreground
                            } else {
                                theme.tokens.foreground
                            }))
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .child(body(item.label).color(if item.disabled {
                            theme.tokens.muted_foreground
                        } else {
                            theme.tokens.foreground
                        })),
                )
                .when_some(item.shortcut, |div, shortcut| {
                    div.child(
                        caption(shortcut)
                            .color(theme.tokens.muted_foreground)
                            .no_wrap(),
                    )
                })
                .when(has_submenu, |div| {
                    div.child(
                        Icon::new("chevron-right")
                            .size(px(14.0))
                            .color(theme.tokens.muted_foreground),
                    )
                })
        }
    }
}

#[derive(Clone)]
pub struct MenuBarItem {
    pub id: SharedString,
    pub label: SharedString,
    pub menu_items: Vec<MenuItem>,
}

impl MenuBarItem {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            menu_items: Vec::new(),
        }
    }

    pub fn with_items(mut self, items: Vec<MenuItem>) -> Self {
        self.menu_items = items;
        self
    }
}

pub struct MenuBar {
    items: Vec<MenuBarItem>,
    active_menu: Option<usize>,
    // ShellDeck patch: SDPATCH-025 — the upstream MenuBar tracked
    // `active_menu` but never rendered a dropdown for it, so clicking a
    // title only highlighted the trigger. These fields carry what the
    // dropdown needs: the measured trigger rects to anchor against, plus
    // host-tunable chrome so a compact application menu row does not have
    // to fork the component just to stop being 40px tall.
    trigger_bounds: Vec<Option<Bounds<Pixels>>>,
    row_height: Pixels,
    menu_min_width: Pixels,
}

impl MenuBar {
    pub fn new(items: Vec<MenuBarItem>) -> Self {
        Self {
            trigger_bounds: vec![None; items.len()],
            items,
            active_menu: None,
            row_height: px(40.0),
            menu_min_width: px(200.0),
        }
    }

    // ShellDeck patch: SDPATCH-025 — hosts that build their menus from
    // live application state (current mode, feature availability) need to
    // swap the item set between renders. Resets the open menu because the
    // previously-active index may no longer exist.
    pub fn set_items(&mut self, items: Vec<MenuBarItem>) {
        // Resize in place rather than reallocating: the measured trigger
        // rects are what the dropdown anchors to, and a host that rebuilds
        // its items every frame would otherwise clear them before every
        // render and never be able to open a menu at all.
        self.trigger_bounds.resize(items.len(), None);
        self.items = items;
        self.active_menu = None;
    }

    /// Height of the trigger row. Defaults to 40px.
    pub fn row_height(mut self, height: Pixels) -> Self {
        self.row_height = height;
        self
    }

    /// Minimum width of the dropdown panel. Defaults to 200px.
    pub fn menu_min_width(mut self, width: Pixels) -> Self {
        self.menu_min_width = width;
        self
    }

    /// Whether a dropdown is currently open.
    pub fn is_open(&self) -> bool {
        self.active_menu.is_some()
    }

    /// Index of the open dropdown, if any. Paired with [`Self::set_open_index`]
    /// so a host that rebuilds its items every frame can carry the open menu
    /// across the rebuild instead of having it snap shut.
    pub fn open_index(&self) -> Option<usize> {
        self.active_menu
    }

    /// Reopen a dropdown by index. Out-of-range indices close the menu rather
    /// than panicking later during render.
    pub fn set_open_index(&mut self, index: Option<usize>) {
        self.active_menu = index.filter(|i| *i < self.items.len());
    }

    /// Close any open dropdown.
    pub fn close(&mut self) {
        self.active_menu = None;
    }
}

impl Render for MenuBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = use_theme();
        // ShellDeck patch: SDPATCH-025 — an open menu bar tracks the pointer:
        // hovering a sibling title switches to it without a second click.
        // That is standard menu-bar behaviour on every desktop platform and
        // the upstream component had no notion of it.
        let menu_open = self.active_menu.is_some();
        let row_height = self.row_height;

        let mut bar = div()
            .flex()
            .items_center()
            .h(row_height)
            .px(px(8.0))
            .gap(px(2.0))
            .bg(theme.tokens.background)
            .border_b_1()
            .border_color(theme.tokens.border)
            // ShellDeck patch: SDPATCH-025 — a click anywhere outside the bar
            // (including inside the deferred dropdown, which is why the item
            // handlers close explicitly) dismisses the open menu.
            .on_mouse_down_out(cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                if this.active_menu.is_some() {
                    this.active_menu = None;
                    cx.notify();
                }
            }))
            .children(self.items.iter().enumerate().map(|(idx, item)| {
                let is_active = self.active_menu == Some(idx);
                let label = item.label.clone();
                let entity = cx.entity();

                // ShellDeck patch: SDPATCH-026 — keep a full-row anchor and
                // hit target, but paint the rounded hover fill in a child
                // inset by 2px vertically. Padding a ~20px label by 6px had
                // made the trigger ~32px tall inside a 28px ShellDeck row, so
                // its rounded background escaped above and below the bar.
                div()
                    .id(SharedString::from(format!("menubar-trigger-{}", item.id)))
                    .relative()
                    .h(row_height)
                    .flex()
                    .items_center()
                    .cursor(CursorStyle::PointingHand)
                    // ShellDeck patch: SDPATCH-025 — record where this trigger
                    // actually painted so the dropdown can anchor to its bottom
                    // edge instead of guessing an offset.
                    .child(
                        canvas(
                            move |bounds, _window, cx| {
                                entity.update(cx, |this, _| {
                                    if let Some(slot) = this.trigger_bounds.get_mut(idx) {
                                        *slot = Some(bounds);
                                    }
                                });
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _event, _window, cx| {
                            this.active_menu = if this.active_menu == Some(idx) {
                                None
                            } else {
                                Some(idx)
                            };
                            cx.notify();
                        }),
                    )
                    .when(menu_open && !is_active, |div| {
                        div.on_mouse_move(cx.listener(move |this, _event, _window, cx| {
                            if this.active_menu.is_some() && this.active_menu != Some(idx) {
                                this.active_menu = Some(idx);
                                cx.notify();
                            }
                        }))
                    })
                    .child(
                        div()
                            .h(row_height - px(4.0))
                            .flex()
                            .items_center()
                            .px(px(12.0))
                            .rounded(theme.tokens.radius_sm)
                            .when(is_active, |div| div.bg(theme.tokens.accent))
                            .when(!is_active, |div| {
                                div.hover(|style| style.bg(theme.tokens.muted))
                            })
                            .child(body(label).color(theme.tokens.foreground)),
                    )
            }));

        // ShellDeck patch: SDPATCH-025 — render the dropdown the upstream
        // component was missing. `deferred` + `anchored` paints it above
        // sibling content and escapes any clipping ancestor, matching the
        // pattern `Select` already uses. Each item's handler is wrapped so
        // selecting a command also closes the menu.
        if let Some(active) = self.active_menu {
            if let (Some(item), Some(Some(anchor))) =
                (self.items.get(active), self.trigger_bounds.get(active))
            {
                let position = point(anchor.origin.x, anchor.origin.y + anchor.size.height);
                let entity = cx.entity();
                let items = item
                    .menu_items
                    .iter()
                    .cloned()
                    .map(|mut menu_item| {
                        if let Some(handler) = menu_item.on_click.take() {
                            let entity = entity.clone();
                            menu_item.on_click =
                                Some(Rc::new(move |window: &mut Window, cx: &mut App| {
                                    entity.update(cx, |this, cx| {
                                        this.active_menu = None;
                                        cx.notify();
                                    });
                                    handler(window, cx);
                                }));
                        }
                        menu_item
                    })
                    .collect::<Vec<_>>();

                bar = bar.child(
                    deferred(
                        anchored()
                            .position(position)
                            .snap_to_window_with_margin(Edges::all(px(8.0)))
                            .child(
                                div()
                                    .occlude()
                                    .child(Menu::new(items).min_width(self.menu_min_width)),
                            ),
                    )
                    .with_priority(1),
                );
            }
        }

        bar
    }
}

#[derive(IntoElement)]
pub struct ContextMenu {
    items: Vec<MenuItem>,
    position: Point<Pixels>,
}

impl ContextMenu {
    pub fn new(items: Vec<MenuItem>, position: Point<Pixels>) -> Self {
        Self { items, position }
    }
}

impl RenderOnce for ContextMenu {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = use_theme();

        anchored()
            .snap_to_window_with_margin(px(8.0))
            .anchor(Corner::TopLeft)
            .position(self.position)
            .child(
                div()
                    .min_w(px(200.0))
                    .max_h(px(400.0))
                    .flex()
                    .flex_col()
                    .bg(theme.tokens.popover)
                    .border_1()
                    .border_color(theme.tokens.border)
                    .rounded(theme.tokens.radius_md)
                    .shadow_lg()
                    .p(px(4.0))
                    .children(self.items.into_iter().map(render_menu_item)),
            )
    }
}
