//! Per-session browser lifetimes and ephemeral provider tool grants.
use super::*;
use crate::native_browser::{NativeBrowserEvent, NativeBrowserView};
use adabraka_ui::components::split_pane::{SplitDirection, SplitPane, SplitPaneState};
use shelldeck_browser::mcp::{self, BrowserGrant, BrowserPermission};
use shelldeck_core::agent_browser_tools::AgentMcpServerLaunch;
use shelldeck_core::agent_runtime::AgentInputOptions;
use std::io::Write;

const MAX_BROWSER_CONTEXTS: usize = 4;

struct BrowserInstance {
    session_id: Uuid,
    context: AgentExecutionContext,
    view: Entity<NativeBrowserView>,
    visible: bool,
    _subscription: Subscription,
}

struct RunBrowserGrant {
    _grant: BrowserGrant,
    // Claude needs a private filename, not a capability in its process argv.
    _config: Option<tempfile::NamedTempFile>,
}

#[derive(Default)]
pub(super) struct AgentLiveBrowsersUi {
    instances: Vec<BrowserInstance>,
    grants: HashMap<Uuid, RunBrowserGrant>,
    pub(super) error: Option<String>,
    account_namespace: Option<Uuid>,
    split: Option<Entity<SplitPaneState>>,
}

impl AgentConsoleView {
    fn live_browser_index(&self, cx: &App) -> Option<usize> {
        let session_id = self.sessions.selected_id()?;
        let context = self.displayed_context(cx);
        self.live_browsers
            .instances
            .iter()
            .position(|b| b.session_id == session_id && b.context == context)
    }

    pub(super) fn live_browser_visible(&self, cx: &App) -> bool {
        self.live_browser_index(cx)
            .is_some_and(|i| self.live_browsers.instances[i].visible)
    }

    pub(super) fn focus_visible_browser(&self, cx: &mut Context<Self>) -> bool {
        let Some(index) = self
            .live_browser_index(cx)
            .filter(|index| self.live_browsers.instances[*index].visible)
        else {
            return false;
        };
        self.live_browsers.instances[index]
            .view
            .update(cx, |view, cx| view.request_focus(cx));
        true
    }

    pub(super) fn hide_live_browser(&mut self, cx: &App) {
        if let Some(i) = self.live_browser_index(cx) {
            self.live_browsers.instances[i].visible = false;
        }
    }

