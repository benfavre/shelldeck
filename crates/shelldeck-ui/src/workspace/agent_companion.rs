use super::*;

impl Workspace {
    pub(super) fn companion_owns_agents(&self, cx: &App) -> bool {
        self.ai_dock_open && self.ai_dock_assistant.read(cx).has_shared_agents()
    }

    /// One retained console has exactly one presentation host. The Workspace
    /// continues owning runtime subscriptions while the companion displays it.
    pub(super) fn sync_agent_companion_host(&mut self, cx: &mut Context<Self>) {
        let claimed = self.ai_dock_open && self.can_access_mode(AppMode::Dev);
        let expected = claimed.then(|| self.agent_console.entity_id());
        let current = self
            .ai_dock_assistant
            .read(cx)
            .shared_agents
            .as_ref()
            .map(|view| view.entity_id());
        if current == expected {
            return;
        }
        self.ai_dock_assistant.update(cx, |assistant, cx| {
            assistant.set_shared_agent_console(claimed.then(|| self.agent_console.clone()), cx);
        });
        self.workspace_hub.update(cx, |hub, cx| {
            hub.set_agent_host((!claimed).then(|| self.agent_console.clone()), cx);
        });
        let visible = claimed && self.ai_dock_assistant.read(cx).shared_agent_chat_visible();
        self.agent_console.update(cx, |view, cx| {
            view.set_workflows_authorized(self.can_access_mode(AppMode::Dev), cx);
            view.prepare_window_host(cx);
            view.set_surface_visible(visible, cx);
        });
        if !claimed {
            // Reuse the ordinary pane/session visibility projection when returning.
            // The recursive sync is already settled (both host identities are None).
            self.on_active_view_changed(cx);
        }
    }

    pub(super) fn render_agents_in_companion(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .items_center()
            .justify_center()
            .gap(px(12.0))
            .p(px(24.0))
            .text_color(ShellDeckColors::text_muted())
            .child(t!("agents.companion.open").to_string())
            .child(
                Button::new(
                    "agents-return-from-companion",
                    t!("agents.companion.return").to_string(),
                )
                .debug_selector(|| "agents-return-from-companion".into())
                .variant(ButtonVariant::Secondary)
                .size(ButtonSize::Sm)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.ai_dock_assistant
                        .update(cx, |view, cx| view.open_main_window(cx));
                })),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-2939 / SDUC-434, SDUC-564: no duplicate host and no Dev-capability bypass.
    #[::core::prelude::v1::test]
    fn shared_assistant_host_is_exclusive_and_revalidates_account_access() {
        let mut app = TestAppContext::single();
        app.update(|cx| {
            adabraka_ui::init(cx);
            adabraka_ui::components::input::init(cx);
        });
        let workspace = app.update(|cx| {
            let assistant = cx.new(|cx| {
                AiAssistantView::new(
                    AiContext::new(AiSurface::Global, "Fixture", serde_json::json!({})),
                    cx,
                )
            });
            let mut config = AppConfig::default();
            config.general.ui_font_family = crate::settings::normalize_ui_font_family("Inter", cx);
            config.account = Some(AccountInfo {
                email: "fixture@example.test".into(),
                is_superadmin: true,
                ..Default::default()
            });
            config.cloud_sync.enabled = true;
            config.cloud_sync.token = "offline-fixture-token".into();
            config.cloud_sync.base_url = "https://fixture.invalid".into();
            cx.new(|cx| {
                Workspace::new(
                    cx,
                    config,
                    vec![],
                    ConnectionStore::default(),
                    WorkspaceAiBindings {
                        assistant,
                        tasks: vec![],
                        config: Rc::new(RefCell::new(AiConfig::default())),
                        clippy_config: Rc::new(RefCell::new(ClippyConfig::default())),
                    },
                )
            })
        });
        workspace.update(&mut app, |view, cx| {
            let retained = view.agent_console.entity_id();
            let session = view.agent_console.read(cx).selected_session_id();
            view.set_ai_dock_open(true, cx);
            assert!(view.companion_owns_agents(cx));
            assert_eq!(
                view.ai_dock_assistant
                    .read(cx)
                    .shared_agents
                    .as_ref()
                    .unwrap()
                    .entity_id(),
                retained
            );
            assert!(view
                .workspace_hub
                .read(cx)
                .attached_agent_host_id()
                .is_none());
            assert!(
                !view.app_config.ai.enabled,
                "sharing does not enable legacy helper execution"
            );
            view.set_ai_dock_open(false, cx);
            assert!(!view.companion_owns_agents(cx));
            assert_eq!(
                view.workspace_hub.read(cx).attached_agent_host_id(),
                Some(retained)
            );
            assert_eq!(view.agent_console.read(cx).selected_session_id(), session);
            view.set_ai_dock_open(true, cx);
            view.app_config.account.as_mut().unwrap().is_superadmin = false;
            view.app_config.account.as_mut().unwrap().is_admin = true;
            view.sync_agent_companion_host(cx);
            assert!(
                !view.companion_owns_agents(cx),
                "generic admin access cannot unlock Agents"
            );
            assert!(!view.ai_dock_assistant.read(cx).has_shared_agents());
            view.set_ai_dock_open(false, cx);
            view.set_ai_dock_open(true, cx);
            assert!(!view.companion_owns_agents(cx));
        });
    }
}
