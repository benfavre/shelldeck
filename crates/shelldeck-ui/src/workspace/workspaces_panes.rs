//! Native presentation for the typed tabs retained by `WorkspaceSurfaceState`.
//!
//! The core DTO remains the source of pane/split/focus identity. This module
//! only binds those identities to GPUI surfaces and reports activations that
//! require an application-owned runtime (currently providers and browsers).

use super::*;
use shelldeck_core::workspace_navigation::{PaneNode, SplitAxis};

pub(super) fn resolve_local_tab_path(
    checkout: &ProjectCheckout,
    relative: &shelldeck_core::config::workspace_catalog::WorkspaceRelativePath,
) -> Option<shelldeck_core::config::workspace_catalog::AuthorizedLocalPath> {
    checkout.resolve_existing_local_path(relative).ok()
}

pub(super) fn validated_browser_location(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 4_096 || raw.chars().any(char::is_control) {
        return None;
    }
    let parsed = url::Url::parse(raw).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.host_str().is_none_or(str::is_empty)
    {
        return None;
    }
    Some(raw.to_owned())
}

#[derive(Clone, Debug)]
pub(super) enum RetainedWorkspaceEvent {
    Activate(WorkspacePaneActivation),
    LayoutChanged {
        workspace: CatalogWorkspaceId,
        surface: WorkspaceSurfaceState,
    },
}

impl EventEmitter<RetainedWorkspaceEvent> for RetainedWorkspaceSurface {}

pub(super) fn set_active_tab(node: &mut PaneNode, focus: WorkspaceFocus) -> Option<WorkspaceTab> {
    match node {
        PaneNode::Leaf(leaf) if leaf.id == focus.pane_id => {
            let tab = leaf.tabs.iter().find(|tab| tab.id == focus.tab_id)?.clone();
            leaf.active_tab = Some(focus.tab_id);
            Some(tab)
        }
        PaneNode::Leaf(_) => None,
        PaneNode::Split { first, second, .. } => {
            set_active_tab(first, focus).or_else(|| set_active_tab(second, focus))
        }
    }
}

/// Remove every pane reference to one local agent runtime. Session UUIDs are
/// global, but malformed/restored state may contain duplicates, so this never
/// stops after the first match. Empty leaves remain valid split placeholders.
pub(super) fn remove_agent_session_tabs(
    surface: &mut WorkspaceSurfaceState,
    session_id: Uuid,
) -> bool {
    fn remove(node: &mut PaneNode, session_id: Uuid) -> bool {
        match node {
            PaneNode::Leaf(leaf) => {
                let before = leaf.tabs.len();
                leaf.tabs.retain(|tab| {
                    !matches!(&tab.content, WorkspaceTabContent::AgentSession(binding)
                        if binding.session_id == session_id)
                });
                if leaf
                    .active_tab
                    .is_some_and(|active| !leaf.tabs.iter().any(|tab| tab.id == active))
                {
                    leaf.active_tab = leaf.tabs.first().map(|tab| tab.id);
                }
                before != leaf.tabs.len()
            }
            PaneNode::Split { first, second, .. } => {
                let first_changed = remove(first, session_id);
                remove(second, session_id) || first_changed
            }
        }
    }

    fn contains(node: &PaneNode, focus: WorkspaceFocus) -> bool {
        match node {
            PaneNode::Leaf(leaf) => {
                leaf.id == focus.pane_id && leaf.tabs.iter().any(|tab| tab.id == focus.tab_id)
            }
            PaneNode::Split { first, second, .. } => {
                contains(first, focus) || contains(second, focus)
            }
        }
    }

    fn first_focus(node: &PaneNode) -> Option<WorkspaceFocus> {
        match node {
            PaneNode::Leaf(leaf) => leaf.active_tab.map(|tab_id| WorkspaceFocus {
                pane_id: leaf.id,
                tab_id,
            }),
            PaneNode::Split { first, second, .. } => {
                first_focus(first).or_else(|| first_focus(second))
            }
        }
    }

    let Some(root) = surface.root.as_mut() else {
        return false;
    };
    let changed = remove(root, session_id);
    if changed && surface.focus.is_some_and(|focus| !contains(root, focus)) {
        surface.focus = first_focus(root);
    }
    changed
}