    pub(super) fn show_live_browser(&mut self, url: Option<String>, cx: &mut Context<Self>) {
        let Some(session_id) = self.sessions.selected_id() else {
            return;
        };
        let index = if let Some(i) = self.live_browser_index(cx) {
            i
        } else {
            if self.live_browsers.instances.len() >= MAX_BROWSER_CONTEXTS {
                self.live_browsers.error = Some(
                    t!(
                        "agents.live_browser.context_limit",
                        count = MAX_BROWSER_CONTEXTS
                    )
                    .to_string(),
                );
                cx.notify();
                return;
            }
            let context = self.displayed_context(cx);
            let scope = browser_scope(
                self.live_browsers.account_namespace.unwrap_or(Uuid::nil()),
                session_id,
                &context,
            );
            let observation_available = matches!(context.target, AgentTarget::Local)
                && matches!(
                    context.provider,
                    AgentProvider::Claude | AgentProvider::Codex | AgentProvider::Automonique
                );
            let view = cx.new(|cx| {
                let mut view = NativeBrowserView::new(scope, cx);
                view.set_agent_observation_available(observation_available, cx);
                view.set_agent_control_available(
                    observation_available && context.access != AgentAccessMode::ReadOnly,
                    cx,
                );
                view
            });
            let expected_context = context.clone();
            let subscription = cx.subscribe(&view, move |this, view, event, cx| {
                match event {
                    NativeBrowserEvent::ControlChanged { allowed: false } => {
                        if let Some(client) = view.read(cx).control() {
                            client.revoke_control_grants();
                        }
                    }
                    NativeBrowserEvent::Ready => this.live_browsers.error = None,
                    NativeBrowserEvent::ShareScreenshot {
                        png,
                        url: _,
                        title: _,
                    } => {
                        if !this.browser_screenshot_context_is_current(
                            session_id,
                            &expected_context,
                            cx,
                        ) || !this
                            .live_browsers
                            .instances
                            .iter()
                            .any(|instance| instance.view == view)
                        {
                            this.live_browsers.error =
                                Some(t!("agents.editor.context_changed").to_string());
                            cx.notify();
                            return;
                        }
                        match crate::issue_attachments::AttachmentDraft::from_bytes(
                            "browser.png",
                            png.clone(),
                        ) {
                            Ok(draft) => this.stage_agent_image(session_id, draft, cx),
                            Err(error) => this.live_browsers.error = Some(error),
                        }
                        if this.sessions.selected_id() == Some(session_id)
                            && this.displayed_context(cx) == expected_context
                        {
                            this.show_agent_conversation(cx);
                        }
                    }
                    // Runtime failures belong to the browser's retry banner only.
                    NativeBrowserEvent::Error(_) => {}
                    NativeBrowserEvent::CloseRequested
                        if this
                            .live_browser_index(cx)
                            .is_some_and(|i| this.live_browsers.instances[i].view == view) =>
                    {
                        this.close_current_browser(cx);
                    }
                    _ => {}
                }
                cx.notify();
            });
            self.live_browsers.instances.push(BrowserInstance {
                session_id,
                context,
                view,
                visible: false,
                _subscription: subscription,
            });
            self.live_browsers.instances.len() - 1
        };
        self.live_browsers.error = None;
        self.live_browsers.instances[index].visible = true;
        self.tools.panel_visible = false;
        self.tools.toolbar_expanded = false;
        let view = self.live_browsers.instances[index].view.clone();
        if let Some(url) = url {
            view.update(cx, |view, cx| view.navigate(url, cx));
        } else if view.read(cx).browser_ids().is_empty() {
            view.update(cx, |view, cx| view.navigate("about:blank".into(), cx));
        }
        view.update(cx, |view, cx| view.request_focus(cx));
        cx.notify();
    }

    fn browser_screenshot_context_is_current(
        &self,
        session_id: Uuid,
        context: &AgentExecutionContext,
        cx: &App,
    ) -> bool {
        // The visible draft may have unsent target/provider/workdir changes.
        // Saved execution context alone is not authority for a screenshot share.
        self.sessions.selected_id() == Some(session_id) && self.displayed_context(cx) == *context
    }

    pub(super) fn render_browser_split(
        &mut self,
        conversation: impl IntoElement,
        available_width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let direction = if available_width >= 860.0 {
            SplitDirection::Horizontal
        } else {
            SplitDirection::Vertical
        };
        let state = self
            .live_browsers
            .split
            .get_or_insert_with(|| {
                cx.new(|cx| {
                    let mut state = SplitPaneState::new(cx);
                    state.set_min_first(360.0, cx);
                    state.set_min_second(360.0, cx);
                    state.set_ratio(0.44, cx);
                    state
                })
            })
            .clone();
        if state.read(cx).direction() != direction {
            state.update(cx, |state, cx| {
                state.set_direction(direction, cx);
                state.set_min_first(
                    if direction == SplitDirection::Horizontal {
                        360.0
                    } else {
                        280.0
                    },
                    cx,
                );
                state.set_min_second(
                    if direction == SplitDirection::Horizontal {
                        360.0
                    } else {
                        200.0
                    },
                    cx,
                );
                state.set_ratio(
                    if direction == SplitDirection::Horizontal {
                        0.44
                    } else {
                        0.5
                    },
                    cx,
                );
            });
        }
        let browser = self
            .live_browser_index(cx)
            .map(|i| self.live_browsers.instances[i].view.clone());
        SplitPane::new(state)
            .first(
                div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .child(conversation),
            )
            .second(
                div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .children(browser),
            )
            .into_any_element()
    }

