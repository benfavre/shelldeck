use super::*;
use crate::file_editor::view::{ExternalEditorDocument, FileEditorEvent, FileEditorView};
use shelldeck_core::config::app_config::EditorConfig;
use std::io::{Read, Write};

const MAX_TEXT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
pub enum AgentFileOrigin {
    Local {
        root: PathBuf,
        path: PathBuf,
        original: String,
    },
    Remote {
        root: String,
        path: String,
        revision: String,
        connection_id: Uuid,
    },
}

#[derive(Debug, Clone)]
pub struct AgentFileSaveRequest {
    pub session_id: Uuid,
    pub editor_id: Uuid,
    pub document_id: String,
    pub request_id: Uuid,
    pub context: AgentExecutionContext,
    pub origin: AgentFileOrigin,
    pub text: String,
}

struct EditorInstance {
    id: Uuid,
    session_id: Uuid,
    context: AgentExecutionContext,
    editor: Entity<FileEditorView>,
    documents: HashMap<String, AgentFileOrigin>,
    visible: bool,
    loading: Option<Uuid>,
    error: Option<String>,
    _subscription: Subscription,
}

#[derive(Default)]
pub(super) struct AgentEditorsUi {
    instances: Vec<EditorInstance>,
    config: EditorConfig,
}

impl AgentConsoleView {
    fn editor_index(&self, cx: &App) -> Option<usize> {
        let id = self.sessions.selected_id()?;
        let context = self.displayed_context(cx);
        self.editors
            .instances
            .iter()
            .position(|e| e.session_id == id && e.context == context)
    }

    fn ensure_agent_editor(&mut self, cx: &mut Context<Self>) -> usize {
        if let Some(index) = self.editor_index(cx) {
            return index;
        }
        let editor_id = Uuid::new_v4();
        let config = self.editors.config.clone();
        let editor = cx.new(|cx| {
            let mut editor = FileEditorView::new_embedded(cx);
            editor.apply_editor_config(&config, cx);
            editor
        });
        let subscription = cx.subscribe(&editor, move |this, editor, event, cx| match event {
            FileEditorEvent::SaveExternal {
                document_id,
                request_id,
                text,
            } => {
                let Some(instance) = this.editors.instances.iter().find(|e| e.id == editor_id)
                else {
                    return;
                };
                let Some(origin) = instance.documents.get(document_id).cloned() else {
                    return;
                };
                let request = AgentFileSaveRequest {
                    session_id: instance.session_id,
                    editor_id,
                    document_id: document_id.clone(),
                    request_id: *request_id,
                    context: instance.context.clone(),
                    origin,
                    text: text.clone(),
                };
                if this.sessions.selected_id() != Some(request.session_id)
                    || this.displayed_context(cx) != request.context
                {
                    editor.update(cx, |editor, cx| {
                        editor.complete_external_save(
                            document_id,
                            *request_id,
                            Err(t!("agents.editor.context_changed").to_string()),
                            cx,
                        )
                    });
                } else {
                    cx.emit(AgentConsoleEvent::SaveFile(request));
                }
            }
            FileEditorEvent::TabsChanged => cx.notify(),
        });
        self.editors.instances.push(EditorInstance {
            id: editor_id,
            session_id: self.sessions.selected_id().expect("selected session"),
            context: self.displayed_context(cx),
            editor,
            documents: HashMap::new(),
            visible: false,
            loading: None,
            error: None,
            _subscription: subscription,
        });
        self.editors.instances.len() - 1
    }

    pub fn apply_agent_editor_config(&mut self, config: &EditorConfig, cx: &mut Context<Self>) {
        self.editors.config = config.clone();
        for instance in &self.editors.instances {
            instance
                .editor
                .update(cx, |editor, cx| editor.apply_editor_config(config, cx));
        }
        cx.notify();
    }

    fn reveal_agent_editor(&mut self, index: usize, cx: &mut Context<Self>) {
        self.hide_live_browser(cx);
        self.editors.instances[index].visible = true;
        // On small panes Files is a drawer; selecting a file reveals the editor underneath.
        if !self.tools.inspector_wide {
            self.tools.panel_visible = false;
        }
        cx.notify();
    }

    pub(super) fn selected_agent_file_path(&self, cx: &App) -> Option<String> {
        let instance = &self.editors.instances[self.editor_index(cx)?];
        let tab = instance.editor.read(cx).active_tab()?;
        if let Some(id) = tab.external_document_id() {
            match instance.documents.get(id)? {
                AgentFileOrigin::Local { path, .. } => Some(path.display().to_string()),
                AgentFileOrigin::Remote { path, .. } => Some(path.clone()),
            }
        } else {
            tab.path.as_ref().map(|p| p.display().to_string())
        }
    }

