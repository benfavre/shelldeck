//! Shared popovers for the composer’s agent, permissions and context actions.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum ComposerMenu {
    Agent,
    Access,
    Attachments,
}

#[derive(Clone, Copy)]
enum ComposerChoice {
    Agent(AgentProvider),
    Access(AgentAccessMode),
    Attach,
    Mention,
    Paste,
    Capture,
}

pub(super) fn provider_access_label(provider: AgentProvider, access: AgentAccessMode) -> String {
    match (provider, access) {
        (AgentProvider::Claude, AgentAccessMode::ReadOnly) => {
            t!("agents.access.claude_plan").to_string()
        }
        (AgentProvider::Jcode | AgentProvider::DeepSeek, AgentAccessMode::ReadOnly) => {
            t!("agents.access.read_tools").to_string()
        }
        _ => access_label(access),
    }
}

fn provider_access_description(provider: AgentProvider, access: AgentAccessMode) -> String {
    match (provider, access) {
        (AgentProvider::Claude, AgentAccessMode::ReadOnly) => {
            t!("agents.access.claude_plan_help").to_string()
        }
        (AgentProvider::Jcode | AgentProvider::DeepSeek, AgentAccessMode::ReadOnly) => {
            t!("agents.access.read_tools_help").to_string()
        }
        _ => provider_access_label(provider, access),
    }
}

