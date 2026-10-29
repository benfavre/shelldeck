use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AgentToolSurface {
    Sources,
    Instructions,
    Terminal,
    Library,
    Workflows,
    Delivery,
}

pub(super) struct AgentToolsUi {
    pub active: Option<AgentToolSurface>,
    pub panel_visible: bool,
    pub navigator_visible: bool,
    pub navigator_wide: bool,
    pub navigator_overlay: bool,
    pub toolbar_expanded: bool,
    pub panel_width: f32,
    pub measured_width: f32,
    pub inspector_wide: bool,
    pub instructions: Vec<(String, String)>,
    pub loading: bool,
    pub error: Option<String>,
    pub instructions_request: Uuid,
    pub inspector_context: Option<AgentExecutionContext>,
    pub terminal: Option<Entity<crate::terminal_view::TerminalView>>,
}
impl AgentToolsUi {
    pub fn new(_cx: &mut Context<AgentConsoleView>) -> Self {
        Self {
            active: None,
            panel_visible: false,
            navigator_visible: true,
            navigator_wide: true,
            navigator_overlay: false,
            toolbar_expanded: false,
            panel_width: 360.0,
            measured_width: 0.0,
            inspector_wide: false,
            instructions: Vec::new(),
            loading: false,
            error: None,
            instructions_request: Uuid::new_v4(),
            terminal: None,
            inspector_context: None,
        }
    }
}

impl AgentConsoleView {
    pub fn selected_execution_context(&self, cx: &App) -> Option<(Uuid, AgentExecutionContext)> {
        Some((self.sessions.selected_id()?, self.displayed_context(cx)))
    }

    pub fn show_agent_terminal(
        &mut self,
        terminal: Entity<crate::terminal_view::TerminalView>,
        cx: &mut Context<Self>,
    ) {
        self.tools.terminal = Some(terminal);
        self.tools.active = Some(AgentToolSurface::Terminal);
        self.tools.panel_visible = true;
        cx.notify();
    }
    /// The next executable context is also the context shown by every inspector.
    /// Active and catalog-bound sessions always retain their immutable authority.
    pub(super) fn displayed_context(&self, cx: &App) -> AgentExecutionContext {
        let session = self
            .sessions
            .selected()
            .expect("one retained agent session");
        if session.status.is_active() || self.context_locks.contains(&session.id) {
            return session.context.clone();
        }
        let target = selected_agent_target(self.selected_connection, &self.connections);
        AgentExecutionContext {
            provider: self.provider,
            target,
            access: self.access,
            workdir: self.workdir_state.read(cx).content().trim().to_string(),
            model: {
                let m = self.model_state.read(cx).content().trim().to_string();
                (!m.is_empty()).then_some(m)
            },
        }
    }