    /// Called only after the normal run admission/permission path. Capabilities
    /// live in this view until completion and never enter saved/queued requests.
    pub fn attach_browser_tools(
        &mut self,
        request: &AgentRunRequest,
        options: &mut AgentInputOptions,
        cx: &App,
    ) -> Result<(), String> {
        if !matches!(request.target, AgentTarget::Local)
            || !matches!(
                request.provider,
                AgentProvider::Codex | AgentProvider::Claude | AgentProvider::Automonique
            )
        {
            return Ok(());
        }
        let Some(session_id) = self.run_sessions.get(&request.id).copied() else {
            return Ok(());
        };
        let context = self.sessions.get(session_id).map(|s| &s.context);
        let Some(instance) = self
            .live_browsers
            .instances
            .iter()
            .find(|b| b.session_id == session_id && Some(&b.context) == context)
        else {
            return Ok(());
        };
        let view = instance.view.read(cx);
        let Some(client) = view.control() else {
            return Ok(());
        };
        let ids = view.browser_ids();
        if ids.is_empty() {
            return Ok(());
        }
        let permission =
            if view.agent_control_allowed() && request.access != AgentAccessMode::ReadOnly {
                BrowserPermission::Control
            } else {
                BrowserPermission::ObserveOnly
            };
        let grant = mcp::grant(client, ids, permission).map_err(|e| e.to_string())?;
        grant.include_new_tabs();
        let config = grant.configuration();
        let private_config = if request.provider == AgentProvider::Claude {
            let mut file = tempfile::Builder::new()
                .prefix("shelldeck-browser-")
                .suffix(".json")
                .tempfile()
                .map_err(|e| e.to_string())?;
            serde_json::to_writer(&mut file, &serde_json::json!({"mcpServers":{"shelldeck_browser":{"type":"stdio","command":config.command,"args":config.args,"env":config.env}}})).map_err(|e| e.to_string())?;
            file.flush().map_err(|e| e.to_string())?;
            Some(file)
        } else {
            None
        };
        options.mcp_servers.push(AgentMcpServerLaunch {
            name: "shelldeck_browser".into(),
            command: PathBuf::from(config.command),
            args: config.args,
            env: config.env.into_iter().collect(),
            claude_config_path: private_config.as_ref().map(|f| f.path().to_path_buf()),
        });
        self.live_browsers.grants.insert(
            request.id,
            RunBrowserGrant {
                _grant: grant,
                _config: private_config,
            },
        );
        Ok(())
    }

    pub fn retire_browser_run(&mut self, run_id: Uuid) {
        self.live_browsers.grants.remove(&run_id);
    }

    pub(super) fn close_current_browser(&mut self, cx: &mut Context<Self>) {
        if let Some(i) = self.live_browser_index(cx) {
            let session_id = self.live_browsers.instances[i].session_id;
            let runs: Vec<_> = self
                .run_sessions
                .iter()
                .filter_map(|(run, session)| (*session == session_id).then_some(*run))
                .collect();
            for run in runs {
                self.retire_browser_run(run);
            }
            let instance = self.live_browsers.instances.remove(i);
            instance.view.update(cx, |view, cx| view.close_all(cx));
        }
        cx.notify();
    }

    pub fn set_browser_account(
        &mut self,
        origin: &str,
        email: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let namespace = email.filter(|s| !s.trim().is_empty()).map(|email| {
            let identity = format!(
                "{}\0{}",
                origin.trim_end_matches('/'),
                email.trim().to_lowercase()
            );
            Uuid::new_v5(&Uuid::NAMESPACE_URL, identity.as_bytes())
        });
        if namespace != self.live_browsers.account_namespace || namespace.is_none() {
            self.close_all_agent_browsers(cx);
            self.live_browsers.account_namespace = namespace.or_else(|| Some(Uuid::new_v4()));
        }
    }

    pub fn close_all_agent_browsers(&mut self, cx: &mut Context<Self>) {
        self.live_browsers.grants.clear();
        for instance in &self.live_browsers.instances {
            instance.view.update(cx, |view, cx| view.close_all(cx));
        }
        self.live_browsers.instances.clear();
        cx.notify();
    }
}