impl AgentConsoleView {
    pub(super) fn render_composer_menu(
        &self,
        menu: ComposerMenu,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let context = self.displayed_context(cx);
        let (id, label, icon, title) = match menu {
            ComposerMenu::Agent => (
                "agent-picker",
                context.provider.display_name().to_owned(),
                provider_icon(context.provider),
                t!("agents.provider").to_string(),
            ),
            ComposerMenu::Access => (
                "agent-permissions-picker",
                provider_access_label(context.provider, context.access),
                match context.access {
                    AgentAccessMode::ReadOnly => "lock",
                    AgentAccessMode::WorkspaceWrite => "pencil",
                    AgentAccessMode::FullAccess => "shield",
                },
                t!("agents.access.label").to_string(),
            ),
            ComposerMenu::Attachments => (
                "agent-context-actions",
                String::new(),
                "plus",
                t!("agents.composer.add_context").to_string(),
            ),
        };
        let trigger = Button::new(id, label)
            .variant(ButtonVariant::Ghost)
            .size(if matches!(menu, ComposerMenu::Attachments) {
                ButtonSize::Icon
            } else {
                ButtonSize::Sm
            })
            .h(px(30.0))
            .icon(IconSource::from(icon))
            .tooltip(if matches!(menu, ComposerMenu::Access) {
                provider_access_description(context.provider, context.access)
            } else {
                title.clone()
            });
        let parent = cx.entity();
        Popover::new(SharedString::from(format!("{id}-popover")))
            .anchor(Corner::BottomLeft)
            .trigger(trigger)
            .content(move |window, cx| {
                let parent = parent.clone();
                let title = title.clone();
                cx.new(move |content_cx| {
                    content_cx.observe(&parent, |_, _, cx| cx.notify()).detach();
                    PopoverContent::new(window, content_cx, move |_, cx| {
                        let view = parent.read(cx);
                        let session_id = view.sessions.selected_id();
                        let context = view.displayed_context(cx);
                        let locked = view.context_controls_locked();
                        let mut choices = Vec::new();
                        match menu {
                            ComposerMenu::Agent => {
                                for provider in [
                                    AgentProvider::Claude,
                                    AgentProvider::Codex,
                                    AgentProvider::Automonique,
                                    AgentProvider::Jcode,
                                    AgentProvider::DeepSeek,
                                ] {
                                    choices.push((
                                        ComposerChoice::Agent(provider),
                                        provider.display_name().to_owned(),
                                        provider_icon(provider),
                                        provider == context.provider,
                                    ));
                                }
                            }
                            ComposerMenu::Access => {
                                for (access, icon) in [
                                    (AgentAccessMode::ReadOnly, "lock"),
                                    (AgentAccessMode::WorkspaceWrite, "pencil"),
                                    (AgentAccessMode::FullAccess, "shield"),
                                ] {
                                    choices.push((
                                        ComposerChoice::Access(access),
                                        provider_access_label(context.provider, access),
                                        icon,
                                        access == context.access,
                                    ));
                                }
                            }
                            ComposerMenu::Attachments => {
                                choices.extend([
                                    (
                                        ComposerChoice::Attach,
                                        t!("ai.attachment.choose").to_string(),
                                        "paperclip",
                                        false,
                                    ),
                                    (
                                        ComposerChoice::Mention,
                                        t!("agents.composer.mention_file").to_string(),
                                        "file-text",
                                        false,
                                    ),
                                    (
                                        ComposerChoice::Paste,
                                        t!("agents.composer.paste").to_string(),
                                        "copy",
                                        false,
                                    ),
                                ]);
                                if context.provider == AgentProvider::Codex
                                    && matches!(context.target, AgentTarget::Local)
                                {
                                    choices.push((
                                        ComposerChoice::Capture,
                                        t!("agents.composer.capture").to_string(),
                                        "scan",
                                        false,
                                    ));
                                }
                            }
                        }
                        let mut content = div()
                            .w(px(280.0))
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .pb(px(6.0))
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title.clone()),
                            );
                        if matches!(menu, ComposerMenu::Access) {
                            content = content.child(
                                div()
                                    .text_size(px(11.0))
                                    .whitespace_normal()
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(provider_access_description(
                                        context.provider,
                                        context.access,
                                    )),
                            );
                        }
                        for (index, (choice, label, icon, selected)) in
                            choices.into_iter().enumerate()
                        {
                            let parent = parent.clone();
                            let expected_context = context.clone();
                            let unavailable = match choice {
                                ComposerChoice::Agent(AgentProvider::Automonique)
                                    if context.target != AgentTarget::Local =>
                                {
                                    Some("agents.error.automonique_ssh_boundary")
                                }
                                ComposerChoice::Mention if context.target != AgentTarget::Local => {
                                    Some("agents.composer.remote_mention")
                                }
                                _ => None,
                            };
                            let tooltip = unavailable
                                .map(|key| t!(key).to_string())
                                .unwrap_or_else(|| match choice {
                                    ComposerChoice::Access(access) => {
                                        provider_access_description(context.provider, access)
                                    }
                                    _ => label.clone(),
                                });
                            content = content.child(
                                Button::new(("agent-composer-choice", index), label)
                                    .variant(ButtonVariant::Ghost)
                                    .size(ButtonSize::Sm)
                                    .w_full()
                                    .justify_start()
                                    .icon(IconSource::from(icon))
                                    .selected(selected)
                                    .disabled(
                                        unavailable.is_some()
                                            || (locked
                                                && !matches!(menu, ComposerMenu::Attachments)),
                                    )
                                    .tooltip(tooltip)
                                    .on_click(cx.listener(move |_, _, window, cx| {
                                        parent.update(cx, |view, cx| {
                                            if view.sessions.selected_id() != session_id
                                                || view.displayed_context(cx) != expected_context
                                                || unavailable.is_some()
                                            {
                                                return;
                                            }
                                            match choice {
                                                ComposerChoice::Agent(provider) => {
                                                    view.change_draft_provider(provider, cx)
                                                }
                                                ComposerChoice::Access(access) => {
                                                    view.change_draft_access(access, cx)
                                                }
                                                ComposerChoice::Attach => {
                                                    view.pick_agent_attachment(false, window, cx)
                                                }
                                                ComposerChoice::Mention => {
                                                    view.pick_agent_attachment(true, window, cx)
                                                }
                                                ComposerChoice::Paste => {
                                                    view.paste_agent_context(cx)
                                                }
                                                ComposerChoice::Capture => {
                                                    view.capture_agent_context(cx)
                                                }
                                            }
                                        });
                                        cx.emit(DismissEvent);
                                    })),
                            );
                        }
                        if locked && !matches!(menu, ComposerMenu::Attachments) {
                            content = content.child(
                                div()
                                    .pt(px(6.0))
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(t!("agents.selection.locked").to_string()),
                            );
                        } else if matches!(menu, ComposerMenu::Access) {
                            content = content.child(
                                div()
                                    .pt(px(6.0))
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(t!("agents.selection.permissions_hint").to_string()),
                            );
                        }
                        content.into_any_element()
                    })
                })
            })
            .into_any_element()
    }
}
