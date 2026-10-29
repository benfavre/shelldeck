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
                access_label(context.access),
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
            .tooltip(title.clone());
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
                                        access_label(access),
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
                        for (index, (choice, label, icon, selected)) in
                            choices.into_iter().enumerate()
                        {
                            let parent = parent.clone();
                            content = content.child(
                                Button::new(("agent-composer-choice", index), label)
                                    .variant(ButtonVariant::Ghost)
                                    .size(ButtonSize::Sm)
                                    .w_full()
                                    .justify_start()
                                    .icon(IconSource::from(icon))
                                    .selected(selected)
                                    .disabled(locked && !matches!(menu, ComposerMenu::Attachments))
                                    .on_click(cx.listener(move |_, _, window, cx| {
                                        parent.update(cx, |view, cx| {
                                            if view.sessions.selected_id() != session_id {
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