pub(super) fn split_leaf_with_active_tab(
    node: &mut PaneNode,
    pane_id: PaneId,
    axis: SplitAxis,
) -> Option<WorkspaceFocus> {
    match node {
        PaneNode::Leaf(leaf) if leaf.id == pane_id && leaf.tabs.len() > 1 => {
            let active = leaf.active_tab?;
            let active_index = leaf.tabs.iter().position(|tab| tab.id == active)?;
            let mut first = leaf.clone();
            let moved = first.tabs.remove(active_index);
            first.active_tab = first.tabs.first().map(|tab| tab.id);
            let second_id = PaneId::from_uuid(Uuid::new_v4());
            let focus = WorkspaceFocus {
                pane_id: second_id,
                tab_id: moved.id,
            };
            *node = PaneNode::Split {
                axis,
                ratio_basis_points: 5_000,
                first: Box::new(PaneNode::Leaf(first)),
                second: Box::new(PaneNode::Leaf(PaneLeaf {
                    id: second_id,
                    tabs: vec![moved],
                    active_tab: Some(focus.tab_id),
                })),
            };
            Some(focus)
        }
        PaneNode::Leaf(_) => None,
        PaneNode::Split { first, second, .. } => split_leaf_with_active_tab(first, pane_id, axis)
            .or_else(|| split_leaf_with_active_tab(second, pane_id, axis)),
    }
}

fn first_pane_id(node: &PaneNode) -> PaneId {
    match node {
        PaneNode::Leaf(leaf) => leaf.id,
        PaneNode::Split { first, .. } => first_pane_id(first),
    }
}

pub(super) fn adjust_split_ratio(
    node: &mut PaneNode,
    first_anchor: PaneId,
    second_anchor: PaneId,
    delta_basis_points: i16,
) -> bool {
    match node {
        PaneNode::Leaf(_) => false,
        PaneNode::Split {
            ratio_basis_points,
            first,
            second,
            ..
        } if first_pane_id(first) == first_anchor && first_pane_id(second) == second_anchor => {
            *ratio_basis_points =
                (*ratio_basis_points as i32 + delta_basis_points as i32).clamp(1_000, 9_000) as u16;
            true
        }
        PaneNode::Split { first, second, .. } => {
            adjust_split_ratio(first, first_anchor, second_anchor, delta_basis_points)
                || adjust_split_ratio(second, first_anchor, second_anchor, delta_basis_points)
        }
    }
}

impl RetainedWorkspaceSurface {
    pub(super) fn set_surface(&mut self, surface: WorkspaceSurfaceState) {
        self.surface = surface;
    }

    pub(super) fn activate_tab(&mut self, focus: WorkspaceFocus, cx: &mut Context<Self>) {
        let Some(root) = self.surface.root.as_mut() else {
            return;
        };
        let Some(tab) = set_active_tab(root, focus) else {
            return;
        };
        self.surface.focus = Some(focus);
        self.resolved_local_tab = None;
        match &tab.content {
            WorkspaceTabContent::Terminal(_) => {
                self.terminal
                    .update(cx, |terminal, _| terminal.select_tab(tab.id.as_uuid()));
            }
            WorkspaceTabContent::Editor { relative_path, .. } => {
                if let Some(path) = resolve_local_tab_path(&self.checkout, relative_path) {
                    self.resolved_local_tab = Some(tab.id);
                    self.editor.update(cx, |editor, cx| {
                        editor.file_browser_visible = false;
                        editor.open_file(path.as_path().to_path_buf(), cx);
                    });
                }
            }
            WorkspaceTabContent::Files { relative_root, .. } => {
                if let Some(path) = resolve_local_tab_path(&self.checkout, relative_root) {
                    self.resolved_local_tab = Some(tab.id);
                    self.editor.update(cx, |editor, _| {
                        editor.file_browser.set_root(path.as_path().to_path_buf());
                        editor.file_browser_visible = true;
                    });
                }
            }
            WorkspaceTabContent::Browser { .. }
            | WorkspaceTabContent::AgentSession(_)
            | WorkspaceTabContent::ProviderSession(_) => {}
        }
        cx.emit(RetainedWorkspaceEvent::Activate(WorkspacePaneActivation {
            workspace: self.workspace,
            focus,
            title: tab.title,
            content: tab.content,
        }));
        cx.notify();
    }

