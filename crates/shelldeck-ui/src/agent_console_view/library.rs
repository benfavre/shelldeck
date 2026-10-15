//! Conversation-library UI and asynchronous persistence. Presentation state is
//! intentionally independent from Workspace's authoritative account config.
use super::*;
use shelldeck_core::agent_session::storage::{
    AgentConsolePreferences, AgentSessionSnapshot, AgentSessionStore,
};
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;

pub(super) struct AgentLibraryUi {
    pub preferences: AgentConsolePreferences,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub rename_session: Option<Uuid>,
    pub rename_state: Entity<InputState>,
    store: AgentSessionStore,
    loading: bool,
    writable: bool,
    generation: Arc<AtomicU64>,
    write_lock: Arc<parking_lot::Mutex<()>>,
    save_task: Option<Task<()>>,
    restore_task: Option<Task<()>>,
}

impl AgentLibraryUi {
    pub fn new(cx: &mut Context<AgentConsoleView>) -> Self {
        Self {
            preferences: AgentConsolePreferences::default(),
            error: None,
            notice: None,
            rename_session: None,
            rename_state: input_state(cx, String::new()),
            store: AgentSessionStore::default(),
            loading: false,
            writable: true,
            generation: Arc::new(AtomicU64::new(0)),
            write_lock: Arc::new(parking_lot::Mutex::new(())),
            save_task: None,
            restore_task: None,
        }
    }
    pub fn show_archived(&self) -> bool {
        self.preferences.show_archived
    }
}