    pub(super) fn show_project_instructions(&mut self, cx: &mut Context<Self>) {
        let context = self.displayed_context(cx);
        self.tools.active = Some(AgentToolSurface::Instructions);
        self.tools.panel_visible = true;
        self.tools.instructions.clear();
        self.tools.error = None;
        self.tools.loading = false;
        let request_id = Uuid::new_v4();
        self.tools.instructions_request = request_id;
        if !matches!(context.target, AgentTarget::Local) {
            self.tools.error = Some(t!("agents.parity.remote_instructions").to_string());
            cx.notify();
            return;
        }
        self.tools.loading = true;
        let selected = self.sessions.selected_id();
        let requested_context = context.clone();
        let task = cx
            .background_executor()
            .spawn(async move { scoped_instructions(std::path::Path::new(&context.workdir)) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.sessions.selected_id() != selected
                    || this.tools.instructions_request != request_id
                    || this.displayed_context(cx) != requested_context
                    || this.tools.active != Some(AgentToolSurface::Instructions)
                {
                    return;
                }
                this.tools.loading = false;
                match result {
                    Ok(files) => this.tools.instructions = files,
                    Err(error) => this.tools.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn activate_workspace_tool(&mut self, id: &str, cx: &mut Context<Self>) {
        match id {
            "files" => {
                let close = self.tools.active.is_none()
                    && self.inspector_tab == AgentInspectorTab::Files
                    && self.tools.panel_visible;
                self.tools.active = None;
                self.inspector_tab = AgentInspectorTab::Files;
                self.tools.panel_visible = !close;
            }
            "review" => {
                self.tools.active = None;
                self.inspector_tab = AgentInspectorTab::Changes;
                self.tools.panel_visible = true;
                self.refresh_review(cx);
            }
            "terminal" => cx.emit(AgentConsoleEvent::OpenTerminal {
                context: self.displayed_context(cx),
            }),
            "browser" => self.show_live_browser(None, cx),
            "library" => {
                self.tools.active = Some(AgentToolSurface::Library);
                self.tools.panel_visible = true;
            }
            "workflows" => {
                self.tools.active = Some(AgentToolSurface::Workflows);
                self.tools.panel_visible = true;
            }
            "delivery" => {
                self.tools.active = Some(AgentToolSurface::Delivery);
                self.tools.panel_visible = true;
            }
            "sources" => {
                self.tools.active = Some(AgentToolSurface::Sources);
                self.tools.panel_visible = true;
            }
            _ => self.show_project_instructions(cx),
        }
    }

    /// Primary tools share the session header; the full toolbox remains one click away.
    pub(super) fn render_quick_tools(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut row = div().flex().items_center().flex_shrink_0().gap(px(6.0));
        for (id, icon, key) in [
            ("files", "file-text", "agents.inspector.files"),
            ("review", "git-branch", "agents.inspector.changes"),
            ("terminal", "terminal", "agents.parity.terminal"),
        ] {
            row = row.child(
                Button::new(SharedString::from(format!("agent-quick-{id}")), "")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Icon)
                    .w(px(28.0))
                    .h(px(28.0))
                    .icon(IconSource::from(icon))
                    .tooltip(t!(key).to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.activate_workspace_tool(id, cx);
                        cx.notify();
                    })),
            );
        }
        row.child(
            Button::new("agent-more-tools", "")
                .variant(if self.tools.toolbar_expanded {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                })
                .size(ButtonSize::Icon)
                .w(px(28.0))
                .h(px(28.0))
                .icon(IconSource::from("ellipsis"))
                .tooltip(t!("agents.parity.more_tools").to_string())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.tools.toolbar_expanded = !this.tools.toolbar_expanded;
                    cx.notify();
                })),
        )
        .into_any_element()
    }

    pub(super) fn render_workspace_tools(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.0))
            .px(px(10.0))
            .py(px(5.0))
            .border_b_1()
            .border_color(ShellDeckColors::border());
        for (id, icon, key) in [
            ("files", "file-text", "agents.inspector.files"),
            ("review", "git-branch", "agents.inspector.changes"),
            ("terminal", "terminal", "agents.parity.terminal"),
            ("browser", "globe", "agents.browser.title"),
            ("sources", "globe", "agents.parity.sources"),
            ("instructions", "list-checks", "agents.parity.instructions"),
            ("library", "messages-square", "agents.parity.sessions"),
            ("workflows", "calendar", "agents.workflows.title"),
            ("delivery", "upload", "agents.delivery.title"),
        ] {
            let primary = matches!(id, "files" | "review");
            row = row.child(
                Button::new(
                    SharedString::from(format!("agent-tool-{id}")),
                    if primary {
                        t!(key).to_string()
                    } else {
                        String::new()
                    },
                )
                .variant(ButtonVariant::Ghost)
                .size(if primary {
                    ButtonSize::Sm
                } else {
                    ButtonSize::Icon
                })
                .icon(IconSource::from(icon))
                .tooltip(t!(key).to_string())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.activate_workspace_tool(id, cx);
                    cx.notify();
                })),
            );
        }
        row = row.child(div().flex_1()).child(
            Button::new("agent-tool-project", "")
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Icon)
                .icon(IconSource::from("plus"))
                .tooltip(t!("agents.parity.workspace").to_string())
                .on_click(
                    cx.listener(|_, _, _, cx| cx.emit(AgentConsoleEvent::OpenWorkspaceLauncher)),
                ),
        );
        row.into_any_element()
    }

    pub(super) fn render_tools_panel(
        &mut self,
        session: &AgentSession,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title = match self.tools.active {
            Some(AgentToolSurface::Library) => t!("agents.parity.sessions").to_string(),
            Some(AgentToolSurface::Workflows) => t!("agents.workflows.title").to_string(),
            Some(AgentToolSurface::Delivery) => t!("agents.delivery.title").to_string(),
            Some(AgentToolSurface::Sources) => t!("agents.parity.sources").to_string(),
            Some(AgentToolSurface::Terminal) => t!("agents.parity.terminal").to_string(),
            _ => t!("agents.parity.instructions").to_string(),
        };
        let header = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .p(px(8.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(div().flex_1().min_w_0().truncate().child(title))
            .child(
                Button::new("agent-panel-less", "")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Icon)
                    .icon(IconSource::from("minus"))
                    .tooltip(t!("agents.parity.narrow_panel").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.tools.panel_width = (this.tools.panel_width - 60.0).max(260.0);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("agent-panel-more", "")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Icon)
                    .icon(IconSource::from("plus"))
                    .tooltip(t!("agents.parity.widen_panel").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.tools.panel_width = (this.tools.panel_width + 60.0).min(900.0);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("agent-panel-close", "")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Icon)
                    .icon(IconSource::from("x"))
                    .tooltip(t!("agents.parity.close_panel").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.tools.panel_visible = false;
                        cx.notify();
                    })),
            );
        let mut body = div()
            .id("agent-tool-content")
            .flex()
            .flex_col()
            .gap(px(9.0))
            .p(px(12.0))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        if self.tools.loading {
            body = body.child(t!("agents.parity.loading").to_string());
        } else if let Some(error) = &self.tools.error {
            body = body.child(error.clone());
        } else {
            match self.tools.active {
                Some(AgentToolSurface::Sources) => {
                    if session.sources.is_empty() && session.artifacts.is_empty() {
                        body = body.child(t!("agents.parity.no_sources").to_string());
                    }
                    for (index, (title, uri)) in session
                        .sources
                        .iter()
                        .map(|s| (&s.title, &s.uri))
                        .chain(session.artifacts.iter().map(|s| (&s.title, &s.uri)))
                        .enumerate()
                    {
                        let title = title.clone();
                        let uri = uri.clone();
                        let action_uri = uri.clone();
                        body = body
                            .child(
                                Button::new(("agent-source", index), title)
                                    .variant(ButtonVariant::Outline)
                                    .size(ButtonSize::Sm)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(url) = safe_browser_url(&action_uri) {
                                            this.show_live_browser(Some(url), cx);
                                        } else {
                                            this.open_agent_source_file(&action_uri, cx);
                                        }
                                    })),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(uri.clone()),
                            );
                    }
                }
                Some(AgentToolSurface::Instructions) => {
                    if self.tools.instructions.is_empty() {
                        body = body.child(t!("agents.parity.no_instructions").to_string());
                    }
                    for (name, text) in &self.tools.instructions {
                        body = body
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(name.clone()))
                            .child(Markdown::new(text.clone()).compact());
                    }
                    if let Some(metadata) = &session.metadata {
                        for tool in &metadata.available_tools {
                            body =
                                body.child(Badge::new(tool.clone()).variant(BadgeVariant::Outline));
                        }
                    }
                }
                Some(AgentToolSurface::Terminal) => {
                    if let Some(terminal) = &self.tools.terminal {
                        return div()
                            .flex()
                            .flex_col()
                            .size_full()
                            .child(header)
                            .child(div().flex_1().min_h_0().child(terminal.clone()))
                            .into_any_element();
                    }
                }
                Some(AgentToolSurface::Library) => {
                    let parent = cx.entity();
                    body = body
                        .child(self.render_library_controls(cx))
                        .child(
                            Input::new(&self.navigator_search)
                                .size(InputSize::Sm)
                                .placeholder(t!("fleet.sessions.search").to_string())
                                .on_change(move |_, cx| parent.update(cx, |_, cx| cx.notify())),
                        )
                        .child(
                            Button::new("agent-library-new", t!("agents.session.new").to_string())
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::Sm)
                                .icon(IconSource::from("plus"))
                                .on_click(cx.listener(|this, _, _, cx| this.new_session(cx))),
                        )
                        .child(self.render_session_rows(None, cx));
                }
                Some(AgentToolSurface::Workflows) => {
                    body = body.child(self.render_workflow_controls(cx));
                }
                Some(AgentToolSurface::Delivery) => {
                    body = body.child(self.render_delivery_controls(session, cx));
                }
                None => {}
            }
        }
        div()
            .flex()
            .flex_col()
            .size_full()
            .text_size(px(11.0))
            .child(header)
            .child(body)
            .into_any_element()
    }
}

