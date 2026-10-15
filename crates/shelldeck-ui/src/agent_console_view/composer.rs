//! Session-scoped input, explicit context intake and queued follow-up turns.
use super::*;
use crate::attachment_annotator::AttachmentAnnotator;
use crate::issue_attachments::{AttachmentDraft, AttachmentLightbox, LightboxItem};
use shelldeck_core::agent_runtime::{AgentInputOptions, AgentReasoningEffort};
use shelldeck_core::ai::{
    AiAttachment, AI_ATTACHMENT_IMAGE_MAX_BYTES, AI_ATTACHMENT_MAX_COUNT,
    AI_ATTACHMENT_TEXT_MAX_BYTES,
};
use std::collections::VecDeque;
use std::sync::Arc;

const MAX_ATTACHMENTS: usize = AI_ATTACHMENT_MAX_COUNT;
const MAX_TEXT_ATTACHMENT_BYTES: u64 = AI_ATTACHMENT_TEXT_MAX_BYTES as u64;
const MAX_IMAGE_BYTES: u64 = AI_ATTACHMENT_IMAGE_MAX_BYTES as u64;

#[derive(Clone)]
struct AgentAttachment {
    path: PathBuf,
    name: String,
    text: Option<String>,
    preview: Option<Arc<Image>>,
}

#[derive(Clone, PartialEq, Eq)]
struct QueuedEditSnapshot {
    context: AgentExecutionContext,
    prompt: String,
    options: AgentInputOptions,
}

struct RestoredQueuedDraft {
    prompt: String,
    images: Vec<AgentAttachment>,
    effort: Option<AgentReasoningEffort>,
}

impl QueuedEditSnapshot {
    fn from_pending(pending: &PendingRun) -> Self {
        Self {
            context: pending.context.clone(),
            prompt: pending.prompt.clone(),
            options: pending.options.clone(),
        }
    }

    fn matches(&self, pending: &PendingRun) -> bool {
        self.context == pending.context
            && self.prompt == pending.prompt
            && self.options == pending.options
    }

    fn restore(&self, images: Vec<AgentAttachment>) -> Result<RestoredQueuedDraft, String> {
        Ok(RestoredQueuedDraft {
            prompt: self.prompt.clone(),
            images: queued_edit_image_context(&self.options.image_paths, images)?,
            effort: self.options.reasoning_effort,
        })
    }
}

/// Text context is already embedded in the queued prompt. Only reload images;
/// reattaching text would append that context a second time on the next submit.
fn queued_edit_image_context(
    paths: &[PathBuf],
    mut images: Vec<AgentAttachment>,
) -> Result<Vec<AgentAttachment>, String> {
    if paths.len() != images.len() || images.iter().any(|image| image.text.is_some()) {
        return Err(t!("agents.composer.queue_edit_changed").to_string());
    }
    for (image, original_path) in images.iter_mut().zip(paths) {
        // The shared loader canonicalizes for safe reading. Preserve the
        // selected target path and ordering from the approved queued input.
        image.path = original_path.clone();
    }
    Ok(images)
}

pub(super) struct AgentComposerUi {
    attachments: HashMap<Uuid, Vec<AgentAttachment>>,
    queues: HashMap<Uuid, VecDeque<PendingRun>>,
    pub(super) effort: HashMap<Uuid, AgentReasoningEffort>,
    run_options: HashMap<Uuid, AgentInputOptions>,
    pub(super) errors: HashMap<Uuid, String>,
    loading: HashSet<Uuid>,
    temporary_paths: Vec<PathBuf>,
    lightbox: Option<Entity<AttachmentLightbox>>,
    annotator: Option<Entity<AttachmentAnnotator>>,
}

impl AgentComposerUi {
    pub(super) fn new(_cx: &mut Context<AgentConsoleView>) -> Self {
        Self {
            attachments: HashMap::new(),
            queues: HashMap::new(),
            effort: HashMap::new(),
            run_options: HashMap::new(),
            errors: HashMap::new(),
            loading: HashSet::new(),
            temporary_paths: Vec::new(),
            lightbox: None,
            annotator: None,
        }
    }
}

