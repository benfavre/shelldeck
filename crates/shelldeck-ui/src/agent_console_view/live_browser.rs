//! Per-session browser lifetimes and ephemeral provider tool grants.
use super::*;
use crate::native_browser::{NativeBrowserEvent, NativeBrowserView};
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
            let control_available = context.access != AgentAccessMode::ReadOnly
                && matches!(context.target, AgentTarget::Local)
                && matches!(
                    context.provider,
                    AgentProvider::Claude | AgentProvider::Codex | AgentProvider::Automonique
                );
            let view = cx.new(|cx| {
                let mut view = NativeBrowserView::new(scope, cx);
                view.set_agent_control_available(control_available, cx);
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
                        if !this
                            .sessions
                            .get(session_id)
                            .is_some_and(|s| s.context == expected_context)
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
                    NativeBrowserEvent::Error(error) => {
                        this.live_browsers.error = Some(error.clone())
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
        if !self.tools.inspector_wide {
            self.tools.panel_visible = false;
        }
        let view = self.live_browsers.instances[index].view.clone();
        if let Some(url) = url {
            view.update(cx, |view, cx| view.navigate(url, cx));
        } else if view.read(cx).browser_ids().is_empty() {
            view.update(cx, |view, cx| view.navigate("about:blank".into(), cx));
        }
        cx.notify();
    }

    pub(super) fn render_live_browser(&self, cx: &App) -> AnyElement {
        let mut pane = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden();
        if let Some(i) = self.live_browser_index(cx) {
            let instance = &self.live_browsers.instances[i];
            if !matches!(instance.context.target, AgentTarget::Local)
                || !matches!(
                    instance.context.provider,
                    AgentProvider::Codex | AgentProvider::Claude | AgentProvider::Automonique
                )
            {
                pane = pane.child(
                    div()
                        .p(px(8.0))
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("agents.live_browser.local_tools").to_string()),
                );
            }
            pane = pane.child(instance.view.clone());
        }
        pane.into_any_element()
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
}
