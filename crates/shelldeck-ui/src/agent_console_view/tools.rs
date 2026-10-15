use super::*;
use crate::file_editor::view::FileEditorView;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AgentToolSurface {
    Preview,
    Sources,
    Instructions,
    Terminal,
    Browser,
    Library,
    Workflows,
    Delivery,
}

pub(super) struct AgentToolsUi {
    pub active: Option<AgentToolSurface>,
    pub panel_visible: bool,
    pub navigator_visible: bool,
    pub panel_width: f32,
    pub measured_width: f32,
    pub editor: Entity<FileEditorView>,
    preview_editors: HashMap<PathBuf, Entity<FileEditorView>>,
    pub preview_title: String,
    pub remote_preview: Option<String>,
    pub instructions: Vec<(String, String)>,
    pub loading: bool,
    pub error: Option<String>,
    pub preview_session: Option<Uuid>,
    pub preview_request: Uuid,
    pub preview_context: Option<AgentExecutionContext>,
    pub instructions_request: Uuid,
    pub inspector_context: Option<AgentExecutionContext>,
    pub terminal: Option<Entity<crate::terminal_view::TerminalView>>,
}
impl AgentToolsUi {
    pub fn new(cx: &mut Context<AgentConsoleView>) -> Self {
        Self {
            active: None,
            panel_visible: false,
            navigator_visible: true,
            panel_width: 360.0,
            measured_width: 0.0,
            editor: cx.new(FileEditorView::new),
            preview_editors: HashMap::new(),
            preview_title: String::new(),
            remote_preview: None,
            instructions: Vec::new(),
            loading: false,
            error: None,
            preview_session: None,
            preview_request: Uuid::new_v4(),
            preview_context: None,
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
        let target = self
            .selected_connection
            .and_then(|id| self.connections.iter().find(|c| c.id == id))
            .map(|c| AgentTarget::Ssh {
                connection_id: c.id,
                connection_label: c.label.clone(),
            })
            .unwrap_or(AgentTarget::Local);
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

    pub fn open_local_agent_preview(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let context = self.displayed_context(cx);
        let Some(session_id) = self.sessions.selected_id() else {
            return;
        };
        if !matches!(context.target, AgentTarget::Local) {
            return;
        }
        self.tools.active = Some(AgentToolSurface::Preview);
        self.tools.panel_visible = true;
        self.tools.preview_session = Some(session_id);
        let request_id = Uuid::new_v4();
        self.tools.preview_request = request_id;
        self.tools.preview_context = Some(context.clone());
        self.tools.loading = true;
        self.tools.error = None;
        self.tools.remote_preview = None;
        let preview_context = context.clone();
        let result = cx.background_executor().spawn(async move {
            let root = std::fs::canonicalize(&context.workdir).map_err(|e| e.to_string())?;
            let path = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
            let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
            if !path.starts_with(root) || !metadata.is_file() || metadata.len() > 8 * 1024 * 1024 {
                return Err(
                    "File is outside this session or exceeds the 8 MiB preview limit".to_string(),
                );
            }
            Ok(path)
        });
        cx.spawn(async move |this, cx| {
            let result = result.await;
            let _ = this.update(cx, |this, cx| {
                if this.sessions.selected_id() != Some(session_id)
                    || this.tools.preview_request != request_id
                    || this.displayed_context(cx) != preview_context
                {
                    return;
                }
                this.tools.loading = false;
                match result {
                    Ok(path) => {
                        this.tools.preview_title = path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();
                        let editor = this
                            .tools
                            .preview_editors
                            .entry(path.clone())
                            .or_insert_with(|| {
                                cx.new(|cx| {
                                    let mut editor = FileEditorView::new(cx);
                                    editor.file_browser_visible = false;
                                    editor.open_file(path, cx);
                                    editor
                                })
                            })
                            .clone();
                        this.tools.editor = editor;
                    }
                    Err(error) => this.tools.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn set_remote_file_preview(
        &mut self,
        session_id: Uuid,
        request_id: Uuid,
        path: String,
        result: Result<String, String>,
        cx: &mut Context<Self>,
    ) {
        if self.sessions.selected_id() != Some(session_id)
            || self.tools.preview_request != request_id
            || self.tools.preview_context.as_ref() != Some(&self.displayed_context(cx))
        {
            return;
        }
        self.tools.loading = false;
        self.tools.preview_title = path;
        match result {
            Ok(text) => self.tools.remote_preview = Some(text),
            Err(error) => self.tools.error = Some(error),
        }
        cx.notify();
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
                .on_click(cx.listener(move |this, _, window, cx| {
                    match id {
                        "files" => {
                            let close = this.tools.active.is_none()
                                && this.inspector_tab == AgentInspectorTab::Files
                                && this.tools.panel_visible;
                            this.tools.active = None;
                            this.inspector_tab = AgentInspectorTab::Files;
                            this.tools.panel_visible = !close;
                        }
                        "review" => {
                            this.tools.active = None;
                            this.inspector_tab = AgentInspectorTab::Changes;
                            this.tools.panel_visible = true;
                            this.refresh_review(cx);
                        }
                        "terminal" => cx.emit(AgentConsoleEvent::OpenTerminal {
                            context: this.displayed_context(cx),
                        }),
                        "browser" => this.show_agent_browser(None, window, cx),
                        "library" => {
                            this.tools.active = Some(AgentToolSurface::Library);
                            this.tools.panel_visible = true;
                        }
                        "workflows" => {
                            this.tools.active = Some(AgentToolSurface::Workflows);
                            this.tools.panel_visible = true;
                        }
                        "delivery" => {
                            this.tools.active = Some(AgentToolSurface::Delivery);
                            this.tools.panel_visible = true;
                        }
                        "sources" => {
                            this.tools.active = Some(AgentToolSurface::Sources);
                            this.tools.panel_visible = true;
                        }
                        _ => this.show_project_instructions(cx),
                    }
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
        if self.tools.active == Some(AgentToolSurface::Browser) {
            return self.render_browser_panel(cx);
        }
        let title = match self.tools.active {
            Some(AgentToolSurface::Library) => t!("agents.parity.sessions").to_string(),
            Some(AgentToolSurface::Workflows) => t!("agents.workflows.title").to_string(),
            Some(AgentToolSurface::Delivery) => t!("agents.delivery.title").to_string(),
            Some(AgentToolSurface::Preview) => self.tools.preview_title.clone(),
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
                                            cx.open_url(&url);
                                        } else {
                                            let context = this.displayed_context(cx);
                                            let path = std::path::Path::new(&context.workdir)
                                                .join(&action_uri);
                                            this.open_local_agent_preview(path, cx);
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
                Some(AgentToolSurface::Preview) => {
                    if let Some(text) = &self.tools.remote_preview {
                        for line in text.lines() {
                            body = body.child(
                                div()
                                    .font_family("JetBrains Mono")
                                    .text_size(px(11.0))
                                    .child(line.to_string()),
                            );
                        }
                    } else if self.tools.preview_session == Some(session.id) {
                        return div()
                            .flex()
                            .flex_col()
                            .size_full()
                            .child(header)
                            .child(div().flex_1().min_h_0().child(self.tools.editor.clone()))
                            .into_any_element();
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
                Some(AgentToolSurface::Browser) => {}
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
