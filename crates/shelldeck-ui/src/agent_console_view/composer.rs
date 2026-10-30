//! Session-scoped input, explicit context intake and queued follow-up turns.
use super::*;
use crate::attachment_annotator::AttachmentAnnotator;
use crate::issue_attachments::{AttachmentDraft, AttachmentLightbox, LightboxItem};
use shelldeck_core::agent_runtime::{AgentInputOptions, AgentReasoningEffort};
use shelldeck_core::agent_session::storage::{
    AgentDraftAttachment, AgentDraftBundle, AgentQueuedDraft, AGENT_DRAFT_SCHEMA_VERSION,
    MAX_SAVED_AGENT_FOLLOWUPS,
};
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
    needs_review: bool,
    managed: bool,
    image_digest: Option<String>,
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
    paused_queues: HashMap<Uuid, VecDeque<AgentQueuedDraft>>,
    image_digests: HashMap<PathBuf, String>,
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
            paused_queues: HashMap::new(),
            image_digests: HashMap::new(),
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
    fn attachment_descriptor(item: &AgentAttachment) -> AgentDraftAttachment {
        AgentDraftAttachment {
            path: item.path.clone(),
            name: item.name.clone(),
            text: item.text.clone(),
            image_digest: item.image_digest.clone(),
            capture_image: item.text.is_none() && !item.needs_review && !item.managed,
        }
    }

    fn restored_attachment(item: &AgentDraftAttachment) -> AgentAttachment {
        AgentAttachment {
            path: item.path.clone(),
            name: item.name.clone(),
            text: item.text.clone(),
            preview: None,
            needs_review: item.text.is_none(),
            managed: true,
            image_digest: item.image_digest.clone(),
        }
    }

    pub(super) fn promote_saved_images(
        &mut self,
        report: &shelldeck_core::agent_session::storage::AgentSessionSaveReport,
    ) {
        for items in self.composer_ui.attachments.values_mut() {
            for item in items {
                if item.text.is_none() && !item.needs_review && !item.managed {
                    if report.unresolved_images.contains(&item.path) {
                        item.path = PathBuf::new();
                        item.needs_review = true;
                        item.managed = true;
                        item.preview = None;
                    } else if let Some(path) = report.captured_images.get(&item.path) {
                        item.path = path.clone();
                        item.managed = true;
                    }
                }
            }
        }
        for queue in self.composer_ui.queues.values_mut() {
            for pending in queue {
                for path in &mut pending.options.image_paths {
                    if !report.unresolved_images.contains(path) {
                        if let Some(captured) = report.captured_images.get(path) {
                            *path = captured.clone();
                        }
                    }
                }
            }
        }
        for (id, queue) in &mut self.composer_ui.queues {
            let mut keep = VecDeque::new();
            while let Some(pending) = queue.pop_front() {
                if pending
                    .options
                    .image_paths
                    .iter()
                    .any(|path| report.unresolved_images.contains(path))
                {
                    self.composer_ui
                        .paused_queues
                        .entry(*id)
                        .or_default()
                        .push_back(AgentQueuedDraft {
                            context: pending.context,
                            prompt: pending.prompt,
                            effort: pending.options.reasoning_effort,
                            attachments: pending
                                .options
                                .image_paths
                                .iter()
                                .map(|path| AgentDraftAttachment {
                                    path: if report.unresolved_images.contains(path) {
                                        PathBuf::new()
                                    } else {
                                        path.clone()
                                    },
                                    name: path
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .into(),
                                    text: None,
                                    image_digest: None,
                                    capture_image: false,
                                })
                                .collect(),
                        });
                } else {
                    keep.push_back(pending);
                }
            }
            *queue = keep;
        }
    }

    pub(super) fn saved_composer_bundle(
        &self,
        id: Uuid,
        context: AgentExecutionContext,
    ) -> AgentDraftBundle {
        let mut followups = self
            .composer_ui
            .paused_queues
            .get(&id)
            .into_iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>();
        followups.extend(
            self.composer_ui
                .queues
                .get(&id)
                .into_iter()
                .flatten()
                .map(|pending| AgentQueuedDraft {
                    context: pending.context.clone(),
                    prompt: pending.prompt.clone(),
                    effort: pending.options.reasoning_effort,
                    attachments: pending
                        .options
                        .image_paths
                        .iter()
                        .map(|path| AgentDraftAttachment {
                            path: path.clone(),
                            name: path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into(),
                            text: None,
                            image_digest: self.composer_ui.image_digests.get(path).cloned(),
                            capture_image: !self.draft_store().owns_draft_image_path(path),
                        })
                        .collect(),
                }),
        );
        AgentDraftBundle {
            version: AGENT_DRAFT_SCHEMA_VERSION,
            context,
            attachments: self
                .composer_ui
                .attachments
                .get(&id)
                .into_iter()
                .flatten()
                .map(Self::attachment_descriptor)
                .collect(),
            effort: self.composer_ui.effort.get(&id).copied(),
            followups,
        }
    }

    pub(super) fn restore_composer_bundle(&mut self, id: Uuid, bundle: &AgentDraftBundle) {
        self.composer_ui.attachments.insert(
            id,
            bundle
                .attachments
                .iter()
                .map(Self::restored_attachment)
                .collect(),
        );
        if let Some(effort) = bundle.effort {
            self.composer_ui.effort.insert(id, effort);
        }
        // Deliberately never put saved items back in the live completion-triggered queue.
        self.composer_ui
            .paused_queues
            .insert(id, bundle.followups.clone().into());
    }

    fn has_unreviewed_attachments(&self, id: Uuid) -> bool {
        self.composer_ui
            .attachments
            .get(&id)
            .is_some_and(|items| items.iter().any(|item| item.needs_review))
    }

    fn review_saved_followup(&mut self, id: Uuid, index: usize, cx: &mut Context<Self>) {
        if self.sessions.selected_id() != Some(id)
            || self.composer_context_loading(id)
            || self
                .sessions
                .get(id)
                .is_none_or(|session| session.status.is_active())
        {
            return;
        }
        if self.has_unsent_draft(id, cx) {
            self.composer_ui
                .errors
                .insert(id, t!("agents.composer.queue_edit_draft_busy").to_string());
            cx.notify();
            return;
        }
        let Some(saved) = self
            .composer_ui
            .paused_queues
            .get(&id)
            .and_then(|queue| queue.get(index))
            .cloned()
        else {
            return;
        };
        if self.context_from_controls(cx).ok().as_ref() != Some(&saved.context) {
            self.composer_ui
                .errors
                .insert(id, t!("agents.composer.saved_context_changed").to_string());
            cx.notify();
            return;
        }
        self.composer_ui.attachments.insert(
            id,
            saved
                .attachments
                .iter()
                .map(Self::restored_attachment)
                .collect(),
        );
        match saved.effort {
            Some(effort) => {
                self.composer_ui.effort.insert(id, effort);
            }
            None => {
                self.composer_ui.effort.remove(&id);
            }
        }
        self.session_ui[&id]
            .prompt
            .update(cx, |state, cx| state.replace_content(saved.prompt, cx));
        self.composer_ui
            .paused_queues
            .get_mut(&id)
            .unwrap()
            .remove(index);
        self.composer_ui.errors.remove(&id);
        self.schedule_library_save(cx);
        cx.notify();
    }

    fn review_saved_attachments(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.composer_context_loading(id) || self.sessions.selected_id() != Some(id) {
            return;
        }
        let Some(context) = self.attachment_context(id, cx) else {
            return;
        };
        let captured = self
            .composer_ui
            .attachments
            .get(&id)
            .cloned()
            .unwrap_or_default();
        if !captured.iter().any(|item| item.needs_review) {
            return;
        }
        let descriptors = captured
            .iter()
            .map(Self::attachment_descriptor)
            .collect::<Vec<_>>();
        let store = self.draft_store();
        self.composer_ui.loading.insert(id);
        cx.spawn(async move |this, cx| {
            let restored = cx
                .background_executor()
                .spawn(async move {
                    let mut restored = captured;
                    for item in &mut restored {
                        if item.needs_review {
                            let bytes = store
                                .read_draft_image(&item.path)
                                .map_err(|error| format!("{}: {error}", item.name))?;
                            let draft = AttachmentDraft::from_bytes(item.name.clone(), bytes)?;
                            item.preview = Some(draft.image);
                            item.needs_review = false;
                        }
                    }
                    Ok::<_, String>(restored)
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.composer_ui.loading.remove(&id);
                if !view.admit_attachment_context(id, &context, cx)
                    || view
                        .composer_ui
                        .attachments
                        .get(&id)
                        .map(|items| {
                            items
                                .iter()
                                .map(Self::attachment_descriptor)
                                .collect::<Vec<_>>()
                        })
                        .as_ref()
                        != Some(&descriptors)
                {
                    return;
                }
                match restored {
                    Ok(items) => {
                        view.composer_ui.attachments.insert(id, items);
                        view.composer_ui.errors.remove(&id);
                    }
                    Err(error) => {
                        view.composer_ui.errors.insert(
                            id,
                            format!("{} {error}", t!("agents.composer.saved_attachment_help")),
                        );
                    }
                }
                view.schedule_library_save(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn has_unsent_draft(&self, session_id: Uuid, cx: &App) -> bool {
        self.session_ui
            .get(&session_id)
            .is_some_and(|ui| !ui.prompt.read(cx).content().trim().is_empty())
            || self
                .composer_ui
                .attachments
                .get(&session_id)
                .is_some_and(|items| !items.is_empty())
    }

    pub(super) fn composer_context_loading(&self, id: Uuid) -> bool {
        self.composer_ui.loading.contains(&id)
    }

    /// Copy user-authored draft context only; queues and run authority stay on the source.
    pub(super) fn copy_composer_draft_context(&mut self, source: Uuid, target: Uuid) {
        if let Some(items) = self.composer_ui.attachments.get(&source).cloned() {
            self.composer_ui.attachments.insert(target, items);
        }
        if let Some(effort) = self.composer_ui.effort.get(&source).copied() {
            self.composer_ui.effort.insert(target, effort);
        }
    }

    pub(super) fn prepare_composer_prompt(
        &self,
        session_id: Uuid,
        prompt: &str,
        _cx: &Context<Self>,
    ) -> Result<String, String> {
        if self.composer_ui.loading.contains(&session_id) {
            return Err(t!("agents.composer.loading").to_string());
        }
        if self.has_unreviewed_attachments(session_id) {
            return Err(t!("agents.composer.saved_attachment_help").to_string());
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
                expanded.push_str(t!("agents.composer.context_data").as_ref());
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
        if self.has_unreviewed_attachments(session_id) {
            return Err(t!("agents.composer.saved_attachment_help").to_string());
        }
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
            mcp_servers: Vec::new(),
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
                if let Some(items) = self.composer_ui.attachments.get(&session_id) {
                    for item in items {
                        if let Some(digest) = &item.image_digest {
                            self.composer_ui
                                .image_digests
                                .entry(item.path.clone())
                                .or_insert_with(|| digest.clone());
                        }
                    }
                }
                let paused_count = self
                    .composer_ui
                    .paused_queues
                    .get(&session_id)
                    .map_or(0, VecDeque::len);
                let queue = self.composer_ui.queues.entry(session_id).or_default();
                if queue.len() + paused_count >= MAX_SAVED_AGENT_FOLLOWUPS {
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
        self.schedule_library_save(cx);
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
            .get(&session_id)
            .and_then(|queue| queue.front())
        else {
            return;
        };
        let captured = QueuedEditSnapshot::from_pending(pending);
        let entity = cx.entity().downgrade();
        cx.defer(move |cx| {
            let _ = entity.update(cx, move |view, cx| {
                view.admit_queued_message(session_id, &captured, cx)
            });
        });
    }

    fn admit_queued_message(
        &mut self,
        session_id: Uuid,
        captured: &QueuedEditSnapshot,
        cx: &mut Context<Self>,
    ) {
        // Completion can race navigation, another turn, or queue edits.
        // Retain the original queued input until admission succeeds.
        if !self
            .sessions
            .get(session_id)
            .is_some_and(|session| session.status == AgentSessionStatus::Completed)
            || self.pending_confirm.is_some()
            || self.composer_ui.loading.contains(&session_id)
            || !self
                .composer_ui
                .queues
                .get(&session_id)
                .and_then(|queue| queue.front())
                .is_some_and(|front| captured.matches(front))
        {
            return;
        }
        if self.attachment_context(session_id, cx).as_ref() != Some(&captured.context) {
            self.composer_ui.errors.insert(
                session_id,
                t!("agents.composer.queue_edit_context_changed").to_string(),
            );
            cx.notify();
            return;
        }
        let pending = PendingRun {
            session_id,
            context: captured.context.clone(),
            prompt: captured.prompt.clone(),
            options: captured.options.clone(),
            source_draft: None,
        };
        if pending.context.access == AgentAccessMode::ReadOnly {
            self.start_pending(pending, cx);
            if !self
                .sessions
                .get(session_id)
                .is_some_and(|session| session.status.is_active())
            {
                return;
            }
        } else {
            self.pending_confirm = Some(pending);
        }
        cx.notify();
    }

    /// Called only after the exact queued request passes UI admission. Until
    /// then the queue owns its text/options, including while confirmation is open.
    pub(super) fn consume_admitted_queued(&mut self, request: &AgentRunRequest) {
        let Some(session_id) = self.run_sessions.get(&request.id).copied() else {
            return;
        };
        let Some(options) = self.composer_ui.run_options.get(&request.id) else {
            return;
        };
        let Some(queue) = self.composer_ui.queues.get_mut(&session_id) else {
            return;
        };
        if queue.front().is_some_and(|pending| {
            pending.context == AgentExecutionContext::from_request(request)
                && pending.prompt == request.prompt
                && pending.options == *options
        }) {
            queue.pop_front();
        }
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

    fn attachment_context(
        &self,
        session_id: Uuid,
        cx: &Context<Self>,
    ) -> Option<AgentExecutionContext> {
        let session = self.sessions.get(session_id)?;
        Some(
            if self.sessions.selected_id() == Some(session_id) && !session.status.is_active() {
                self.displayed_context(cx)
            } else {
                session.context.clone()
            },
        )
    }

    fn admit_attachment_context(
        &mut self,
        session_id: Uuid,
        expected: &AgentExecutionContext,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.attachment_context(session_id, cx).as_ref() == Some(expected) {
            return true;
        }
        if self.sessions.get(session_id).is_some() {
            self.composer_ui.errors.insert(
                session_id,
                t!("agents.composer.attachment_context_changed").to_string(),
            );
            cx.notify();
        }
        false
    }

    pub(super) fn pick_agent_attachment(
        &mut self,
        scoped: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session_id) = self.sessions.selected_id() else {
            return;
        };
        let Some(expected) = self.attachment_context(session_id, cx) else {
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
                view.import_agent_paths_at_context(
                    session_id,
                    paths,
                    if scoped { root } else { None },
                    expected,
                    cx,
                )
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
        let Some(expected) = self.attachment_context(session_id, cx) else {
            return;
        };
        self.import_agent_paths_at_context(session_id, paths, scope, expected, cx);
    }

    fn import_agent_paths_at_context(
        &mut self,
        session_id: Uuid,
        paths: Vec<PathBuf>,
        scope: Option<PathBuf>,
        expected: AgentExecutionContext,
        cx: &mut Context<Self>,
    ) {
        if !self.admit_attachment_context(session_id, &expected, cx) {
            return;
        }
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
        let allow_images = expected.provider.capabilities().image_attachments
            && expected.target == AgentTarget::Local;
        self.composer_ui.loading.insert(session_id);
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move { load_agent_attachments(paths, scope, allow_images) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.composer_ui.loading.remove(&session_id);
                if !view.admit_attachment_context(session_id, &expected, cx) {
                    return;
                }
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
                view.schedule_library_save(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn paste_agent_context(&mut self, cx: &mut Context<Self>) {
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
                        needs_review: false,
                        managed: false,
                        image_digest: None,
                    });
            }
        }
        self.schedule_library_save(cx);
        cx.notify();
    }

    pub(super) fn stage_agent_image(
        &mut self,
        session_id: Uuid,
        draft: AttachmentDraft,
        cx: &mut Context<Self>,
    ) {
        let Some(expected) = self.attachment_context(session_id, cx) else {
            return;
        };
        if !expected.provider.capabilities().image_attachments
            || expected.target != AgentTarget::Local
        {
            self.composer_ui.errors.insert(
                session_id,
                t!("agents.composer.image_unsupported").to_string(),
            );
            cx.notify();
            return;
        }
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
                    let digest = shelldeck_core::agent_session::storage::draft_image_digest(
                        draft.bytes.as_slice(),
                    );
                    Ok::<_, String>((path, draft.filename, draft.image, digest))
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.composer_ui.loading.remove(&session_id);
                if !view.admit_attachment_context(session_id, &expected, cx) {
                    if let Ok((path, _, _, _)) = saved {
                        let _ = std::fs::remove_file(path);
                    }
                    return;
                }
                match saved {
                    Ok((path, name, preview, digest)) => {
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
                                needs_review: false,
                                managed: false,
                                image_digest: Some(digest),
                            });
                    }
                    Err(error) => {
                        view.composer_ui.errors.insert(session_id, error);
                    }
                }
                view.schedule_library_save(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn capture_agent_context(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.sessions.selected_id() else {
            return;
        };
        let Some(expected) = self.attachment_context(id, cx) else {
            return;
        };
        if !expected.provider.capabilities().image_attachments
            || expected.target != AgentTarget::Local
        {
            self.composer_ui
                .errors
                .insert(id, t!("agents.composer.image_unsupported").to_string());
            cx.notify();
            return;
        }
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
                if !view.admit_attachment_context(id, &expected, cx) {
                    return;
                }
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
                                            if view.admit_attachment_context(id, &expected, cx) {
                                                view.stage_agent_image(id, draft, cx);
                                            }
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
        let missing_target =
            validate_agent_target(&self.displayed_context(cx).target, &self.connections).is_err();
        // An answer goes to an already running process, rather than starting a
        // new connection. Keep that route available if its saved profile vanished.
        let answering = session
            .question
            .as_ref()
            .is_some_and(|q| q.response_supported)
            && self
                .session_ui
                .get(&id)
                .is_some_and(|ui| ui.run_id.is_some());
        let text = prompt.read(cx).content();
        let ready = !self.has_unreviewed_attachments(id)
            && self
                .validate_native_context_transition(id, &self.displayed_context(cx))
                .is_ok()
            && !self
                .native_history
                .get(&id)
                .is_some_and(|state| state.fresh_view)
            && !text.trim().is_empty()
            && !self.composer_ui.loading.contains(&id)
            && ((!missing_target
                && !Self::native_context_needs_reconnect(session, &self.displayed_context(cx)))
                || answering);
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
            .commit_enabled(ready)
            .on_commit(move |cx| parent.update(cx, |view, cx| view.submit(cx)))
            .action(
                self.render_composer_menu(super::composer_controls::ComposerMenu::Attachments, cx),
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
                        this.schedule_library_save(cx);
                        cx.notify();
                    })),
            );
            composer = composer.context(context);
        }
        // Keep every execution choice beside the message, in a wrapping slot
        // that cannot displace the shared Composer's Send/Stop controls.
        let mut options = div()
            .flex()
            .flex_wrap()
            .w_full()
            .min_w_0()
            .items_center()
            .gap(px(4.0))
            .child(self.render_composer_menu(super::composer_controls::ComposerMenu::Agent, cx))
            .child(self.render_composer_menu(super::composer_controls::ComposerMenu::Access, cx))
            .child(div().flex_1().min_w_0());
        if self.provider != AgentProvider::Automonique {
            options = options.child(self.render_model_popover(cx));
        }
        composer = composer.option(options);
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
        if let Some(change) = self.render_native_context_change(session, cx) {
            result = result.child(change);
        }
        if self.has_unreviewed_attachments(id) {
            result =
                result.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .whitespace_normal()
                                .child(t!("agents.composer.saved_attachment_help").to_string()),
                        )
                        .child(
                            Button::new(
                                "check-saved-attachments",
                                t!("agents.composer.check_saved_attachments").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .disabled(self.composer_context_loading(id))
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.review_saved_attachments(id, cx),
                            )),
                        ),
                );
        }
        let has_queued = self
            .composer_ui
            .paused_queues
            .get(&id)
            .is_some_and(|queue| !queue.is_empty())
            || self
                .composer_ui
                .queues
                .get(&id)
                .is_some_and(|queue| !queue.is_empty());
        // Queue controls have their own bounded list; the transcript remains the
        // sole conversation scroll owner and the composer keeps its usable height.
        let mut queued_controls = div()
            .id(("agent-queued-controls", uuid_key(id)))
            .w_full()
            .min_w_0()
            .max_h(px(120.0))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(6.0));
        if let Some(queue) = self.composer_ui.paused_queues.get(&id) {
            if !queue.is_empty() {
                queued_controls = queued_controls.child(
                    div()
                        .text_size(px(11.0))
                        .whitespace_normal()
                        .child(t!("agents.composer.saved_followups_paused").to_string()),
                );
            }
            for (index, saved) in queue.iter().enumerate() {
                queued_controls = queued_controls.child(
                    div()
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .text_size(px(11.0))
                                .child(saved.prompt.lines().next().unwrap_or_default().to_string()),
                        )
                        .child(
                            Button::new(("copy-saved-followup", index), "")
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::Icon)
                                .icon(IconSource::from("copy"))
                                .tooltip(t!("agents.composer.copy").to_string())
                                .on_click({
                                    let text = saved.prompt.clone();
                                    move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            text.clone(),
                                        ))
                                    }
                                }),
                        )
                        .child(
                            Button::new(
                                ("review-saved-followup", index),
                                t!("agents.composer.review_saved_followup").to_string(),
                            )
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .disabled(
                                session.status.is_active() || self.composer_context_loading(id),
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.review_saved_followup(id, index, cx),
                            )),
                        )
                        .child(
                            Button::new(("remove-saved-followup", index), "")
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::Icon)
                                .icon(IconSource::from("x"))
                                .tooltip(t!("agents.composer.remove_followup").to_string())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(queue) = this.composer_ui.paused_queues.get_mut(&id)
                                    {
                                        queue.remove(index);
                                    }
                                    this.schedule_library_save(cx);
                                    cx.notify();
                                })),
                        ),
                );
            }
        }
        if let Some(queue) = self.composer_ui.queues.get(&id) {
            for (index, pending) in queue.iter().enumerate() {
                queued_controls = queued_controls.child(
                    div()
                        .flex_shrink_0()
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
                            Button::new(("agent-queue-remove", index), "")
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::Icon)
                                .icon(IconSource::from("x"))
                                .tooltip(t!("agents.composer.remove_followup").to_string())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(queue) = this.composer_ui.queues.get_mut(&id) {
                                        queue.remove(index);
                                    }
                                    this.schedule_library_save(cx);
                                    cx.notify();
                                })),
                        ),
                );
            }
        }
        if has_queued {
            result = result.child(queued_controls);
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
        if missing_target && !answering {
            composer = composer.footnote(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::warning())
                    .child(t!("agents.composer.reconnect_hint").to_string()),
            );
        } else if session
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
            .child(self.render_project_picker(cx))
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
        let mut row = div().flex().gap(px(2.0)).pt(px(4.0));
        row = row.child(
            Button::new(("agent-copy-message", uuid_key(message.id)), "")
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Icon)
                .w(px(26.0))
                .h(px(26.0))
                .icon(IconSource::from("copy"))
                .tooltip(t!("agents.composer.copy").to_string())
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                }),
        );
        row = row.child(
            Button::new(("agent-quote-message", uuid_key(message.id)), "")
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Icon)
                .w(px(26.0))
                .h(px(26.0))
                .icon(IconSource::from("reply"))
                .tooltip(t!("agents.composer.quote").to_string())
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
                Button::new(("agent-edit-message", uuid_key(message.id)), "")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Icon)
                    .w(px(26.0))
                    .h(px(26.0))
                    .icon(IconSource::from("pencil"))
                    .tooltip(t!("agents.composer.edit").to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(id) = this.sessions.selected_id() {
                            if let Some(ui) = this.session_ui.get(&id) {
                                ui.prompt.update(cx, |state, cx| {
                                    state.replace_content(retry.clone(), cx)
                                });
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
            let image_digest = Some(shelldeck_core::agent_session::storage::draft_image_digest(
                &bytes,
            ));
            let draft = AttachmentDraft::from_bytes(name.clone(), bytes)?;
            result.push(AgentAttachment {
                path,
                name,
                text: None,
                preview: Some(draft.image),
                needs_review: false,
                managed: false,
                image_digest,
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
                needs_review: false,
                managed: false,
                image_digest: None,
            });
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    // SDTEST-2838 / SDUC-564 — saved queues never drain automatically; recovery
    // preserves current drafts, blocks unresolved images, and retains exact context.
    #[test]
    fn saved_followups_require_review_and_missing_images_block_send() {
        let mut app = gpui::TestAppContext::single();
        let entity = app.new(AgentConsoleView::new);
        entity.update(&mut app, |view, cx| {
            view.library = AgentLibraryUi::new(cx);
            let id = view.sessions.selected_id().unwrap();
            let context = view.sessions.get(id).unwrap().context.clone();
            let saved = AgentQueuedDraft {
                context: context.clone(),
                prompt: "Saved follow-up".into(),
                effort: Some(AgentReasoningEffort::High),
                attachments: vec![AgentDraftAttachment {
                    path: PathBuf::from("/missing/saved.image"),
                    name: "Screenshot.png".into(),
                    text: None,
                    image_digest: None,
                    capture_image: false,
                }],
            };
            let bundle = AgentDraftBundle {
                version: AGENT_DRAFT_SCHEMA_VERSION,
                context: context.clone(),
                attachments: vec![],
                effort: None,
                followups: vec![saved],
            };
            view.restore_composer_bundle(id, &bundle);
            view.sessions.get_mut(id).unwrap().status = AgentSessionStatus::Completed;
            view.drain_queued(id, cx);
            assert!(view.run_sessions.is_empty());
            assert!(view
                .composer_ui
                .queues
                .get(&id)
                .is_none_or(VecDeque::is_empty));
            assert_eq!(view.composer_ui.paused_queues[&id].len(), 1);
            view.session_ui[&id]
                .prompt
                .update(cx, |state, cx| state.replace_content("Keep new typing", cx));
            view.review_saved_followup(id, 0, cx);
            assert_eq!(
                view.session_ui[&id].prompt.read(cx).content(),
                "Keep new typing"
            );
            assert_eq!(view.composer_ui.paused_queues[&id].len(), 1);
            view.session_ui[&id]
                .prompt
                .update(cx, |state, cx| state.reset(cx));
            view.change_draft_access(AgentAccessMode::FullAccess, cx);
            view.review_saved_followup(id, 0, cx);
            assert_eq!(view.composer_ui.paused_queues[&id].len(), 1);
            view.apply_selected_context(cx);
            view.review_saved_followup(id, 0, cx);
            assert!(view.composer_ui.paused_queues[&id].is_empty());
            assert_eq!(
                view.session_ui[&id].prompt.read(cx).content(),
                "Saved follow-up"
            );
            assert!(view
                .prepare_composer_prompt(id, "Saved follow-up", cx)
                .is_err());
            assert!(view.composer_input_options(id).is_err());
            assert!(view.run_sessions.is_empty());
            // Remove is an explicit choice to send without the unavailable image.
            view.composer_ui.attachments.remove(&id);
            assert!(view
                .prepare_composer_prompt(id, "Saved follow-up", cx)
                .is_ok());
            // Capture failure also pauses live followups rather than silently dropping images.
            let path = PathBuf::from("/missing/live.png");
            view.composer_ui
                .queues
                .entry(id)
                .or_default()
                .push_back(PendingRun {
                    session_id: id,
                    context,
                    prompt: "Keep queued image".into(),
                    source_draft: None,
                    options: AgentInputOptions {
                        image_paths: vec![path.clone()],
                        ..Default::default()
                    },
                });
            let mut report =
                shelldeck_core::agent_session::storage::AgentSessionSaveReport::default();
            report.unresolved_images.insert(path);
            view.promote_saved_images(&report);
            assert!(view.composer_ui.queues[&id].is_empty());
            assert_eq!(
                view.composer_ui.paused_queues[&id][0].prompt,
                "Keep queued image"
            );
            assert!(view.run_sessions.is_empty());
        });
    }

    // SDTEST-2841 / SDUC-564 — reusing a path for a different image must not
    // rewrite an earlier queued approval; both drafts remain reviewable.
    #[test]
    fn queued_image_digest_preserves_first_approval_for_reused_path() {
        use shelldeck_core::agent_session::storage::{
            draft_image_digest, AgentSessionSnapshot, AgentSessionStore,
        };
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("changing.png");
        std::fs::write(&path, b"image B").unwrap();
        let store = AgentSessionStore::new(temporary.path().join("sessions.json"));
        let mut app = gpui::TestAppContext::single();
        let entity = app.new(AgentConsoleView::new);
        entity.update(&mut app, |view, cx| {
            view.library = AgentLibraryUi::new(cx);
            let mut context = view.sessions.selected().unwrap().context.clone();
            context.provider = AgentProvider::Codex;
            let id = view
                .create_session("Two image follow-ups", context.clone(), cx)
                .unwrap();
            view.sessions.get_mut(id).unwrap().status = AgentSessionStatus::Running;
            for (prompt, bytes) in [("First image", b"image A"), ("Second image", b"image B")] {
                view.composer_ui.attachments.insert(
                    id,
                    vec![AgentAttachment {
                        path: path.clone(),
                        name: "changing.png".into(),
                        text: None,
                        preview: None,
                        needs_review: false,
                        managed: false,
                        image_digest: Some(draft_image_digest(bytes)),
                    }],
                );
                view.session_ui[&id]
                    .prompt
                    .update(cx, |state, cx| state.replace_content(prompt, cx));
                assert!(view.queue_active_draft(cx));
            }
            assert_eq!(view.composer_ui.queues[&id].len(), 2);
            assert_eq!(
                view.composer_ui.image_digests[&path],
                draft_image_digest(b"image A")
            );
            let mut snapshot = AgentSessionSnapshot {
                collection: view.sessions.clone(),
                ..Default::default()
            };
            snapshot
                .draft_bundles
                .insert(id, view.saved_composer_bundle(id, context));
            let report = store.save_with_drafts(&snapshot).unwrap();
            assert!(report.unresolved_images.contains(&path));
            assert!(report.captured_images.is_empty());
            view.promote_saved_images(&report);
            assert!(view.composer_ui.queues[&id].is_empty());
            assert_eq!(
                view.composer_ui.paused_queues[&id]
                    .iter()
                    .map(|draft| draft.prompt.as_str())
                    .collect::<Vec<_>>(),
                ["First image", "Second image"]
            );
            assert!(view.run_sessions.is_empty());
        });
    }

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
                ..Default::default()
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
            needs_review: false,
            managed: false,
            image_digest: None,
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

    // SDTEST-2827 / SDUC-564
    #[test]
    fn attachment_intake_keeps_original_session_and_rejects_changed_context() {
        let mut app = gpui::TestAppContext::single();
        let view = app.update(|cx| cx.new(AgentConsoleView::new));
        view.update(&mut app, |view, cx| {
            let first = view.sessions.selected_id().unwrap();
            let expected = view.attachment_context(first, cx).unwrap();
            let second = view.create_session("Other", expected.clone(), cx).unwrap();
            // A delayed file chooser result still belongs to the old draft.
            assert!(view.admit_attachment_context(first, &expected, cx));
            assert_eq!(view.sessions.selected_id(), Some(second));
            let mut changed = expected.clone();
            changed.provider = if changed.provider == AgentProvider::Codex {
                AgentProvider::Claude
            } else {
                AgentProvider::Codex
            };
            view.sessions
                .get_mut(first)
                .unwrap()
                .set_context(changed, 1)
                .unwrap();
            assert!(!view.admit_attachment_context(first, &expected, cx));
            assert!(view.composer_ui.errors.contains_key(&first));
            assert!(!view.composer_ui.errors.contains_key(&second));
            view.sessions.remove(first).unwrap();
            view.composer_ui.errors.remove(&first);
            assert!(!view.admit_attachment_context(first, &expected, cx));
            assert!(!view.composer_ui.errors.contains_key(&first));
        });
    }

    // SDTEST-2828 / SDUC-564
    #[test]
    fn queued_followup_survives_admission_failure_and_does_not_duplicate_dispatch() {
        let mut app = gpui::TestAppContext::single();
        let view = app.update(|cx| cx.new(AgentConsoleView::new));
        view.update(&mut app, |view, cx| {
            let connection_id = Uuid::new_v4();
            let context = AgentExecutionContext {
                provider: AgentProvider::Codex,
                target: AgentTarget::Ssh {
                    connection_id,
                    connection_label: "remote".into(),
                },
                access: AgentAccessMode::ReadOnly,
                workdir: "/srv/project".into(),
                model: None,
            };
            let session_id = view.create_session("Queued", context.clone(), cx).unwrap();
            view.sessions.get_mut(session_id).unwrap().status = AgentSessionStatus::Completed;
            let queued = PendingRun {
                session_id,
                context: context.clone(),
                prompt: "Preserve this follow-up".into(),
                options: AgentInputOptions::default(),
                source_draft: None,
            };
            let snapshot = QueuedEditSnapshot::from_pending(&queued);
            view.composer_ui
                .queues
                .entry(session_id)
                .or_default()
                .push_back(queued);
            // No saved SSH target: admission must not eat the queued text/options.
            view.admit_queued_message(session_id, &snapshot, cx);
            assert!(view.composer_ui.queues[&session_id]
                .front()
                .is_some_and(|pending| snapshot.matches(pending)));
            assert!(view.run_sessions.is_empty());
            view.set_connections(
                vec![AgentConnectionOption {
                    id: connection_id,
                    label: "remote".into(),
                    host: "fixture.invalid".into(),
                }],
                cx,
            );
            view.apply_selected_context(cx);
            view.admit_queued_message(session_id, &snapshot, cx);
            assert!(view.composer_ui.queues[&session_id].is_empty());
            assert_eq!(view.run_sessions.len(), 1);
            view.admit_queued_message(session_id, &snapshot, cx);
            assert_eq!(view.run_sessions.len(), 1);
            assert_eq!(view.sessions.get(session_id).unwrap().messages.len(), 1);

            // Write-enabled follow-ups remain queued through confirmation too.
            view.sessions
                .finish(session_id, AgentSessionStatus::Completed, 4)
                .unwrap();
            let writable = AgentExecutionContext {
                access: AgentAccessMode::WorkspaceWrite,
                ..context
            };
            view.sessions
                .get_mut(session_id)
                .unwrap()
                .set_context(writable.clone(), 5)
                .unwrap();
            view.apply_selected_context(cx);
            let queued = PendingRun {
                session_id,
                context: writable,
                prompt: "Confirmed change".into(),
                options: AgentInputOptions::default(),
                source_draft: None,
            };
            let snapshot = QueuedEditSnapshot::from_pending(&queued);
            view.composer_ui
                .queues
                .get_mut(&session_id)
                .unwrap()
                .push_back(queued);
            view.admit_queued_message(session_id, &snapshot, cx);
            assert!(view.pending_confirm.is_some());
            assert_eq!(view.composer_ui.queues[&session_id].len(), 1);
            view.set_connections(Vec::new(), cx);
            let confirmed = view.pending_confirm.take().unwrap();
            view.start_pending(confirmed, cx);
            assert_eq!(view.composer_ui.queues[&session_id].len(), 1);
            assert_eq!(view.run_sessions.len(), 1);
            view.set_connections(
                vec![AgentConnectionOption {
                    id: connection_id,
                    label: "remote".into(),
                    host: "fixture.invalid".into(),
                }],
                cx,
            );
            view.apply_selected_context(cx);
            view.admit_queued_message(session_id, &snapshot, cx);
            let confirmed = view.pending_confirm.take().unwrap();
            view.start_pending(confirmed, cx);
            assert!(view.composer_ui.queues[&session_id].is_empty());
            assert_eq!(view.run_sessions.len(), 2);
        });
    }
}