impl AgentConsoleView {
    pub(super) fn restore_library(&mut self, cx: &mut Context<Self>) {
        if self.library.loading {
            return;
        }
        self.library.loading = true;
        let store = self.library.store.clone();
        let generation = self.library.generation.load(Ordering::SeqCst);
        self.library.restore_task = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            let loaded = cx
                .background_executor()
                .spawn(async move { store.load(now_ms()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.library.loading = false;
                match loaded {
                    Ok(snapshot) => {
                        let surface_visible = this.sessions.surface_visible();
                        this.library.writable = true;
                        this.library.error = None;
                        let untouched = this.library.generation.load(Ordering::SeqCst)
                            == generation
                            && this.sessions.sessions().len() == 1
                            && this.sessions.sessions().iter().all(|session| {
                                session.messages.is_empty() && !session.status.is_active()
                            })
                            && this
                                .session_ui
                                .values()
                                .all(|ui| ui.prompt.read(cx).content().is_empty());
                        if !snapshot.collection.sessions().is_empty() {
                            if untouched {
                                this.sessions = snapshot.collection;
                                this.session_ui.clear();
                            } else {
                                this.sessions.merge_restored(snapshot.collection);
                            }
                            for session in this.sessions.sessions() {
                                let ui = this
                                    .session_ui
                                    .entry(session.id)
                                    .or_insert_with(|| AgentSessionUi::new(cx));
                                if ui.prompt.read(cx).content().is_empty() {
                                    if let Some(draft) = snapshot.drafts.get(&session.id) {
                                        ui.prompt.update(cx, |state, cx| {
                                            state.replace_content(draft.clone(), cx)
                                        });
                                    }
                                }
                            }
                        }
                        if untouched {
                            this.library.preferences = snapshot.preferences;
                            this.tools.navigator_visible =
                                this.library.preferences.navigator_visible;
                            this.tools.panel_visible = this.library.preferences.inspector_visible;
                            this.tools.panel_width =
                                f32::from(this.library.preferences.inspector_width);
                            this.context_expanded = this.library.preferences.context_expanded;
                            this.inspector_tab =
                                if this.library.preferences.inspector_tab == "changes" {
                                    AgentInspectorTab::Changes
                                } else {
                                    AgentInspectorTab::Files
                                };
                        }
                        if !snapshot.warnings.is_empty() {
                            this.library.notice = Some(snapshot.warnings.join("\n"));
                        }
                        if this.sessions.selected_id().is_none() {
                            this.new_session(cx);
                        } else {
                            this.sessions.set_surface_visible(surface_visible);
                            this.apply_selected_context(cx);
                        }
                        this.schedule_library_save(cx);
                    }
                    Err(error) => {
                        // A damaged/newer file must never be overwritten by a
                        // fresh empty library on the next input event.
                        this.library.writable = false;
                        this.library.error =
                            Some(format!("{}: {error}", t!("agents.library.load_failed")));
                    }
                }
                cx.notify();
            });
        }));
    }

    fn library_snapshot(&self, cx: &Context<Self>) -> AgentSessionSnapshot {
        let drafts = self
            .session_ui
            .iter()
            .filter_map(|(id, ui)| {
                let draft = ui.prompt.read(cx).content().to_string();
                (!draft.is_empty()).then_some((*id, draft))
            })
            .collect::<BTreeMap<_, _>>();
        let mut preferences = self.library.preferences.clone();
        preferences.navigator_visible = self.tools.navigator_visible;
        preferences.inspector_visible = self.tools.panel_visible;
        preferences.inspector_width = self.tools.panel_width as u16;
        preferences.context_expanded = self.context_expanded;
        preferences.inspector_tab = match self.inspector_tab {
            AgentInspectorTab::Files => "files",
            AgentInspectorTab::Changes => "changes",
        }
        .into();
        AgentSessionSnapshot {
            collection: self.sessions.clone(),
            drafts,
            preferences,
            warnings: Vec::new(),
        }
    }

    /// Coalesce streaming/draft updates. The generation is checked under the
    /// write lock, so an older background save cannot replace a newer one.
    pub(super) fn schedule_library_save(&mut self, cx: &mut Context<Self>) {
        let revision = self.library.generation.fetch_add(1, Ordering::SeqCst) + 1;
        if self.library.loading || !self.library.writable {
            return;
        }
        let generation = self.library.generation.clone();
        let write_lock = self.library.write_lock.clone();
        let store = self.library.store.clone();
        self.library.save_task = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            cx.background_executor()
                .timer(Duration::from_millis(350))
                .await;
            let Ok(snapshot) = this.update(cx, |this, cx| this.library_snapshot(cx)) else {
                return;
            };
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _guard = write_lock.lock();
                    if generation.load(Ordering::SeqCst) != revision {
                        return Ok(());
                    }
                    store.save(&snapshot)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.library.generation.load(Ordering::SeqCst) != revision {
                    return;
                }
                if let Err(error) = result {
                    this.library.error =
                        Some(format!("{}: {error}", t!("agents.library.save_failed")));
                }
                cx.notify();
            });
        }));
    }

    /// Shutdown is the explicit durability boundary for the final keystroke.
    /// The bounded synchronous write drains/fences older background saves.
    pub fn flush_library(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        if !self.library.writable {
            return Err(t!("agents.library.load_failed").to_string());
        }
        self.library.generation.fetch_add(1, Ordering::SeqCst);
        self.library.save_task = None;
        let mut snapshot = self.library_snapshot(cx);
        let _guard = self.library.write_lock.lock();
        if self.library.loading {
            // Quitting before startup I/O finishes must not replace the saved
            // library with only the new placeholder or an early typed draft.
            let restored = self
                .library
                .store
                .load(now_ms())
                .map_err(|error| error.to_string())?;
            let has_current_work = !snapshot.drafts.is_empty()
                || snapshot
                    .collection
                    .sessions()
                    .iter()
                    .any(|session| !session.messages.is_empty() || session.status.is_active())
                || snapshot.collection.sessions().len() > 1;
            if has_current_work {
                snapshot.collection.merge_restored(restored.collection);
                for (id, draft) in restored.drafts {
                    snapshot.drafts.entry(id).or_insert(draft);
                }
            } else {
                snapshot = restored;
            }
        }
        self.library
            .store
            .save(&snapshot)
            .map_err(|error| error.to_string())
    }

    fn rename_library_session(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.library.rename_session else {
            return;
        };
        let name = self
            .library
            .rename_state
            .read(cx)
            .content()
            .trim()
            .to_string();
        match self.sessions.rename(id, name, now_ms()) {
            Ok(()) => {
                self.library.rename_session = None;
                self.library.error = None;
                self.schedule_library_save(cx);
            }
            Err(error) => self.library.error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn fork_library_session(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get(id) else {
            return;
        };
        let base = session.name.chars().take(24).collect::<String>();
        let name = t!("agents.library.copy_name", name = base).to_string();
        match self.sessions.duplicate(id, name, now_ms()) {
            Ok(id) => {
                self.session_ui.insert(id, AgentSessionUi::new(cx));
                self.select_session(id, cx);
                self.schedule_library_save(cx);
            }
            Err(error) => self.library.error = Some(error.to_string()),
        }
        cx.notify();
    }

    pub(super) fn archive_library_session(
        &mut self,
        id: Uuid,
        archived: bool,
        cx: &mut Context<Self>,
    ) {
        match self.sessions.archive(id, archived, now_ms()) {
            Ok(()) => {
                if self.sessions.selected_id().is_none() {
                    self.new_session(cx);
                } else {
                    self.apply_selected_context(cx);
                }
                self.schedule_library_save(cx);
            }
            Err(error) => self.library.error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn export_library_session(&mut self, id: Uuid, json: bool, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get(id) else {
            return;
        };
        let content = if json {
            match session.export_json() {
                Ok(content) => content,
                Err(error) => {
                    self.library.error = Some(error.to_string());
                    cx.notify();
                    return;
                }
            }
        } else {
            session.export_markdown()
        };
        let name = format!(
            "shelldeck-conversation-{id}.{}",
            if json { "json" } else { "md" }
        );
        let receiver = cx.prompt_for_new_path(&std::env::temp_dir(), Some(&name));
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let Ok(Ok(Some(path))) = receiver.await else {
                return;
            };
            let result = cx
                .background_executor()
                .spawn(async move {
                    if std::fs::symlink_metadata(&path)
                        .is_ok_and(|metadata| metadata.file_type().is_symlink())
                    {
                        return Err(
                            "refusing to write a conversation export through a symlink".to_string()
                        );
                    }
                    // Existing file replacement follows the native save dialog's
                    // confirmation; exported transcripts remain private on Unix.
                    let mut options = std::fs::OpenOptions::new();
                    options.write(true).create(true).truncate(true);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::OpenOptionsExt;
                        options.mode(0o600);
                    }
                    let mut file = options.open(path).map_err(|error| error.to_string())?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        file.set_permissions(std::fs::Permissions::from_mode(0o600))
                            .map_err(|error| error.to_string())?;
                    }
                    std::io::Write::write_all(&mut file, content.as_bytes())
                        .map_err(|error| error.to_string())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.library.notice = Some(t!("agents.library.exported").to_string()),
                    Err(error) => this.library.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_library_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut shell = div().flex().flex_col().flex_shrink_0().min_w(px(0.0));
        if let Some(session) = self.sessions.selected() {
            let id = session.id;
            let pinned = session.pinned;
            let archived = session.archived;
            let active = session.status.is_active();
            shell = shell.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(12.0))
                    .py(px(5.0))
                    .child(
                        Button::new(
                            "agent-library-pin",
                            t!(if pinned {
                                "agents.library.unpin"
                            } else {
                                "agents.library.pin"
                            })
                            .to_string(),
                        )
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .icon(IconSource::from("pin"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let _ = this.sessions.set_pinned(id, !pinned, now_ms());
                            this.schedule_library_save(cx);
                            cx.notify();
                        })),
                    )
                    .child(
                        Button::new(
                            "agent-library-rename",
                            t!("agents.library.rename").to_string(),
                        )
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .icon(IconSource::from("pencil"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(session) = this.sessions.get(id) {
                                this.library.rename_state = input_state(cx, session.name.clone());
                                this.library.rename_session = Some(id);
                                cx.notify();
                            }
                        })),
                    )
                    .child(
                        Button::new("agent-library-fork", t!("agents.library.fork").to_string())
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .icon(IconSource::from("copy"))
                            .on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.fork_library_session(id, cx)
                                }),
                            ),
                    )
                    .child(
                        Button::new(
                            "agent-library-archive",
                            t!(if archived {
                                "agents.library.restore"
                            } else {
                                "agents.library.archive"
                            })
                            .to_string(),
                        )
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .icon(IconSource::from(if archived {
                            "archive-restore"
                        } else {
                            "archive"
                        }))
                        .disabled(active)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.archive_library_session(id, !archived, cx)
                        })),
                    )
                    .child(
                        Button::new(
                            "agent-library-copy",
                            t!("agents.library.copy_chat").to_string(),
                        )
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(session) = this.sessions.get(id) {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    session.export_markdown(),
                                ));
                                this.library.notice = Some(t!("agents.library.copied").to_string());
                                cx.notify();
                            }
                        })),
                    )
                    .child(
                        Button::new(
                            "agent-library-export-md",
                            t!("agents.library.export_md").to_string(),
                        )
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .icon(IconSource::from("download"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.export_library_session(id, false, cx)
                        })),
                    )
                    .child(
                        Button::new(
                            "agent-library-export-json",
                            t!("agents.library.export_json").to_string(),
                        )
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.export_library_session(id, true, cx)
                        })),
                    ),
            );
        }
        if self.library.rename_session.is_some() {
            shell = shell.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(12.0))
                    .py(px(6.0))
                    .child(
                        div().flex_1().min_w(px(0.0)).child(
                            Input::new(&self.library.rename_state)
                                .size(InputSize::Sm)
                                .w_full()
                                .placeholder(t!("agents.library.rename").to_string()),
                        ),
                    )
                    .child(
                        Button::new(
                            "agent-library-save-name",
                            t!("agents.library.save").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .on_click(cx.listener(|this, _, _, cx| this.rename_library_session(cx))),
                    )
                    .child(
                        Button::new(
                            "agent-library-cancel-name",
                            t!("agents.library.cancel").to_string(),
                        )
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.library.rename_session = None;
                            cx.notify();
                        })),
                    ),
            );
        }
        if let Some(error) = &self.library.error {
            shell = shell.child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.0))
                    .px(px(12.0))
                    .py(px(6.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::error())
                            .child(error.clone()),
                    )
                    .child(
                        Button::new(
                            "agent-library-retry-load",
                            t!("agents.library.retry").to_string(),
                        )
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.restore_library(cx);
                            cx.notify();
                        })),
                    ),
            );
        } else if let Some(notice) = &self.library.notice {
            shell = shell.child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.0))
                    .px(px(12.0))
                    .py(px(4.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(notice.clone()),
                    )
                    .child(
                        Button::new("agent-library-dismiss-notice", "")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Icon)
                            .icon(IconSource::from("x"))
                            .tooltip(t!("agents.library.dismiss").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.library.notice = None;
                                cx.notify();
                            })),
                    ),
            );
        }
        shell.into_any_element()
    }
}