pub(super) fn safe_browser_url(raw: &str) -> Option<String> {
    if raw.len() > 8192 || raw.chars().any(char::is_control) {
        return None;
    }
    let url = url::Url::parse(raw).ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

fn scoped_instructions(root: &std::path::Path) -> Result<Vec<(String, String)>, String> {
    let root = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
    let mut paths = vec![root.join("AGENTS.md"), root.join("CLAUDE.md")];
    if let Ok(entries) = std::fs::read_dir(root.join(".agents")) {
        for entry in entries.flatten().take(64) {
            let path = entry.path();
            if path.extension().is_some_and(|x| x == "md") {
                paths.push(path);
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(root.join(".agents/skills")) {
        for entry in entries.flatten().take(32) {
            paths.push(entry.path().join("SKILL.md"));
        }
    }
    paths.sort();
    paths.dedup();
    let mut result = Vec::new();
    let mut total = 0;
    for path in paths.into_iter().take(64) {
        let Ok(path) = std::fs::canonicalize(path) else {
            continue;
        };
        if !path.starts_with(&root) {
            continue;
        }
        let Ok(metadata) = std::fs::metadata(&path) else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > 64 * 1024 || total + metadata.len() > 512 * 1024
        {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&path) {
            total += metadata.len();
            result.push((
                path.strip_prefix(&root)
                    .unwrap_or(&path)
                    .display()
                    .to_string(),
                text,
            ));
        }
    }
    Ok(result)
}