impl Drop for AgentComposerUi {
    fn drop(&mut self) {
        for path in &self.temporary_paths {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl AgentConsoleView {
    pub(super) fn prepare_composer_prompt(
        &self,
        session_id: Uuid,
        prompt: &str,
        _cx: &Context<Self>,
    ) -> Result<String, String> {
        if self.composer_ui.loading.contains(&session_id) {
            return Err(t!("agents.composer.loading").to_string());
        }
        let mut expanded = prompt.to_string();
        for attachment in self
            .composer_ui
            .attachments
            .get(&session_id)
            .into_iter()
            .flatten()
        {
            if let Some(text) = &attachment.text {
                expanded.push_str("\n\n");
                expanded.push_str(&t!("agents.composer.context_data").to_string());
                expanded.push('\n');
                expanded.push_str(&serde_json::json!({"file": attachment.name, "content": shelldeck_core::agent_runtime::bounded_agent_context_text(text)}).to_string());
            }
        }
        if expanded.len() > MAX_AGENT_PROMPT_BYTES {
            return Err(t!("agents.error.prompt_too_long").to_string());
        }
        Ok(expanded)
    }

    pub(super) fn composer_input_options(
        &self,
        session_id: Uuid,
    ) -> Result<AgentInputOptions, String> {
        let image_paths: Vec<_> = self
            .composer_ui
            .attachments
            .get(&session_id)
            .into_iter()
            .flatten()
            .filter(|a| a.text.is_none())
            .map(|a| a.path.clone())
            .collect();
        if !image_paths.is_empty()
            && (!self.provider.capabilities().image_attachments
                || self.selected_connection.is_some())
        {
            return Err(t!("agents.composer.image_unsupported").to_string());
        }
        Ok(AgentInputOptions {
            image_paths,
            reasoning_effort: self
                .provider
                .capabilities()
                .reasoning_effort
                .then(|| self.composer_ui.effort.get(&session_id).copied())
                .flatten(),
        })
    }

    pub fn set_run_options(&mut self, run_id: Uuid, options: AgentInputOptions) {
        self.composer_ui.run_options.insert(run_id, options);
    }
    pub fn take_run_options(&mut self, run_id: Uuid) -> AgentInputOptions {
        self.composer_ui
            .run_options
            .remove(&run_id)
            .unwrap_or_default()
    }
    pub(super) fn clear_composer_context(&mut self, session_id: Uuid) {
        self.composer_ui.attachments.remove(&session_id);
        self.composer_ui.errors.remove(&session_id);
    }

    pub(super) fn queue_active_draft(&mut self, cx: &mut Context<Self>) -> bool {
        if !self
            .sessions
            .selected()
            .is_some_and(|session| session.status.is_active())
        {
            return false;
        }
        match self.pending_from_draft(cx) {
            Ok(mut pending) => {
                pending.source_draft = None;
                let session_id = pending.session_id;
                let queue = self.composer_ui.queues.entry(session_id).or_default();
                if queue.len() >= 8 {
                    self.composer_ui
                        .errors
                        .insert(session_id, t!("agents.composer.queue_full").to_string());
                } else {
                    queue.push_back(pending);
                    self.clear_composer_context(session_id);
                    if let Some(ui) = self.session_ui.get(&session_id) {
                        ui.prompt
                            .update(cx, |state, cx| state.replace_content("", cx));
                    }
                }
            }
            Err(error) => {
                if let Some(id) = self.sessions.selected_id() {
                    self.composer_ui.errors.insert(id, error);
                }
            }
        }
        cx.notify();
        true
    }

    pub fn drain_queued(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        if !self
            .sessions
            .get(session_id)
            .is_some_and(|s| s.status == AgentSessionStatus::Completed)
            || self.pending_confirm.is_some()
            || self.composer_ui.loading.contains(&session_id)
        {
            return;
        }
        let Some(pending) = self
            .composer_ui
            .queues
            .get_mut(&session_id)
            .and_then(VecDeque::pop_front)
        else {
            return;
        };
        let entity = cx.entity().downgrade();
        cx.defer(move |cx| {
            let _ = entity.update(cx, move |view, cx| {
                if pending.context.access == AgentAccessMode::ReadOnly {
                    view.start_pending(pending, cx);
                } else {
                    view.pending_confirm = Some(pending);
                    cx.notify();
                }
            });
        });
    }

    fn edit_queued_message(&mut self, session_id: Uuid, index: usize, cx: &mut Context<Self>) {
        let Some(ui) = self.session_ui.get(&session_id) else {
            return;
        };
        if self.composer_ui.loading.contains(&session_id)
            || !ui.prompt.read(cx).content().is_empty()
            || self
                .composer_ui
                .attachments
                .get(&session_id)
                .is_some_and(|items| !items.is_empty())
        {
            self.composer_ui.errors.insert(
                session_id,
                t!("agents.composer.queue_edit_draft_busy").to_string(),
            );
            cx.notify();
            return;
        }
        let Some(snapshot) = self
            .composer_ui
            .queues
            .get(&session_id)
            .and_then(|queue| queue.get(index))
            .map(QueuedEditSnapshot::from_pending)
        else {
            return;
        };
        if self.context_from_controls(cx).ok().as_ref() != Some(&snapshot.context) {
            self.composer_ui.errors.insert(
                session_id,
                t!("agents.composer.queue_edit_context_changed").to_string(),
            );
            cx.notify();
            return;
        }
        self.composer_ui.loading.insert(session_id);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let paths = snapshot.options.image_paths.clone();
            let loaded = cx
                .background_executor()
                .spawn(async move {
                    let loaded = load_agent_attachments(paths.clone(), None, true)?;
                    queued_edit_image_context(&paths, loaded)
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.composer_ui.loading.remove(&session_id);
                let result = loaded.and_then(|images| {
                    let restored = snapshot.restore(images)?;
                    let ui = view
                        .session_ui
                        .get(&session_id)
                        .ok_or_else(|| t!("agents.composer.queue_edit_changed").to_string())?;
                    if !ui.prompt.read(cx).content().is_empty()
                        || view
                            .composer_ui
                            .attachments
                            .get(&session_id)
                            .is_some_and(|items| !items.is_empty())
                    {
                        return Err(t!("agents.composer.queue_edit_draft_busy").to_string());
                    }
                    if view.sessions.selected_id() != Some(session_id)
                        || view.context_from_controls(cx).ok().as_ref() != Some(&snapshot.context)
                    {
                        return Err(t!("agents.composer.queue_edit_context_changed").to_string());
                    }
                    let queue = view
                        .composer_ui
                        .queues
                        .get_mut(&session_id)
                        .ok_or_else(|| t!("agents.composer.queue_edit_changed").to_string())?;
                    if !queue
                        .get(index)
                        .is_some_and(|pending| snapshot.matches(pending))
                    {
                        return Err(t!("agents.composer.queue_edit_changed").to_string());
                    }
                    // Commit only after every guard and every image load has
                    // succeeded; failures leave both queue and draft intact.
                    queue.remove(index);
                    view.composer_ui
                        .attachments
                        .insert(session_id, restored.images);
                    match restored.effort {
                        Some(effort) => {
                            view.composer_ui.effort.insert(session_id, effort);
                        }
                        None => {
                            view.composer_ui.effort.remove(&session_id);
                        }
                    }
                    ui.prompt
                        .update(cx, |state, cx| state.replace_content(restored.prompt, cx));
                    Ok(())
                });
                match result {
                    Ok(()) => {
                        view.composer_ui.errors.remove(&session_id);
                    }
                    Err(error) => {
                        view.composer_ui.errors.insert(session_id, error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn pick_agent_attachment(&mut self, scoped: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session_id) = self.sessions.selected_id() else {
            return;
        };
        let root = self
            .context_from_controls(cx)
            .ok()
            .filter(|context| matches!(context.target, AgentTarget::Local))
            .map(|context| PathBuf::from(context.workdir));
        if scoped && root.is_none() {
            self.composer_ui
                .errors
                .insert(session_id, t!("agents.composer.remote_mention").to_string());
            cx.notify();
            return;
        }
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(t!("ai.attachment.choose").to_string().into()),
            starting_directory: root.clone(),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let _ = this.update(cx, |view, cx| {
                view.import_agent_paths(session_id, paths, if scoped { root } else { None }, cx)
            });
        })
        .detach();
    }

    fn import_agent_paths(
        &mut self,
        session_id: Uuid,
        paths: Vec<PathBuf>,
        scope: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if self.composer_ui.loading.contains(&session_id) {
            return;
        }
        let remaining = MAX_ATTACHMENTS.saturating_sub(
            self.composer_ui
                .attachments
                .get(&session_id)
                .map_or(0, Vec::len),
        );
        if paths.len() > remaining {
            self.composer_ui.errors.insert(
                session_id,
                t!("agents.composer.attachment_limit").to_string(),
            );
            cx.notify();
            return;
        }
        let allow_images =
            self.provider.capabilities().image_attachments && self.selected_connection.is_none();
        self.composer_ui.loading.insert(session_id);
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move { load_agent_attachments(paths, scope, allow_images) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.composer_ui.loading.remove(&session_id);
                match loaded {
                    Ok(attachments) => {
                        view.composer_ui
                            .attachments
                            .entry(session_id)
                            .or_default()
                            .extend(attachments);
                        view.composer_ui.errors.remove(&session_id);
                    }
                    Err(error) => {
                        view.composer_ui.errors.insert(session_id, error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn paste_agent_context(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.sessions.selected_id() else {
            return;
        };
        if self.composer_ui.loading.contains(&session_id) {
            return;
        }
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        if self
            .composer_ui
            .attachments
            .get(&session_id)
            .map_or(0, Vec::len)
            >= MAX_ATTACHMENTS
        {
            self.composer_ui.errors.insert(
                session_id,
                t!("agents.composer.attachment_limit").to_string(),
            );
            cx.notify();
            return;
        }
        if let Some(image) = item.entries().iter().find_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(image.clone()),
            _ => None,
        }) {
            if self.provider != AgentProvider::Codex || self.selected_connection.is_some() {
                self.composer_ui.errors.insert(
                    session_id,
                    t!("agents.composer.image_unsupported").to_string(),
                );
                cx.notify();
                return;
            }
            match crate::issue_attachments::draft_from_clipboard_image(&image) {
                Ok(draft) => {
                    self.stage_agent_image(session_id, draft, cx);
                }
                Err(error) => {
                    self.composer_ui.errors.insert(session_id, error);
                }
            }
        } else if let Some(text) = item.text().filter(|text| !text.is_empty()) {
            if text.len() > MAX_TEXT_ATTACHMENT_BYTES as usize {
                self.composer_ui.errors.insert(
                    session_id,
                    t!("agents.composer.attachment_too_large").to_string(),
                );
            } else {
                self.composer_ui
                    .attachments
                    .entry(session_id)
                    .or_default()
                    .push(AgentAttachment {
                        path: PathBuf::new(),
                        name: t!("ai.attachment.clipboard_name").to_string(),
                        text: Some(shelldeck_core::agent_runtime::bounded_agent_context_text(
                            &text,
                        )),
                        preview: None,
                    });
            }
        }
        cx.notify();
    }

    pub(super) fn stage_agent_image(
        &mut self,
        session_id: Uuid,
        draft: AttachmentDraft,
        cx: &mut Context<Self>,
    ) {
        if self.composer_ui.loading.contains(&session_id) {
            self.composer_ui
                .errors
                .insert(session_id, t!("agents.composer.loading").to_string());
            cx.notify();
            return;
        }
        if draft.bytes.len() > AI_ATTACHMENT_IMAGE_MAX_BYTES
            || self
                .composer_ui
                .attachments
                .get(&session_id)
                .map_or(0, Vec::len)
                >= MAX_ATTACHMENTS
        {
            self.composer_ui.errors.insert(
                session_id,
                t!("agents.composer.attachment_too_large").to_string(),
            );
            cx.notify();
            return;
        }
        self.composer_ui.loading.insert(session_id);
        cx.spawn(async move |this, cx| {
            let saved = cx
                .background_executor()
                .spawn(async move {
                    use std::io::Write as _;
                    // NamedTempFile creates an exclusive private file, including
                    // mode 0600 on Unix. Never use a default-permission temp path.
                    let mut file = tempfile::Builder::new()
                        .prefix("shelldeck-agent-")
                        .suffix(".png")
                        .tempfile()
                        .map_err(|e| e.to_string())?;
                    file.write_all(draft.bytes.as_slice())
                        .map_err(|e| e.to_string())?;
                    let (_, path) = file.keep().map_err(|e| e.to_string())?;
                    Ok::<_, String>((path, draft.filename, draft.image))
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.composer_ui.loading.remove(&session_id);
                match saved {
                    Ok((path, name, preview)) => {
                        view.composer_ui.temporary_paths.push(path.clone());
                        view.composer_ui
                            .attachments
                            .entry(session_id)
                            .or_default()
                            .push(AgentAttachment {
                                path,
                                name,
                                text: None,
                                preview: Some(preview),
                            });
                    }
                    Err(error) => {
                        view.composer_ui.errors.insert(session_id, error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn capture_agent_context(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.sessions.selected_id() else {
            return;
        };
        if self.composer_ui.loading.contains(&id) {
            return;
        }
        self.composer_ui.loading.insert(id);
        cx.spawn(async move |this, cx| {
            let captured = cx
                .background_executor()
                .spawn(async { crate::issue_attachments::capture_region() })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.composer_ui.loading.remove(&id);
                match captured {
                    Ok(draft) => {
                        let cancel = cx.entity().downgrade();
                        let apply = cancel.clone();
                        view.composer_ui.annotator = Some(cx.new(|cx| {
                            AttachmentAnnotator::new(
                                draft,
                                move |cx| {
                                    if let Some(parent) = cancel.upgrade() {
                                        parent.update(cx, |view, cx| {
                                            view.composer_ui.annotator = None;
                                            cx.notify();
                                        });
                                    }
                                },
                                move |draft, cx| {
                                    if let Some(parent) = apply.upgrade() {
                                        parent.update(cx, |view, cx| {
                                            view.composer_ui.annotator = None;
                                            view.stage_agent_image(id, draft, cx);
                                        });
                                    }
                                },
                                cx,
                            )
                        }));
                    }
                    Err(error) => {
                        view.composer_ui.errors.insert(id, error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn preview_agent_attachment(&mut self, id: Uuid, index: usize, cx: &mut Context<Self>) {
        let images: Vec<_> = self
            .composer_ui
            .attachments
            .get(&id)
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(index, item)| {
                item.preview.clone().map(|image| {
                    (
                        index,
                        LightboxItem::from_image(
                            format!("agent-image-{index}"),
                            item.name.clone(),
                            image,
                        ),
                    )
                })
            })
            .collect();
        if images.is_empty() {
            return;
        }
        let selected = images
            .iter()
            .position(|(position, _)| *position == index)
            .unwrap_or_default();
        let parent = cx.entity().downgrade();
        self.composer_ui.lightbox = Some(cx.new(|cx| {
            AttachmentLightbox::new(
                images.into_iter().map(|(_, item)| item).collect(),
                selected,
                move |cx| {
                    if let Some(parent) = parent.upgrade() {
                        parent.update(cx, |view, cx| {
                            view.composer_ui.lightbox = None;
                            cx.notify();
                        });
                    }
                },
                cx,
            )
        }));
        cx.notify();
    }

    pub(super) fn render_composer_overlay(&self) -> Option<AnyElement> {
        if let Some(annotator) = &self.composer_ui.annotator {
            return Some(annotator.clone().into_any_element());
        }
        self.composer_ui
            .lightbox
            .as_ref()
            .map(|lightbox| lightbox.clone().into_any_element())
    }

    pub(super) fn render_composer(
        &mut self,
        session: &AgentSession,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = session.id;
        let prompt = self
            .session_ui
            .get(&id)
            .expect("session input exists")
            .prompt
            .clone();
        let parent = cx.entity();
        let mut composer = Composer::new(format!("agent-composer-{id}"), &prompt)
            .placeholder(
                t!(if session.status.is_active() {
                    "agents.composer.queue_placeholder"
                } else {
                    "agents.prompt.placeholder"
                })
                .to_string(),
            )
            .min_rows(2)
            .max_rows(8)
            .on_commit(move |cx| parent.update(cx, |view, cx| view.submit(cx)))
            .action(
                Button::new("agent-attach", "")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Icon)
                    .icon(IconSource::from("plus"))
                    .tooltip(t!("ai.attachment.choose").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.pick_agent_attachment(false, window, cx)
                    })),
            )
            .action(
                Button::new("agent-mention-file", "@")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .tooltip(t!("agents.composer.mention_file").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.pick_agent_attachment(true, window, cx)
                    })),
            )
            .action(
                Button::new("agent-paste-context", "")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Icon)
                    .icon(IconSource::from("copy"))
                    .tooltip(t!("agents.composer.paste").to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.paste_agent_context(cx))),
            );
        for (index, attachment) in self
            .composer_ui
            .attachments
            .get(&id)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let mut context = div().flex().items_center().gap(px(6.0));
            context = context.child(
                Button::new(("agent-context", index), attachment.name.clone())
                    .variant(ButtonVariant::Outline)
                    .size(ButtonSize::Sm)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.preview_agent_attachment(id, index, cx)
                    })),
            );
            context = context.child(
                Button::new(("agent-context-remove", index), "×")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(items) = this.composer_ui.attachments.get_mut(&id) {
                            if index < items.len() {
                                items.remove(index);
                            }
                        }
                        cx.notify();
                    })),
            );
            composer = composer.context(context);
        }
        if self.provider == AgentProvider::Codex && self.selected_connection.is_none() {
            composer = composer.action(
                Button::new("agent-capture", "")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Icon)
                    .icon(IconSource::from("scan"))
                    .tooltip(t!("agents.composer.capture").to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.capture_agent_context(cx))),
            );
        }
        if self.provider != AgentProvider::Automonique {
            composer = composer.option(self.render_model_popover(cx));
        }
        if let Some(run_id) = self.session_ui.get(&id).and_then(|ui| ui.run_id) {
            if session.status == AgentSessionStatus::Running
                && session.context.provider == AgentProvider::Codex
                && matches!(session.context.target, AgentTarget::Local)
                && session.question.is_none()
            {
                composer = composer.option(
                    Button::new("agent-steer", t!("agents.live.steer").to_string())
                        .variant(ButtonVariant::Secondary)
                        .size(ButtonSize::Sm)
                        .disabled(self.pending_live_inputs.contains_key(&run_id))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let prompt = this
                                .session_ui
                                .get(&id)
                                .unwrap()
                                .prompt
                                .read(cx)
                                .content()
                                .trim()
                                .to_string();
                            if prompt.is_empty() {
                                return;
                            }
                            if let Ok(options) = this.composer_input_options(id) {
                                if !options.image_paths.is_empty() {
                                    this.composer_ui
                                        .errors
                                        .insert(id, t!("agents.live.image_queue").to_string());
                                    cx.notify();
                                    return;
                                }
                            }
                            match this.prepare_composer_prompt(id, &prompt, cx) {
                                Ok(text) => {
                                    if !this.reserve_live_input(
                                        run_id,
                                        PendingLiveInput::steering(prompt, &text),
                                        cx,
                                    ) {
                                        return;
                                    }
                                    cx.emit(AgentConsoleEvent::Steer { run_id, text });
                                }
                                Err(error) => {
                                    this.composer_ui.errors.insert(id, error);
                                    cx.notify();
                                }
                            }
                        })),
                );
            }
            composer = composer.option(
                Button::new(("agent-stop", uuid_key(run_id)), "")
                    .variant(ButtonVariant::Destructive)
                    .size(ButtonSize::Icon)
                    .icon(IconSource::from("square"))
                    .tooltip(t!("agents.stop").to_string())
                    .on_click(cx.listener(move |this, _, _, cx| this.request_stop(run_id, cx))),
            );
        }
        let mut result = div().w_full().flex().flex_col().gap(px(6.0));
        if let Some(queue) = self.composer_ui.queues.get(&id) {
            for (index, pending) in queue.iter().enumerate() {
                result = result.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .text_size(px(11.0))
                                .child(format!(
                                    "{} · {}",
                                    t!("agents.composer.queued"),
                                    pending.prompt.lines().next().unwrap_or_default()
                                )),
                        )
                        .child(
                            Button::new(
                                ("agent-queue-edit", index),
                                t!("agents.composer.edit").to_string(),
                            )
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.edit_queued_message(id, index, cx),
                            )),
                        )
                        .child(
                            Button::new(("agent-queue-remove", index), "×")
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::Sm)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(queue) = this.composer_ui.queues.get_mut(&id) {
                                        queue.remove(index);
                                    }
                                    cx.notify();
                                })),
                        ),
                );
            }
        }
        if let Some(error) = self.composer_ui.errors.get(&id) {
            result = result.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::error())
                    .child(error.clone()),
            );
        }
        if self.composer_ui.loading.contains(&id) {
            result = result.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("agents.composer.loading").to_string()),
            );
        }
        if session
            .question
            .as_ref()
            .is_some_and(|q| q.response_supported)
        {
            composer = composer.footnote(
                div()
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("agents.live.answer_hint").to_string()),
            );
        } else if session.status.is_active() {
            composer = composer.footnote(
                div()
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("agents.composer.queue_hint").to_string()),
            );
        }
        result
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                this.import_agent_paths(id, paths.paths().to_vec(), None, cx)
            }))
            .child(composer)
            .into_any_element()
    }

    pub(super) fn render_message_actions(
        &self,
        message: &AgentMessage,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let copy = message.text.clone();
        let quote = message.text.clone();
        let retry = message.text.clone();
        let role = message.role;
        let mut row = div().flex().gap(px(6.0)).pt(px(4.0));
        row = row.child(
            Button::new(
                ("agent-copy-message", uuid_key(message.id)),
                t!("agents.composer.copy").to_string(),
            )
            .variant(ButtonVariant::Ghost)
            .size(ButtonSize::Sm)
            .on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
            }),
        );
        row = row.child(
            Button::new(
                ("agent-quote-message", uuid_key(message.id)),
                t!("agents.composer.quote").to_string(),
            )
            .variant(ButtonVariant::Ghost)
            .size(ButtonSize::Sm)
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(id) = this.sessions.selected_id() {
                    if let Some(ui) = this.session_ui.get(&id) {
                        let old = ui.prompt.read(cx).content().to_string();
                        let text = format!(
                            "{}\n\n{}",
                            quote
                                .lines()
                                .map(|line| format!("> {line}"))
                                .collect::<Vec<_>>()
                                .join("\n"),
                            old
                        );
                        ui.prompt
                            .update(cx, |state, cx| state.replace_content(text, cx));
                    }
                }
                cx.notify();
            })),
        );
        if role == AgentMessageRole::User {
            row = row.child(
                Button::new(
                    ("agent-edit-message", uuid_key(message.id)),
                    t!("agents.composer.edit").to_string(),
                )
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Sm)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(id) = this.sessions.selected_id() {
                        if let Some(ui) = this.session_ui.get(&id) {
                            ui.prompt
                                .update(cx, |state, cx| state.replace_content(retry.clone(), cx));
                        }
                    }
                    cx.notify();
                })),
            );
        }
        row.into_any_element()
    }
}

