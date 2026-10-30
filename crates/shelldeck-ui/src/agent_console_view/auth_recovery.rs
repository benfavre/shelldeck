//! Recovery guidance for a provider's explicit credential-import consent gate.
use super::*;

const DEEPSEEK_SETUP_COMMAND: &str = "jcode repl --provider deepseek --no-update --no-selfdev";

fn deepseek_consent_required(text: &str) -> bool {
    text.contains("Found existing DeepSeek credentials")
        && text.contains("will not read them without confirmation")
}

fn session_needs_deepseek_setup(session: &AgentSession) -> bool {
    if session.status != AgentSessionStatus::Failed
        || !matches!(
            session.context.provider,
            AgentProvider::DeepSeek | AgentProvider::Jcode
        )
    {
        return false;
    }
    let turn = session.current_turn_id().or_else(|| {
        session
            .turns
            .last()
            .filter(|turn| turn.context == session.context)
            .map(|turn| turn.id)
    });
    session.messages.iter().any(|message| {
        message.turn_id == turn && message.role == AgentMessageRole::Error
            && deepseek_consent_required(&message.text)
    }) || session.trace.iter().any(|trace| {
        trace.turn_id == turn
            && matches!(&trace.detail, AgentTraceKind::Activity { label } if deepseek_consent_required(label))
    })
}

impl AgentConsoleView {
    pub(super) fn render_auth_recovery(
        &self,
        session: &AgentSession,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !session_needs_deepseek_setup(session) {
            return None;
        }
        let context = session.context.clone();
        let session_id = session.id;
        let target = target_label(&context.target);
        let can_open = validate_agent_target(&context.target, &self.connections).is_ok();
        Some(
            div()
                .w_full()
                .min_w_0()
                .overflow_hidden()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .p(px(14.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(ShellDeckColors::warning().opacity(0.4))
                .bg(ShellDeckColors::bg_surface())
                .child(
                    div()
                        .text_size(px(13.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t!("agents.auth.deepseek_required").to_string()),
                )
                .child(
                    div()
                        .max_w(px(770.0))
                        .whitespace_normal()
                        .text_size(px(12.0))
                        .child(t!("agents.auth.deepseek_consent", target = target).to_string()),
                )
                .child(
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .whitespace_normal()
                        .text_size(px(12.0))
                        .font_family("JetBrains Mono")
                        .child(DEEPSEEK_SETUP_COMMAND),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(8.0))
                        .child(
                            Button::new(
                                "deepseek-copy-setup",
                                t!("ai.subscription.copy").to_string(),
                            )
                            .variant(ButtonVariant::Outline)
                            .size(ButtonSize::Sm)
                            .on_click(|_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    DEEPSEEK_SETUP_COMMAND.into(),
                                ))
                            }),
                        )
                        .child(
                            Button::new(
                                "deepseek-open-terminal",
                                t!("agents.auth.open_terminal").to_string(),
                            )
                            .variant(ButtonVariant::Default)
                            .size(ButtonSize::Sm)
                            .disabled(!can_open)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    // A stale card must not open a different target after navigation.
                                    if this.sessions.selected().is_some_and(|s| {
                                        s.id == session_id
                                            && s.context == context
                                            && session_needs_deepseek_setup(s)
                                    }) && validate_agent_target(
                                        &context.target,
                                        &this.connections,
                                    )
                                    .is_ok()
                                    {
                                        cx.emit(AgentConsoleEvent::OpenTerminal {
                                            context: context.clone(),
                                        });
                                    }
                                },
                            )),
                        ),
                )
                .child(
                    div()
                        .max_w(px(770.0))
                        .whitespace_normal()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("agents.auth.retry_after_setup").to_string()),
                )
                .into_any_element(),
        )
    }

    pub(super) fn concise_activity_label(label: String) -> String {
        if deepseek_consent_required(&label) {
            t!("agents.auth.deepseek_required").to_string()
        } else {
            label
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-2796 — recognize only actual provider diagnostics in the failed
    // current turn, including histories recorded before this recovery UI shipped.
    #[::core::prelude::v1::test]
    fn consent_recovery_ignores_user_text_other_providers_and_previous_turns() {
        let diagnostic = "Error: Found existing DeepSeek credentials from OpenCode auth.json but jcode will not read them without confirmation.";
        let mut session = AgentSession::new(
            "Test",
            AgentExecutionContext {
                provider: AgentProvider::DeepSeek,
                access: AgentAccessMode::ReadOnly,
                target: AgentTarget::Local,
                workdir: "/fixture".into(),
                model: None,
            },
            0,
        )
        .unwrap();
        let _ = session.begin_run(diagnostic, 1).unwrap();
        session.finish(AgentSessionStatus::Failed, 2, true).unwrap();
        assert!(!session_needs_deepseek_setup(&session));
        session.apply_stream_event(AgentStreamEvent::Activity(diagnostic.into()), 3, true);
        assert!(session_needs_deepseek_setup(&session));
        let restored: AgentSession =
            serde_json::from_str(&serde_json::to_string(&session).unwrap()).unwrap();
        assert!(restored.current_turn_id().is_none());
        assert!(session_needs_deepseek_setup(&restored));
        session.context.provider = AgentProvider::Claude;
        assert!(!session_needs_deepseek_setup(&session));
        session.context.provider = AgentProvider::DeepSeek;
        session.begin_run("Retry", 4).unwrap();
        session.finish(AgentSessionStatus::Failed, 5, true).unwrap();
        assert!(!session_needs_deepseek_setup(&session));
        session.apply_stream_event(AgentStreamEvent::Error(diagnostic.into()), 6, true);
        assert!(session_needs_deepseek_setup(&session));
        assert!(!deepseek_consent_required("DeepSeek: network unavailable"));
    }
}