fn browser_scope(
    account_namespace: Uuid,
    session_id: Uuid,
    context: &AgentExecutionContext,
) -> String {
    let context = serde_json::to_vec(context).expect("execution context serializes");
    let session_scope = Uuid::new_v5(&account_namespace, session_id.as_bytes());
    Uuid::new_v5(&session_scope, &context).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;
    // SDTEST-2587 — SDUC-557
    #[test]
    fn browser_profiles_separate_sessions_and_execution_contexts() {
        let a = Uuid::new_v4();
        let mut context = AgentExecutionContext {
            provider: AgentProvider::Codex,
            target: AgentTarget::Local,
            access: AgentAccessMode::ReadOnly,
            workdir: "/project".into(),
            model: None,
        };
        let account = Uuid::new_v4();
        let original = browser_scope(account, a, &context);
        assert_ne!(original, browser_scope(Uuid::new_v4(), a, &context));
        assert_ne!(original, browser_scope(account, Uuid::new_v4(), &context));
        context.access = AgentAccessMode::FullAccess;
        assert_ne!(original, browser_scope(account, a, &context));
        assert_eq!(
            browser_scope(account, a, &context),
            browser_scope(account, a, &context)
        );
        assert!(original.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'));
    }

    // SDTEST-2798 — a companion browser never replaces the conversation draft.
    #[test]
    fn companion_browser_keeps_conversation_draft_and_closes_only_its_runtime() {
        let mut app = TestAppContext::single();
        let view = app.update(|cx| cx.new(AgentConsoleView::new));
        view.update(&mut app, |view, cx| {
            let session_id = view.sessions.selected_id().unwrap();
            let context = view.displayed_context(cx);
            let browser = cx.new(|cx| NativeBrowserView::new("isolated-no-engine".into(), cx));
            let subscription = cx.subscribe(&browser, |_, _, _, _| {});
            view.live_browsers.instances.push(BrowserInstance {
                session_id,
                context: context.clone(),
                view: browser.clone(),
                visible: true,
                _subscription: subscription,
            });
            let prompt = view.session_ui.get(&session_id).unwrap().prompt.clone();
            prompt.update(cx, |input, cx| input.replace_content("Keep this draft", cx));
            view.show_agent_conversation(cx);
            assert!(view.live_browser_visible(cx));
            assert_eq!(prompt.read(cx).content(), "Keep this draft");
            view.hide_live_browser(cx);
            assert!(!view.live_browser_visible(cx));
            assert_eq!(view.live_browsers.instances[0].view, browser);
            view.close_current_browser(cx);
            assert!(view.live_browsers.instances.is_empty());
            assert_eq!(view.sessions.selected_id(), Some(session_id));
            assert_eq!(view.displayed_context(cx), context);
            assert_eq!(prompt.read(cx).content(), "Keep this draft");
        });
    }

    // SDTEST-2772 — screenshot sharing follows the visible draft, not stale persisted context.
    #[test]
    fn browser_screenshot_rejects_stale_draft_context_without_launching_browser() {
        let mut app = TestAppContext::single();
        let view = app.update(|cx| cx.new(AgentConsoleView::new));
        view.update(&mut app, |view, cx| {
            let session_id = view.sessions.selected_id().unwrap();
            let saved = view.sessions.selected().unwrap().context.clone();
            assert!(view.browser_screenshot_context_is_current(session_id, &saved, cx));
            let other_root = std::env::temp_dir()
                .join("shelldeck-unsent-browser-context")
                .display()
                .to_string();
            view.workdir_state
                .update(cx, |input, cx| input.replace_content(other_root, cx));
            let displayed = view.displayed_context(cx);
            assert_ne!(saved, displayed);
            assert_eq!(view.sessions.selected().unwrap().context, saved);
            assert!(!view.browser_screenshot_context_is_current(session_id, &saved, cx));
            assert!(view.browser_screenshot_context_is_current(session_id, &displayed, cx));
            assert!(!view.browser_screenshot_context_is_current(Uuid::new_v4(), &displayed, cx));
        });
    }
}