    pub(super) fn agent_editor_visible(&self, cx: &App) -> bool {
        !self.live_browser_visible(cx)
            && self
                .editor_index(cx)
                .is_some_and(|i| self.editors.instances[i].visible)
    }

    pub(super) fn show_agent_conversation(&mut self, cx: &mut Context<Self>) {
        self.hide_live_browser(cx);
        if let Some(i) = self.editor_index(cx) {
            self.editors.instances[i].visible = false;
        }
        cx.notify();
    }

    pub(super) fn open_agent_source_file(&mut self, uri: &str, cx: &mut Context<Self>) {
        let context = self.displayed_context(cx);
        let Some(path) =
            agent_source_file_path(uri, matches!(context.target, AgentTarget::Ssh { .. }))
        else {
            return;
        };
        match context.target {
            AgentTarget::Local => {
                self.open_local_agent_preview(std::path::Path::new(&context.workdir).join(path), cx)
            }
            AgentTarget::Ssh { connection_id, .. } => {
                if let Some(id) = self.sessions.selected_id() {
                    self.open_remote_agent_preview(id, connection_id, path, cx);
                }
            }
        }
    }

    pub fn open_local_agent_preview(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let context = self.displayed_context(cx);
        if !matches!(context.target, AgentTarget::Local) {
            return;
        }
        let index = self.ensure_agent_editor(cx);
        let retained_id = format!("local:{}", path.display());
        if self.editors.instances[index]
            .editor
            .read(cx)
            .contains_external_document(&retained_id)
        {
            self.editors.instances[index]
                .editor
                .update(cx, |editor, cx| {
                    editor.activate_external_document(&retained_id, cx);
                });
            self.editors.instances[index].loading = None;
            self.editors.instances[index].error = None;
            self.reveal_agent_editor(index, cx);
            return;
        }
        let request = Uuid::new_v4();
        let editor_id = self.editors.instances[index].id;
        self.editors.instances[index].loading = Some(request);
        self.editors.instances[index].error = None;
        self.reveal_agent_editor(index, cx);
        let result = cx
            .background_executor()
            .spawn(async move { load_local_document(&context.workdir, &path) });
        cx.spawn(async move |this, cx| {
            let result = result.await;
            let _ = this.update(cx, |this, cx| {
                let Some(index) = this
                    .editors
                    .instances
                    .iter()
                    .position(|e| e.id == editor_id && e.loading == Some(request))
                else {
                    return;
                };
                this.editors.instances[index].loading = None;
                match result {
                    Ok((root, path, text)) => {
                        if this.editor_index(cx) == Some(index) {
                            this.inspector_local_browser.reveal_path(&path);
                        }
                        let id = format!("local:{}", path.display());
                        let filename = path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        let instance = &mut this.editors.instances[index];
                        if let Some(text) = text {
                            if !instance.editor.read(cx).contains_external_document(&id) {
                                instance.documents.insert(
                                    id.clone(),
                                    AgentFileOrigin::Local {
                                        root,
                                        path: path.clone(),
                                        original: text.clone(),
                                    },
                                );
                            }
                            instance.editor.update(cx, |editor, cx| {
                                editor.open_external_document(
                                    ExternalEditorDocument {
                                        document_id: id,
                                        filename,
                                        display_path: path.display().to_string(),
                                        provenance: t!("agents.inspector.local").to_string(),
                                        text,
                                        read_only: false,
                                    },
                                    cx,
                                );
                            });
                        } else {
                            instance.editor.update(cx, |editor, cx| {
                                editor.open_embedded_preview(path.clone(), cx);
                            });
                        }
                    }
                    Err(error) => this.editors.instances[index].error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn open_remote_agent_preview(
        &mut self,
        session_id: Uuid,
        connection_id: Uuid,
        path: String,
        cx: &mut Context<Self>,
    ) {
        if self.sessions.selected_id() != Some(session_id)
            || !matches!(self.displayed_context(cx).target, AgentTarget::Ssh { connection_id: id, .. } if id == connection_id)
        {
            return;
        }
        let index = self.ensure_agent_editor(cx);
        let id = format!("remote:{connection_id}:{path}");
        let instance = &mut self.editors.instances[index];
        if instance.editor.read(cx).contains_external_document(&id) {
            instance.editor.update(cx, |editor, cx| {
                editor.activate_external_document(&id, cx);
            });
            instance.loading = None;
        } else {
            let request_id = Uuid::new_v4();
            instance.loading = Some(request_id);
            cx.emit(AgentConsoleEvent::OpenRemoteFile {
                context: instance.context.clone(),
                session_id,
                request_id,
                connection_id,
                path: path.clone(),
            });
        }
        self.editors.instances[index].error = None;
        self.reveal_agent_editor(index, cx);
    }

    pub fn set_remote_file_document(
        &mut self,
        session_id: Uuid,
        request_id: Uuid,
        result: Result<shelldeck_ssh::agent_files::Document, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .editors
            .instances
            .iter()
            .position(|e| e.session_id == session_id && e.loading == Some(request_id))
        else {
            return;
        };
        let instance = &mut self.editors.instances[index];
        instance.loading = None;
        match result {
            Ok(document) => {
                let AgentTarget::Ssh { connection_id, .. } = instance.context.target else {
                    return;
                };
                let id = format!("remote:{connection_id}:{}", document.path);
                let filename = document
                    .path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&document.path)
                    .to_string();
                if !instance.editor.read(cx).contains_external_document(&id) {
                    instance.documents.insert(
                        id.clone(),
                        AgentFileOrigin::Remote {
                            root: document.root,
                            path: document.path.clone(),
                            revision: document.revision,
                            connection_id,
                        },
                    );
                }
                instance.editor.update(cx, |editor, cx| {
                    editor.open_external_document(
                        ExternalEditorDocument {
                            document_id: id,
                            filename,
                            display_path: document.path.clone(),
                            provenance: target_label(&instance.context.target),
                            text: document.text,
                            read_only: false,
                        },
                        cx,
                    );
                });
                instance.error = None;
            }
            Err(error) => instance.error = Some(error),
        }
        cx.notify();
    }

    pub fn complete_agent_file_save(
        &mut self,
        request: &AgentFileSaveRequest,
        result: Result<AgentFileOrigin, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(instance) = self.editors.instances.iter_mut().find(|e| {
            e.id == request.editor_id
                && e.session_id == request.session_id
                && e.context == request.context
        }) else {
            return;
        };
        if !instance
            .editor
            .read(cx)
            .has_external_save(&request.document_id, request.request_id)
        {
            return;
        }
        let acknowledgement = match result {
            Ok(origin) => {
                instance
                    .documents
                    .insert(request.document_id.clone(), origin);
                Ok(())
            }
            Err(error) => Err(error),
        };
        instance.editor.update(cx, |editor, cx| {
            editor.complete_external_save(
                &request.document_id,
                request.request_id,
                acknowledgement,
                cx,
            )
        });
        cx.notify();
    }

    pub(super) fn render_middle_switcher(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let active = self.agent_editor_visible(cx);
        let browser_active = self.live_browser_visible(cx);
        let index = self.editor_index(cx);
        let count = index
            .map(|i| {
                self.editors.instances[i]
                    .editor
                    .read(cx)
                    .tabs
                    .iter()
                    .filter(|t| t.path.is_some() || t.external_document_id().is_some())
                    .count()
            })
            .unwrap_or(0);
        let mut row = div()
            .flex()
            .items_center()
            .flex_shrink_0()
            .gap(px(6.0))
            .px(px(10.0))
            .py(px(5.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                Button::new(
                    "agent-middle-chat",
                    t!("agents.editor.conversation").to_string(),
                )
                .variant(if active || browser_active {
                    ButtonVariant::Ghost
                } else {
                    ButtonVariant::Secondary
                })
                .size(ButtonSize::Sm)
                .icon(IconSource::from("bot"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.show_agent_conversation(cx);
                    if let Some(ui) = this
                        .sessions
                        .selected_id()
                        .and_then(|id| this.session_ui.get(&id))
                    {
                        ui.prompt.read(cx).focus_handle(cx).focus(window);
                    }
                })),
            )
            .child(
                Button::new(
                    "agent-middle-editor",
                    format!(
                        "{}{}",
                        t!("agents.editor.title"),
                        if count > 0 {
                            format!(" · {count}")
                        } else {
                            String::new()
                        }
                    ),
                )
                .variant(if active {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                })
                .size(ButtonSize::Sm)
                .icon(IconSource::from("file-text"))
                .disabled(index.is_none())
                .on_click(cx.listener(|this, _, window, cx| {
                    if let Some(index) = this.editor_index(cx) {
                        this.hide_live_browser(cx);
                        this.editors.instances[index].visible = true;
                        this.editors.instances[index]
                            .editor
                            .read(cx)
                            .focus_handle
                            .focus(window);
                        cx.notify();
                    }
                })),
            );
        row = row.child(
            Button::new(
                "agent-middle-browser",
                t!("agents.live_browser.title").to_string(),
            )
            .variant(if browser_active {
                ButtonVariant::Secondary
            } else {
                ButtonVariant::Ghost
            })
            .size(ButtonSize::Sm)
            .icon(IconSource::from("globe"))
            .on_click(cx.listener(|this, _, _, cx| this.show_live_browser(None, cx))),
        );
        if browser_active {
            row = row.child(
                Button::new(
                    "agent-close-browser",
                    t!("agents.live_browser.close").to_string(),
                )
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Sm)
                .icon(IconSource::from("x"))
                .on_click(cx.listener(|this, _, _, cx| this.close_current_browser(cx))),
            );
        }
        if let Some(error) = &self.live_browsers.error {
            row = row.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::error())
                    .child(error.clone()),
            );
        }
        if active {
            row = row.child(div().flex_1()).child(
                Button::new("agent-editor-save", t!("agents.editor.save").to_string())
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .icon(IconSource::from("save"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(index) = this.editor_index(cx) {
                            this.editors.instances[index]
                                .editor
                                .update(cx, |editor, cx| editor.save_file(cx));
                        }
                    })),
            );
        }
        row.into_any_element()
    }

    pub(super) fn render_middle_editor(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(index) = self.editor_index(cx) else {
            return div().into_any_element();
        };
        let instance = &self.editors.instances[index];
        let mut body = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_hidden();
        if instance.loading.is_some() {
            body = body.child(
                div()
                    .px(px(12.0))
                    .py(px(8.0))
                    .text_size(px(12.0))
                    .child(t!("agents.parity.loading").to_string()),
            );
        }
        if let Some(error) = &instance.error {
            body = body.child(
                div()
                    .px(px(12.0))
                    .py(px(8.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::error())
                    .child(error.clone()),
            );
        }
        body.child(
            div()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .child(instance.editor.clone()),
        )
        .into_any_element()
    }
}

fn agent_source_file_path(raw: &str, remote: bool) -> Option<String> {
    if std::path::Path::new(raw).is_absolute() {
        return Some(raw.to_string());
    }
    let Ok(url) = url::Url::parse(raw) else {
        return Some(raw.to_string());
    };
    if url.scheme() != "file"
        || url.host_str().is_some_and(|host| host != "localhost")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    if !remote {
        return url
            .to_file_path()
            .ok()
            .map(|p| p.to_string_lossy().into_owned());
    }
    // Remote paths use POSIX semantics even when the desktop runs on Windows.
    let mut bytes = Vec::new();
    let mut encoded = url.path().as_bytes().iter().copied();
    while let Some(byte) = encoded.next() {
        if byte == b'%' {
            let hi = (encoded.next()? as char).to_digit(16)?;
            let lo = (encoded.next()? as char).to_digit(16)?;
            bytes.push((hi * 16 + lo) as u8);
        } else {
            bytes.push(byte);
        }
    }
    let path = String::from_utf8(bytes).ok()?;
    shelldeck_core::config::workspace_catalog::RemotePosixPath::new(path.clone()).ok()?;
    Some(path)
}

fn load_local_document(
    root: &str,
    requested: &std::path::Path,
) -> Result<(PathBuf, PathBuf, Option<String>), String> {
    let root = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
    let metadata = std::fs::symlink_metadata(requested).map_err(|e| e.to_string())?;
    let path = std::fs::canonicalize(requested).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || !path.starts_with(&root) {
        return Err(t!("agents.editor.outside_root").to_string());
    }
    if metadata.len() > MAX_TEXT_BYTES as u64 {
        return Err(t!("agents.editor.too_large").to_string());
    }
    if crate::file_editor::FileKind::from_filename(&path.to_string_lossy())
        != crate::file_editor::FileKind::Text
    {
        return Ok((root, path, None));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .map_err(|e| e.to_string())?
        .take(MAX_TEXT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_TEXT_BYTES {
        return Err(t!("agents.editor.too_large").to_string());
    }
    let text = String::from_utf8(bytes).map_err(|_| t!("agents.editor.binary").to_string())?;
    if text.contains('\0') {
        return Err(t!("agents.editor.binary").to_string());
    }
    Ok((root, path, Some(text)))
}

/// Apply one explicit human save against its captured local content snapshot.
/// Scope and content are rechecked before the atomic file replacement.
pub fn save_local_document(
    context_root: &str,
    origin: AgentFileOrigin,
    text: String,
) -> Result<AgentFileOrigin, String> {
    let AgentFileOrigin::Local {
        root,
        path,
        original,
    } = origin
    else {
        return Err(t!("agents.editor.context_changed").to_string());
    };
    if text.len() > MAX_TEXT_BYTES {
        return Err(t!("agents.editor.too_large").to_string());
    }
    let (current_root, current_path, current) = load_local_document(context_root, &path)?;
    if current_root != root || current_path != path {
        return Err(t!("agents.editor.context_changed").to_string());
    }
    if current.as_deref() != Some(original.as_str()) {
        return Err(t!("agents.editor.conflict").to_string());
    }
    let permissions = std::fs::metadata(&path)
        .map_err(|e| e.to_string())?
        .permissions();
    let mut temporary = tempfile::NamedTempFile::new_in(
        path.parent()
            .ok_or_else(|| t!("agents.editor.outside_root").to_string())?,
    )
    .map_err(|e| e.to_string())?;
    temporary
        .as_file()
        .set_permissions(permissions)
        .map_err(|e| e.to_string())?;
    temporary
        .write_all(text.as_bytes())
        .map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    let (_, final_path, final_text) = load_local_document(context_root, &path)?;
    if final_path != path || final_text.as_deref() != Some(original.as_str()) {
        return Err(t!("agents.editor.conflict").to_string());
    }
    temporary.persist(&path).map_err(|e| e.to_string())?;
    Ok(AgentFileOrigin::Local {
        root,
        path,
        original: text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    // SDTEST-1909 — SDUC-499, SDUC-551
    #[::core::prelude::v1::test]
    fn sdtest_1909_agent_file_open_stays_inside_the_selected_local_session_root() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let admitted = root.path().join("src.rs");
        let refused = outside.path().join("secret.txt");
        std::fs::write(&admitted, "safe").unwrap();
        std::fs::write(&refused, "outside").unwrap();
        let root_path = root.path().to_str().unwrap();
        assert_eq!(
            load_local_document(root_path, &admitted)
                .unwrap()
                .2
                .as_deref(),
            Some("safe")
        );
        assert!(load_local_document(root_path, &refused).is_err());
        #[cfg(unix)]
        {
            let escape = root.path().join("escape.txt");
            std::os::unix::fs::symlink(&refused, &escape).unwrap();
            assert!(load_local_document(root_path, &escape).is_err());
        }
    }

    // SDTEST-2522 — SDUC-551
    #[::core::prelude::v1::test]
    fn remote_source_file_urls_keep_posix_paths_on_every_desktop() {
        assert_eq!(
            agent_source_file_path("file:///srv/my%20project/a%2Bb.rs", true).as_deref(),
            Some("/srv/my project/a+b.rs")
        );
        assert_eq!(
            agent_source_file_path("src/a.rs", true).as_deref(),
            Some("src/a.rs")
        );
        assert!(agent_source_file_path("file://another-host/srv/a.rs", true).is_none());
        assert!(agent_source_file_path("file:///srv/a%00.rs", true).is_none());
        assert!(agent_source_file_path("ssh://host/srv/a.rs", true).is_none());
    }

    // SDTEST-2520 — SDUC-551
    #[::core::prelude::v1::test]
    fn local_editor_save_preserves_external_changes_and_scopes_the_target() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let path = root.path().join("example.rs");
        std::fs::write(&path, "original\n").unwrap();
        let (canonical_root, canonical_path, text) =
            load_local_document(root.path().to_str().unwrap(), &path).unwrap();
        let origin = AgentFileOrigin::Local {
            root: canonical_root,
            path: canonical_path,
            original: text.unwrap(),
        };
        std::fs::write(&path, "other writer\n").unwrap();
        assert!(save_local_document(
            root.path().to_str().unwrap(),
            origin.clone(),
            "edited\n".into()
        )
        .is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "other writer\n");
        assert!(save_local_document(
            other.path().to_str().unwrap(),
            origin.clone(),
            "edited\n".into()
        )
        .is_err());
        std::fs::write(&path, "original\n").unwrap();
        assert!(
            save_local_document(root.path().to_str().unwrap(), origin, "edited\n".into()).is_ok()
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), "edited\n");
    }
}