fn load_agent_attachments(
    paths: Vec<PathBuf>,
    scope: Option<PathBuf>,
    allow_images: bool,
) -> Result<Vec<AgentAttachment>, String> {
    use std::io::Read as _;
    let scope = scope
        .map(std::fs::canonicalize)
        .transpose()
        .map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    for path in paths {
        let path = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
        if scope.as_ref().is_some_and(|root| !path.starts_with(root)) {
            return Err(t!("agents.composer.outside_workspace").to_string());
        }
        let mut file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
        let meta = file.metadata().map_err(|e| e.to_string())?;
        if !meta.is_file() {
            return Err(t!("agents.composer.unsupported_file").to_string());
        }
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if meta.len() > MAX_IMAGE_BYTES {
            return Err(t!("agents.composer.attachment_too_large").to_string());
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_IMAGE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        let attachment = AiAttachment::from_bytes(name.clone(), bytes.clone())
            .map_err(|error| t!(error.message_key()).to_string())?;
        if attachment.is_image() {
            if !allow_images {
                return Err(t!("agents.composer.image_unsupported").to_string());
            }
            if meta.len() > MAX_IMAGE_BYTES {
                return Err(t!("agents.composer.attachment_too_large").to_string());
            }
            let draft = AttachmentDraft::from_bytes(name.clone(), bytes)?;
            result.push(AgentAttachment {
                path,
                name,
                text: None,
                preview: Some(draft.image),
            });
        } else {
            if meta.len() > MAX_TEXT_ATTACHMENT_BYTES {
                return Err(t!("agents.composer.attachment_too_large").to_string());
            }
            if attachment.payload.contains('\0') {
                return Err(t!("agents.composer.unsupported_file").to_string());
            }
            result.push(AgentAttachment {
                path,
                name,
                text: Some(shelldeck_core::agent_runtime::bounded_agent_context_text(
                    &attachment.payload,
                )),
                preview: None,
            });
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn queued_message() -> PendingRun {
        PendingRun {
            session_id: Uuid::new_v4(),
            context: AgentExecutionContext {
                provider: AgentProvider::Codex,
                target: AgentTarget::Local,
                access: AgentAccessMode::ReadOnly,
                workdir: std::env::temp_dir().display().to_string(),
                model: None,
            },
            prompt: "Inspect the screenshot\n\nAttached context:\n{\"file\":\"notes.txt\",\"content\":\"one context block\"}".into(),
            options: AgentInputOptions {
                image_paths: vec![PathBuf::from("selected-image.png")],
                reasoning_effort: Some(AgentReasoningEffort::High),
            },
            source_draft: None,
        }
    }

    fn loaded_image() -> AgentAttachment {
        AgentAttachment {
            path: PathBuf::from("canonical-image.png"),
            name: "screenshot.png".into(),
            text: None,
            preview: None,
        }
    }

    // SDTEST-2470 — SDUC-551
    #[test]
    fn sdtest_2470_queue_edit_preserves_images_effort_and_expanded_text_once() {
        let queued = queued_message();
        let restored = QueuedEditSnapshot::from_pending(&queued)
            .restore(vec![loaded_image()])
            .unwrap();
        assert_eq!(restored.prompt, queued.prompt);
        assert_eq!(restored.prompt.matches("one context block").count(), 1);
        assert!(restored.images.iter().all(|image| image.text.is_none()));
        assert_eq!(restored.images[0].path, queued.options.image_paths[0]);
        assert_eq!(restored.effort, Some(AgentReasoningEffort::High));

        let mut auto = queued;
        auto.options.reasoning_effort = None;
        assert_eq!(
            QueuedEditSnapshot::from_pending(&auto)
                .restore(vec![loaded_image()])
                .unwrap()
                .effort,
            None
        );
    }

    // SDTEST-2471 — SDUC-551
    #[test]
    fn sdtest_2471_queue_edit_rejects_missing_or_reclassified_images() {
        let snapshot = QueuedEditSnapshot::from_pending(&queued_message());
        assert!(snapshot.restore(Vec::new()).is_err());
        let mut changed = loaded_image();
        changed.text = Some("the selected image is now a text file".into());
        assert!(snapshot.restore(vec![changed]).is_err());
        assert!(snapshot
            .restore(vec![loaded_image(), loaded_image()])
            .is_err());
    }

    // SDTEST-2472 — SDUC-551
    #[test]
    fn sdtest_2472_queue_edit_detects_input_or_context_changes_before_removal() {
        let mut queued = queued_message();
        let snapshot = QueuedEditSnapshot::from_pending(&queued);
        assert!(snapshot.matches(&queued));
        queued.options.image_paths.clear();
        assert!(!snapshot.matches(&queued));
        queued.options = snapshot.options.clone();
        queued.options.reasoning_effort = None;
        assert!(!snapshot.matches(&queued));
        queued.options = snapshot.options.clone();
        queued.context.access = AgentAccessMode::FullAccess;
        assert!(!snapshot.matches(&queued));
        queued.context = snapshot.context.clone();
        queued.prompt.push_str("changed while loading");
        assert!(!snapshot.matches(&queued));
    }
}
