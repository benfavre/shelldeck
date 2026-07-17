use adabraka_ui::components::confirm_dialog::Dialog as UiDialog;
use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::prelude::{scrollable_vertical, Badge, BadgeVariant, Button, ButtonVariant};
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::ai::{AiActionKind, AiActionPayload, AiActionPlan, AiActionRisk};

use crate::icons::lucide_icon;
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

pub fn render_ai_action_dialog(
    plan: AiActionPlan,
    on_close: impl Fn(&mut App) + Clone + 'static,
    on_confirm: impl Fn(&mut App) + Clone + 'static,
) -> impl IntoElement {
    let close_backdrop = on_close.clone();
    let close_cancel = on_close;
    let (kind_label, icon) = match plan.kind {
        AiActionKind::TerminalCommand => (t!("ai.action.kind.terminal").to_string(), "terminal"),
        AiActionKind::ScriptExecution => (t!("ai.action.kind.script").to_string(), "play"),
        AiActionKind::SupportSend => (t!("ai.action.kind.support").to_string(), "send"),
        AiActionKind::JeanDispatch => (t!("ai.action.kind.jean").to_string(), "bot"),
        AiActionKind::FleetDispatch => (t!("ai.action.kind.fleet").to_string(), "server"),
    };
    let (risk_label, risk_color) = match plan.risk {
        AiActionRisk::Low => (
            t!("ai.action.risk.low").to_string(),
            ShellDeckColors::success(),
        ),
        AiActionRisk::Moderate => (
            t!("ai.action.risk.moderate").to_string(),
            ShellDeckColors::warning(),
        ),
        AiActionRisk::High => (
            t!("ai.action.risk.high").to_string(),
            ShellDeckColors::error(),
        ),
    };
    let content = match &plan.payload {
        AiActionPayload::TerminalCommand { command } => command.clone(),
        AiActionPayload::ScriptExecution { body } => body.clone(),
        AiActionPayload::SupportSend { body } => body.clone(),
        AiActionPayload::JeanDispatch { prompt } => prompt.clone(),
        AiActionPayload::FleetDispatch {
            issue_id,
            instance_id,
        } => format!(
            "{}\n{}",
            t!("ai.action.issue", id = issue_id).as_ref(),
            t!("ai.action.instance", id = instance_id).as_ref()
        ),
    };
    let confirm_label = match plan.kind {
        AiActionKind::SupportSend => t!("ai.action.confirm_send").to_string(),
        AiActionKind::JeanDispatch | AiActionKind::FleetDispatch => {
            t!("ai.action.confirm_dispatch").to_string()
        }
        _ => t!("ai.action.confirm_execute").to_string(),
    };
    let confirm_icon = match plan.kind {
        AiActionKind::SupportSend => "send",
        AiActionKind::JeanDispatch | AiActionKind::FleetDispatch => "route",
        _ => "play",
    };
    let confirm_variant = if plan.risk == AiActionRisk::High {
        ButtonVariant::Destructive
    } else {
        ButtonVariant::Default
    };
    let warning = match plan.kind {
        AiActionKind::TerminalCommand => t!("ai.action.warning_terminal").to_string(),
        AiActionKind::ScriptExecution => {
            t!("ai.action.warning_script", seconds = plan.timeout_secs).to_string()
        }
        _ => t!("ai.action.warning_network", seconds = plan.timeout_secs).to_string(),
    };

    UiDialog::new()
        .width(gpui::px(560.0))
        .on_backdrop_click(move |_, cx| close_backdrop(cx))
        .header(
            div()
                .flex()
                .items_center()
                .gap(px(9.0))
                .px(px(16.0))
                .py(px(14.0))
                .border_b_1()
                .border_color(ShellDeckColors::border())
                .child(lucide_icon(
                    "shield-check",
                    17.0,
                    ShellDeckColors::primary(),
                ))
                .child(
                    div()
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ShellDeckColors::text_primary())
                        .child(t!("ai.action.confirm_title").to_string()),
                ),
        )
        .content(
            div()
                .flex()
                .flex_col()
                .gap(px(12.0))
                .px(px(16.0))
                .py(px(14.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(lucide_icon(icon, 14.0, ShellDeckColors::text_muted()))
                        .child(Badge::new(kind_label).variant(BadgeVariant::Outline))
                        .child(
                            Badge::new(risk_label)
                                .variant(BadgeVariant::Outline)
                                .border_color(risk_color)
                                .text_color(risk_color),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.0))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ShellDeckColors::text_muted())
                                .child(t!("ai.action.target").to_string()),
                        )
                        .child(
                            div()
                                .text_size(px(13.0))
                                .text_color(ShellDeckColors::text_primary())
                                .child(plan.target_label.clone()),
                        ),
                )
                .child(
                    div()
                        .h(px(190.0))
                        .min_h(px(0.0))
                        .overflow_hidden()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(ShellDeckColors::border())
                        .bg(ShellDeckColors::bg_primary())
                        .child(scrollable_vertical(
                            div()
                                .p(px(10.0))
                                .font_family("monospace")
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::text_primary())
                                .child(content),
                        )),
                )
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(7.0))
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::warning())
                        .child(lucide_icon(
                            "triangle-alert",
                            13.0,
                            ShellDeckColors::warning(),
                        ))
                        .child(warning),
                ),
        )
        .footer(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap(px(8.0))
                .px(px(16.0))
                .py(px(12.0))
                .border_t_1()
                .border_color(ShellDeckColors::border())
                .child(
                    Button::new("ai-action-cancel", t!("scripts.cancel").to_string())
                        .variant(ButtonVariant::Ghost)
                        .on_click(move |_, _, cx| close_cancel(cx)),
                )
                .child(
                    Button::new("ai-action-confirm", confirm_label)
                        .variant(confirm_variant)
                        .icon(IconSource::from(confirm_icon))
                        .on_click(move |_, _, cx| on_confirm(cx)),
                ),
        )
}