    pub(super) fn prepare_files_tab(
        &mut self,
        tab_id: WorkspaceTabId,
        relative_root: &shelldeck_core::config::workspace_catalog::WorkspaceRelativePath,
        cx: &mut Context<Self>,
    ) -> bool {
        let Ok(path) = self.checkout.resolve_existing_local_path(relative_root) else {
            return false;
        };
        self.resolved_local_tab = Some(tab_id);
        self.editor.update(cx, |editor, _| {
            editor.file_browser.set_root(path.as_path().to_path_buf());
            editor.file_browser_visible = true;
        });
        cx.notify();
        true
    }

    fn split_pane(&mut self, pane_id: PaneId, axis: SplitAxis, cx: &mut Context<Self>) {
        let Some(root) = self.surface.root.as_mut() else {
            return;
        };
        let Some(focus) = split_leaf_with_active_tab(root, pane_id, axis) else {
            return;
        };
        self.surface.focus = Some(focus);
        cx.emit(RetainedWorkspaceEvent::LayoutChanged {
            workspace: self.workspace,
            surface: self.surface.clone(),
        });
        cx.notify();
    }

    fn resize_split(
        &mut self,
        first_anchor: PaneId,
        second_anchor: PaneId,
        delta_basis_points: i16,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.surface.root.as_mut() else {
            return;
        };
        if !adjust_split_ratio(root, first_anchor, second_anchor, delta_basis_points) {
            return;
        }
        cx.emit(RetainedWorkspaceEvent::LayoutChanged {
            workspace: self.workspace,
            surface: self.surface.clone(),
        });
        cx.notify();
    }

    fn render_tab_strip(&self, leaf: &PaneLeaf, cx: &mut Context<Self>) -> AnyElement {
        let entity = cx.entity();
        let mut strip = div()
            .flex()
            .items_center()
            .h(px(34.0))
            .flex_shrink_0()
            .overflow_x_hidden()
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_surface());
        for tab in &leaf.tabs {
            let focus = WorkspaceFocus {
                pane_id: leaf.id,
                tab_id: tab.id,
            };
            let active = leaf.active_tab == Some(tab.id);
            let icon = match tab.content {
                WorkspaceTabContent::Terminal(_) => "terminal",
                WorkspaceTabContent::Editor { .. } => "file-code-2",
                WorkspaceTabContent::Files { .. } => "files",
                WorkspaceTabContent::Browser { .. } => "globe-2",
                WorkspaceTabContent::AgentSession(_) => "bot",
                WorkspaceTabContent::ProviderSession(_) => "bot",
            };
            let tab_entity = entity.clone();
            strip = strip.child(
                div()
                    .id(("workspace-pane-tab", tab.id.as_uuid().as_u128() as u64))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .h_full()
                    .px(px(10.0))
                    .border_r_1()
                    .border_color(ShellDeckColors::border())
                    .when(active, |view| view.bg(ShellDeckColors::bg_primary()))
                    .text_size(px(11.0))
                    .text_color(if active {
                        ShellDeckColors::text_primary()
                    } else {
                        ShellDeckColors::text_muted()
                    })
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        tab_entity.update(cx, |surface, cx| surface.activate_tab(focus, cx));
                    })
                    .child(lucide_icon(icon, 13.0, ShellDeckColors::text_muted()))
                    .child(tab.title.clone()),
            );
        }
        if leaf.tabs.len() > 1 {
            let split_entity = entity;
            let pane_id = leaf.id;
            strip = strip.child(
                div()
                    .id(("workspace-pane-split", leaf.id.as_uuid().as_u128() as u64))
                    .ml_auto()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(32.0))
                    .h_full()
                    .border_l_1()
                    .border_color(ShellDeckColors::border())
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        split_entity.update(cx, |surface, cx| {
                            surface.split_pane(pane_id, SplitAxis::Horizontal, cx)
                        });
                    })
                    .child(lucide_icon(
                        "columns-2",
                        13.0,
                        ShellDeckColors::text_muted(),
                    )),
            );
        }
        strip.into_any_element()
    }

    fn render_unattached_runtime(
        &self,
        icon: &'static str,
        title: String,
        detail: String,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(8.0))
            .size_full()
            .text_size(px(11.0))
            .text_color(ShellDeckColors::text_muted())
            .child(lucide_icon(icon, 24.0, ShellDeckColors::text_muted()))
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ShellDeckColors::text_primary())
                    .child(title),
            )
            .child(detail)
            .into_any_element()
    }

    fn render_browser_card(
        &self,
        tab_id: WorkspaceTabId,
        title: String,
        location: String,
    ) -> AnyElement {
        let validated = validated_browser_location(&location);
        let mut open = Button::new(
            ("workspace-browser-open", tab_id.as_uuid().as_u128() as u64),
            t!("workspaces.card.open").to_string(),
        )
        .size(ButtonSize::Sm)
        .variant(ButtonVariant::Default)
        .disabled(validated.is_none());
        if let Some(url) = validated {
            open = open.on_click(move |_, _, cx| cx.open_url(&url));
        }
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.0))
            .size_full()
            .px(px(20.0))
            .text_size(px(11.0))
            .text_color(ShellDeckColors::text_muted())
            .child(lucide_icon("globe-2", 24.0, ShellDeckColors::text_muted()))
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ShellDeckColors::text_primary())
                    .child(title),
            )
            .child(div().max_w(px(520.0)).child(location))
            .child(open)
            .into_any_element()
    }

    fn render_leaf(&self, leaf: &PaneLeaf, cx: &mut Context<Self>) -> AnyElement {
        let active = leaf
            .active_tab
            .and_then(|id| leaf.tabs.iter().find(|tab| tab.id == id));
        let body = match active.map(|tab| &tab.content) {
            Some(WorkspaceTabContent::Terminal(_)) => self.terminal.clone().into_any_element(),
            Some(WorkspaceTabContent::Editor { .. } | WorkspaceTabContent::Files { .. })
                if active.is_some_and(|tab| self.resolved_local_tab == Some(tab.id)) =>
            {
                self.editor.clone().into_any_element()
            }
            Some(WorkspaceTabContent::ProviderSession(binding)) => self.render_unattached_runtime(
                "bot",
                active.map(|tab| tab.title.clone()).unwrap_or_default(),
                binding.session_id.clone(),
            ),
            Some(WorkspaceTabContent::AgentSession(binding))
                if self.agent_host.as_ref().is_some_and(|host| {
                    host.read(cx).selected_session_id() == Some(binding.session_id)
                }) =>
            {
                self.agent_host.clone().unwrap().into_any_element()
            }
            Some(WorkspaceTabContent::AgentSession(binding)) => self.render_unattached_runtime(
                "bot",
                active.map(|tab| tab.title.clone()).unwrap_or_default(),
                binding.session_id.to_string(),
            ),
            Some(WorkspaceTabContent::Browser { location }) => self.render_browser_card(
                active.expect("browser content has an active tab").id,
                active.map(|tab| tab.title.clone()).unwrap_or_default(),
                location.clone(),
            ),
            Some(WorkspaceTabContent::Editor { relative_path, .. }) => self
                .render_unattached_runtime(
                    "file-code-2",
                    relative_path.as_str().to_owned(),
                    String::new(),
                ),
            Some(WorkspaceTabContent::Files { relative_root, .. }) => self
                .render_unattached_runtime(
                    "files",
                    relative_root.as_str().to_owned(),
                    String::new(),
                ),
            None => self.render_unattached_runtime("panel-top", String::new(), String::new()),
        };
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .size_full()
            .child(self.render_tab_strip(leaf, cx))
            .child(div().flex().flex_1().min_h_0().min_w_0().child(body))
            .into_any_element()
    }

    fn render_node(&self, node: &PaneNode, cx: &mut Context<Self>) -> AnyElement {
        match node {
            PaneNode::Leaf(leaf) => self.render_leaf(leaf, cx),
            PaneNode::Split {
                axis,
                ratio_basis_points,
                first,
                second,
            } => {
                let first_ratio = *ratio_basis_points as f32 / 10_000.0;
                let second_ratio = 1.0 - first_ratio;
                let horizontal = *axis == SplitAxis::Horizontal;
                let first_anchor = first_pane_id(first);
                let second_anchor = first_pane_id(second);
                let shrink_entity = cx.entity();
                let grow_entity = shrink_entity.clone();
                let divider = div()
                    .flex()
                    .when(horizontal, |view| view.flex_col().w(px(24.0)).h_full())
                    .when(!horizontal, |view| view.flex_row().h(px(24.0)).w_full())
                    .items_center()
                    .justify_center()
                    .flex_shrink_0()
                    .bg(ShellDeckColors::bg_surface())
                    .child(
                        Button::new(
                            (
                                "workspace-split-shrink",
                                first_anchor.as_uuid().as_u128() as u64,
                            ),
                            "−",
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .on_click(move |_, _, cx| {
                            shrink_entity.update(cx, |surface, cx| {
                                surface.resize_split(first_anchor, second_anchor, -500, cx)
                            });
                        }),
                    )
                    .child(
                        Button::new(
                            (
                                "workspace-split-grow",
                                second_anchor.as_uuid().as_u128() as u64,
                            ),
                            "+",
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .on_click(move |_, _, cx| {
                            grow_entity.update(cx, |surface, cx| {
                                surface.resize_split(first_anchor, second_anchor, 500, cx)
                            });
                        }),
                    );
                div()
                    .flex()
                    .when(horizontal, |view| view.flex_row())
                    .when(!horizontal, |view| view.flex_col())
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .child(
                        div()
                            .flex()
                            .min_w_0()
                            .min_h_0()
                            .flex_basis(relative(first_ratio))
                            .when(horizontal, |view| {
                                view.border_r_1().border_color(ShellDeckColors::border())
                            })
                            .when(!horizontal, |view| {
                                view.border_b_1().border_color(ShellDeckColors::border())
                            })
                            .child(self.render_node(first, cx)),
                    )
                    .child(divider)
                    .child(
                        div()
                            .flex()
                            .min_w_0()
                            .min_h_0()
                            .flex_basis(relative(second_ratio))
                            .child(self.render_node(second, cx)),
                    )
                    .into_any_element()
            }
        }
    }
}

impl Render for RetainedWorkspaceSurface {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = self
            .surface
            .root
            .clone()
            .map(|root| self.render_node(&root, cx))
            .unwrap_or_else(|| {
                self.render_unattached_runtime("panel-top", String::new(), String::new())
            });
        div()
            .id((
                "retained-workspace",
                self.workspace.as_uuid().as_u128() as u64,
            ))
            .flex()
            .flex_col()
            .size_full()
            .child(body)
    }
}
