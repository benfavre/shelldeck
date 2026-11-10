use super::*;
use crate::{icons::lucide_icon, scale::px};
use adabraka_ui::components::{
    button::{Button, ButtonSize, ButtonVariant},
    checkbox::Checkbox,
    confirm_dialog::Dialog,
    input::Input,
};
use gpui::prelude::*;

pub(super) fn status(value: &str) -> (String, Hsla) {
    let (key, color) = match value {
        "ACTIVE" => ("active", ShellDeckColors::success()),
        "COMPLETED" => ("completed", ShellDeckColors::success()),
        "FAILED" | "ERROR" => ("failed", ShellDeckColors::error()),
        "PAUSED" => ("paused", ShellDeckColors::warning()),
        "RUNNING" | "PENDING" => ("running", ShellDeckColors::primary()),
        "DRAFT" => ("draft", ShellDeckColors::text_muted()),
        "ARCHIVED" | "CANCELLED" | "CANCELED" => ("archived", ShellDeckColors::text_muted()),
        _ => ("unknown", ShellDeckColors::text_muted()),
    };
    (t!(format!("automations.status.{key}")).to_string(), color)
}
pub(super) fn date(value: Option<&str>) -> String {
    value
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|date| {
            date.with_timezone(&chrono::Local)
                .format("%d/%m/%Y %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| t!("automations.not_scheduled").to_string())
}
pub(super) fn field(label: String, input: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .w_full()
        .child(
            div()
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(ShellDeckColors::text_primary())
                .child(label),
        )
        .child(input)
}

impl AutomationsView {
    fn render_error(&self, cx: &mut Context<Self>) -> Div {
        let storage_error = self
            .local_agents
            .as_ref()
            .and_then(|agents| agents.read(cx).local_task_storage_error())
            .map(|message| AutomationError {
                status: 0,
                needs_reconnect: false,
                message: format!("{}: {}", t!("automations.target_local"), message),
            });
        let Some(error) = self.error.as_ref().or(storage_error.as_ref()) else {
            return div();
        };
        let mut banner = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(12.0))
            .p(px(12.0))
            .rounded(px(8.0))
            .bg(ShellDeckColors::error().opacity(0.08))
            .child(
                div()
                    .flex_1()
                    .min_w(px(160.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::error())
                    .child(error.message.clone()),
            );
        if error.needs_reconnect {
            banner = banner.child(
                Button::new(
                    "automation-reconnect",
                    t!("automations.reconnect").to_string(),
                )
                .size(ButtonSize::Sm)
                .on_click(cx.listener(|_, _, _, cx| cx.emit(AutomationEvent::Reconnect))),
            );
        } else if self.form.is_none() && storage_error.is_none() {
            banner = banner.child(
                Button::new("automation-retry", t!("automations.retry").to_string())
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .disabled(self.busy)
                    .on_click(cx.listener(|view, _, _, cx| view.refresh(cx))),
            );
        }
        banner
    }

    fn render_templates(&self, columns: u16, cx: &mut Context<Self>) -> Div {
        let mut templates = div()
            .grid()
            .grid_cols(columns)
            .flex_shrink_0()
            .gap(px(16.0))
            .w_full();
        for (index, icon) in [
            "bot",
            "mail",
            "server",
            "scroll-text",
            "globe",
            "calendar",
            "globe",
        ]
        .into_iter()
        .enumerate()
        {
            // Button's single label cannot show the starter's editable task brief.
            templates = templates.child(
                div()
                    .id(("automation-template", index))
                    .debug_selector(move || format!("automation-template-{index}"))
                    .flex()
                    .items_center()
                    .gap(px(16.0))
                    .w_full()
                    .min_w(px(0.0))
                    .min_h(px(104.0))
                    .p(px(20.0))
                    .rounded(px(12.0))
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .cursor_pointer()
                    .hover(|style| {
                        style
                            .bg(ShellDeckColors::hover_bg())
                            .border_color(ShellDeckColors::primary())
                    })
                    .child(lucide_icon(icon, 22.0, ShellDeckColors::primary()))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w(px(0.0))
                            .gap(px(6.0))
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(
                                        t!(format!("automations.template.{index}.title"))
                                            .to_string(),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(
                                        t!(format!("automations.template.{index}.description"))
                                            .to_string(),
                                    ),
                            ),
                    )
                    .on_click(cx.listener(move |view, _, _, cx| view.start_task(Some(index), cx))),
            );
        }
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(28.0))
            .w_full()
            .max_w(px(800.0))
            .debug_selector(|| "automation-template-content".into())
            .flex_shrink_0()
            .my_auto()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(12.0))
                    .child(lucide_icon("calendar", 40.0, ShellDeckColors::primary()))
                    .child(
                        div()
                            .text_size(px(24.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("automations.empty_title").to_string()),
                    )
                    .child(
                        div()
                            .max_w(px(540.0))
                            .text_center()
                            .text_size(px(13.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("automations.empty_hint").to_string()),
                    ),
            )
            .child(templates)
    }

    fn render_list(&self, columns: u16, cx: &mut Context<Self>) -> Div {
        let mut body = div()
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(1100.0))
            .gap(px(12.0));
        let empty = AutomationData::default();
        let data = self.data.as_ref().unwrap_or(&empty);
        if self.data.is_none() && self.tab != Tab::Tasks && self.tab != Tab::Runs {
            return body.items_center().my_auto().child(
                div()
                    .text_size(px(14.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(if self.context.is_none() {
                        t!("automations.select_site").to_string()
                    } else if self.busy {
                        t!("automations.loading").to_string()
                    } else {
                        t!("automations.unavailable").to_string()
                    }),
            );
        }
        let query = self.search.read(cx).content().to_lowercase();
        if self.tab == Tab::Tasks
            && data.tasks.is_empty()
            && self.local_tasks(cx).is_empty()
            && query.is_empty()
        {
            return self.render_templates(columns, cx);
        }
        let mut count = 0;
        let local_rows = if self.tab == Tab::Tasks {
            self.render_local_rows(&query, cx)
        } else if self.tab == Tab::Runs && self.page == 1 {
            self.render_local_history(&query, cx)
        } else {
            Vec::new()
        };
        count += local_rows.len();
        body = body.children(local_rows);
        if self.tab == Tab::Runs {
            for run in data
                .runs
                .iter()
                .filter(|run| run.name.to_lowercase().contains(&query))
            {
                count += 1;
                let (label, color) = status(&run.status);
                let id = run.id.clone();
                body = body.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(16.0))
                        .p(px(16.0))
                        .border_b_1()
                        .border_color(ShellDeckColors::border())
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(180.0))
                                .text_size(px(14.0))
                                .text_color(ShellDeckColors::text_primary())
                                .child(run.name.clone()),
                        )
                        .child(div().text_size(px(12.0)).text_color(color).child(label))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::primary())
                                .child(t!("automations.target_manage").to_string()),
                        )
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(date(run.started_at.as_deref())),
                        )
                        .child(
                            Button::new(
                                SharedString::from(format!("open-run-{id}")),
                                t!("automations.view_result").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .disabled(self.busy)
                            .on_click(cx.listener(
                                move |view, _, _, cx| {
                                    view.operation(
                                        Operation::Open("runs".into(), Some(id.clone())),
                                        cx,
                                    )
                                },
                            )),
                        ),
                );
            }
        } else {
            let workflows = if self.tab == Tab::Tasks {
                &data.tasks
            } else {
                &data.workflows
            };
            for workflow in workflows.iter().filter(|workflow| {
                format!("{} {}", workflow.name, workflow.description)
                    .to_lowercase()
                    .contains(&query)
            }) {
                count += 1;
                let id = workflow.id.clone();
                let open_id = id.clone();
                let test_id = id.clone();
                let (label, color) = status(&workflow.status);
                body = body.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .p(px(18.0))
                        .w_full()
                        .rounded(px(10.0))
                        .border_1()
                        .border_color(ShellDeckColors::border())
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(px(12.0))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(160.0))
                                        .text_size(px(15.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(ShellDeckColors::text_primary())
                                        .child(workflow.name.clone()),
                                )
                                .child(div().text_size(px(12.0)).text_color(color).child(label)),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::primary())
                                .child(t!("automations.target_manage").to_string()),
                        )
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(ShellDeckColors::text_muted())
                                .line_clamp(2)
                                .child(workflow.description.clone()),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(px(8.0))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(180.0))
                                        .text_size(px(11.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(
                                            t!(
                                                "automations.next_run",
                                                date = date(workflow.next_run.as_deref())
                                            )
                                            .to_string(),
                                        ),
                                )
                                .child(
                                    Button::new(
                                        SharedString::from(format!("automation-detail-{id}")),
                                        t!("automations.details").to_string(),
                                    )
                                    .variant(ButtonVariant::Secondary)
                                    .size(ButtonSize::Sm)
                                    .disabled(self.busy)
                                    .on_click(cx.listener(
                                        move |view, _, _, cx| {
                                            view.operation(Operation::Edit(id.clone()), cx)
                                        },
                                    )),
                                )
                                .child(
                                    Button::new(
                                        SharedString::from(format!("automation-test-{test_id}")),
                                        t!("automations.test").to_string(),
                                    )
                                    .variant(ButtonVariant::Ghost)
                                    .size(ButtonSize::Sm)
                                    .disabled(self.busy)
                                    .on_click(cx.listener(
                                        move |view, _, _, cx| {
                                            view.operation(
                                                Operation::Action(
                                                    json!({ "action": "test", "id": test_id }),
                                                ),
                                                cx,
                                            )
                                        },
                                    )),
                                )
                                .child(
                                    Button::new(
                                        SharedString::from(format!("automation-open-{open_id}")),
                                        t!("automations.open_manage").to_string(),
                                    )
                                    .variant(ButtonVariant::Ghost)
                                    .size(ButtonSize::Sm)
                                    .disabled(self.busy)
                                    .on_click(cx.listener(
                                        move |view, _, _, cx| {
                                            view.operation(
                                                Operation::Open(
                                                    "workflows".into(),
                                                    Some(open_id.clone()),
                                                ),
                                                cx,
                                            )
                                        },
                                    )),
                                ),
                        ),
                );
            }
        }
        if count == 0 {
            body = body.child(
                div()
                    .p(px(32.0))
                    .text_center()
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("automations.no_results").to_string()),
            );
        }
        let total = match self.tab {
            Tab::Tasks => data.task_total,
            Tab::Workflows => data.total,
            Tab::Runs => data.run_total,
        };
        let size = if self.tab == Tab::Runs { 20 } else { 100 };
        body.child(
            div()
                .flex()
                .justify_end()
                .items_center()
                .gap(px(12.0))
                .child(
                    Button::new(
                        "automation-previous",
                        t!("automations.previous").to_string(),
                    )
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .disabled(self.busy || self.page <= 1)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.page = view.page.saturating_sub(1).max(1);
                        view.refresh(cx);
                    })),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("automations.page", page = self.page).to_string()),
                )
                .child(
                    Button::new("automation-next", t!("automations.next").to_string())
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .disabled(self.busy || self.page * size >= total)
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.page += 1;
                            view.refresh(cx);
                        })),
                ),
        )
    }

    fn render_form(&self, columns: u16, cx: &mut Context<Self>) -> Div {
        let form = self.form.as_ref().unwrap();
        let mut presets = div()
            .grid()
            .grid_cols(if columns == 1 { 1 } else { 3 })
            .gap(px(8.0));
        for (index, cron) in ["0 9 * * *", "0 9 * * 1-5", "0 9 * * 5"]
            .into_iter()
            .enumerate()
        {
            presets = presets.child(
                Button::new(
                    ("automation-cadence", index),
                    t!(format!("automations.cadence.{index}")).to_string(),
                )
                .size(ButtonSize::Sm)
                .variant(ButtonVariant::Secondary)
                .disabled(self.busy)
                .selected(form.cron.read(cx).content() == cron)
                .on_click(cx.listener(move |view, _, _, cx| {
                    if let Some(form) = &view.form {
                        form.cron
                            .update(cx, |state, cx| state.replace_content(cron, cx));
                    }
                    cx.notify();
                })),
            );
        }
        div()
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(720.0))
            .gap(px(20.0))
            .child(
                div()
                    .text_size(px(22.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(
                        t!(if form.id.is_some() || form.local_id.is_some() {
                            "automations.edit_task"
                        } else {
                            "automations.new_task"
                        })
                        .to_string(),
                    ),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("automations.draft_hint").to_string()),
            )
            .child(self.render_execution_target(cx))
            .child(field(
                t!("automations.task_title").to_string(),
                Input::new(&form.name)
                    .debug_selector("automation-name")
                    .disabled(self.busy),
            ))
            .child(field(
                t!("automations.instructions").to_string(),
                Input::new(&form.prompt)
                    .debug_selector("automation-prompt")
                    .multi_line(true)
                    .min_rows(4)
                    .max_rows(8)
                    .disabled(self.busy),
            ))
            .child(field(t!("automations.schedule").to_string(), presets))
            .child(field(
                t!("automations.cron").to_string(),
                Input::new(&form.cron)
                    .debug_selector("automation-cron")
                    .disabled(self.busy),
            ))
            .child(field(
                t!("automations.timezone").to_string(),
                Input::new(&form.timezone)
                    .debug_selector("automation-timezone")
                    .disabled(self.busy),
            ))
            .child(
                Checkbox::new("automation-once")
                    .checked(form.once)
                    .disabled(self.busy)
                    .label(t!("automations.once").to_string())
                    .on_click(cx.listener(|view, checked: &bool, _, cx| {
                        if let Some(form) = &mut view.form {
                            form.once = *checked;
                        }
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(
                        t!(if form.target == TaskTarget::Local {
                            "automations.local_run_hint"
                        } else {
                            "automations.provider_hint"
                        })
                        .to_string(),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(10.0))
                    .child(
                        Button::new(
                            "automation-save-draft",
                            t!("automations.save_draft").to_string(),
                        )
                        .debug_selector(|| "automation-save-draft".into())
                        .loading(self.busy)
                        .disabled(
                            form.target == TaskTarget::Local
                                && (!self.local_ready(cx) || form.session_id.is_none()),
                        )
                        .on_click(cx.listener(|view, _, _, cx| view.save_form(cx))),
                    )
                    .child(
                        Button::new("automation-cancel", t!("automations.cancel").to_string())
                            .variant(ButtonVariant::Ghost)
                            .disabled(self.busy)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.form = None;
                                view.error = None;
                                cx.notify();
                            })),
                    ),
            )
    }

    fn render_editor(&self, cx: &mut Context<Self>) -> Div {
        let editor = self.editor.as_ref().unwrap();
        let id = editor.id.clone();
        let (label, color) = status(&editor.status);
        let mut body = div()
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(800.0))
            .gap(px(20.0))
            .child(
                div()
                    .text_size(px(24.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(editor.name.clone()),
            )
            .child(div().text_size(px(13.0)).text_color(color).child(label));
        if editor.editable {
            body = body
                .child(
                    div()
                        .text_size(px(14.0))
                        .text_color(ShellDeckColors::text_primary())
                        .child(editor.prompt.clone()),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(format!(
                            "{} · {}{}",
                            editor.cron,
                            editor.timezone,
                            if editor.once {
                                format!(" · {}", t!("automations.once"))
                            } else {
                                String::new()
                            }
                        )),
                );
        } else {
            body = body.child(
                div()
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("automations.complex_workflow").to_string()),
            );
        }
        if editor.has_unpublished_changes {
            body = body.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::warning())
                    .child(t!("automations.unpublished").to_string()),
            );
        }
        for issue in &editor.issues {
            if let Some(message) = issue.get("message").and_then(Value::as_str) {
                body = body.child(
                    div()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::error())
                        .child(message.to_string()),
                );
            }
        }
        let mut actions = div().flex().flex_wrap().gap(px(8.0));
        if editor.editable {
            let draft = editor.draft();
            let publish_id = id.clone();
            let revision = editor.revision.clone();
            let delete_id = id.clone();
            actions = actions.child(
                Button::new(
                    "automation-edit-task",
                    t!("automations.edit_task").to_string(),
                )
                .variant(ButtonVariant::Secondary)
                .disabled(self.busy)
                .on_click(cx.listener(move |view, _, _, cx| view.new_form(draft.clone(), cx))),
            );
            if editor.status != "ACTIVE" || editor.has_unpublished_changes {
                actions = actions.child(Button::new("automation-publish", t!(if editor.has_unpublished_changes { "automations.publish_changes" } else { "automations.activate" }).to_string()).disabled(self.busy)
                    .on_click(cx.listener(move |view, _, _, cx| view.operation(Operation::Action(json!({ "action": "publish", "id": publish_id, "revision": revision })), cx))));
            }
            if editor.status == "ACTIVE" {
                let pause_id = id.clone();
                actions = actions.child(
                    Button::new("automation-pause", t!("automations.pause").to_string())
                        .variant(ButtonVariant::Secondary)
                        .disabled(self.busy)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.operation(
                                Operation::Action(json!({ "action": "pause", "id": pause_id })),
                                cx,
                            )
                        })),
                );
            }
            actions = actions.child(
                Button::new("automation-delete", t!("automations.delete").to_string())
                    .variant(ButtonVariant::Ghost)
                    .disabled(self.busy)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.confirm_delete = Some(DeleteTarget::Manage(delete_id.clone()));
                        cx.notify();
                    })),
            );
        }
        let open_id = id.clone();
        body.child(
            actions
                .child(
                    Button::new("automation-detail-test", t!("automations.test").to_string())
                        .variant(ButtonVariant::Secondary)
                        .disabled(self.busy)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.operation(
                                Operation::Action(json!({ "action": "test", "id": id })),
                                cx,
                            )
                        })),
                )
                .child(
                    Button::new(
                        "automation-detail-manage",
                        t!("automations.open_manage").to_string(),
                    )
                    .variant(ButtonVariant::Ghost)
                    .disabled(self.busy)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.operation(
                            Operation::Open("workflows".into(), Some(open_id.clone())),
                            cx,
                        )
                    })),
                ),
        )
    }
}

impl Render for AutomationsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let logical_width =
            window.viewport_size().width.to_f64() / (window.rem_size().to_f64() / 16.0);
        let columns = if logical_width < 900.0 { 1 } else { 2 };
        let editing = self.form.is_some() || self.editor.is_some() || self.local_editor.is_some();
        let mut header = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(12.0))
            .px(px(24.0))
            .py(px(18.0))
            .flex_shrink_0()
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(180.0))
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_size(px(20.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("automations.title").to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(self.site_label.clone()),
                    ),
            );
        if editing {
            header = header.child(
                Button::new("automation-back", t!("automations.back").to_string())
                    .variant(ButtonVariant::Ghost)
                    .disabled(self.busy)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.form = None;
                        view.editor = None;
                        view.local_editor = None;
                        view.error = None;
                        view.refresh(cx);
                        cx.notify();
                    })),
            );
        } else {
            header = header
                .child(
                    Button::new("automation-refresh", t!("automations.refresh").to_string())
                        .variant(ButtonVariant::Ghost)
                        .icon("refresh-cw")
                        .disabled(self.busy || self.context.is_none())
                        .on_click(cx.listener(|view, _, _, cx| view.refresh(cx))),
                )
                .child(
                    Button::new(
                        "automation-new-task",
                        t!("automations.new_task").to_string(),
                    )
                    .debug_selector(|| "automation-new-task".into())
                    .icon("plus")
                    .disabled(self.busy)
                    .on_click(cx.listener(|view, _, _, cx| view.start_task(None, cx))),
                );
        }
        let mut content = div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .min_h(px(0.0))
            .bg(ShellDeckColors::bg_primary())
            .text_color(ShellDeckColors::text_primary())
            .child(header);
        if !editing {
            let mut tabs = div()
                .flex()
                .flex_wrap()
                .gap(px(8.0))
                .px(px(24.0))
                .py(px(12.0))
                .flex_shrink_0();
            for (index, tab, key) in [
                (0, Tab::Tasks, "automations.tasks"),
                (1, Tab::Workflows, "automations.workflows"),
                (2, Tab::Runs, "automations.runs"),
            ] {
                tabs = tabs.child(
                    Button::new(("automation-tab", index as usize), t!(key).to_string())
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .selected(self.tab == tab)
                        .disabled(self.busy)
                        .on_click(cx.listener(move |view, _, _, cx| view.change_tab(tab, cx))),
                );
            }
            tabs = tabs.child(
                Button::new(
                    "automation-manage-templates",
                    t!("automations.manage_templates").to_string(),
                )
                .size(ButtonSize::Sm)
                .variant(ButtonVariant::Ghost)
                .disabled(self.busy || self.context.is_none())
                .on_click(cx.listener(|view, _, _, cx| {
                    view.operation(Operation::Open("templates".into(), None), cx)
                })),
            );
            content =
                content.child(tabs).child(div().px(px(24.0)).child(
                    Input::new(&self.search).placeholder(t!("automations.search").to_string()),
                ));
        }
        content = content.child(div().px(px(24.0)).py(px(8.0)).child(self.render_error(cx)));
        let body = if self.form.is_some() {
            self.render_form(columns, cx)
        } else if self.local_editor.is_some() {
            self.render_local_editor(cx)
        } else if self.editor.is_some() {
            self.render_editor(cx)
        } else {
            self.render_list(columns, cx)
        };
        content = content.child(
            div()
                .id("automations-scroll")
                .debug_selector(|| "automations-scroll".into())
                .flex()
                .flex_col()
                .items_center()
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .p(px(24.0))
                .when(editing, |style| style.pb(px(64.0)))
                .child(body.flex_shrink_0()),
        );
        if let Some(id) = self.confirm_delete.clone() {
            content = content.child(
                Dialog::new()
                    .header(
                        div()
                            .text_size(px(18.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("automations.delete").to_string()),
                    )
                    .content(
                        div().text_size(px(13.0)).child(
                            t!(if matches!(id, DeleteTarget::Local(_)) {
                                "automations.local_delete_confirm"
                            } else {
                                "automations.delete_confirm"
                            })
                            .to_string(),
                        ),
                    )
                    .footer(
                        div()
                            .flex()
                            .gap(px(8.0))
                            .child(
                                Button::new(
                                    "automation-delete-cancel",
                                    t!("automations.cancel").to_string(),
                                )
                                .variant(ButtonVariant::Ghost)
                                .on_click(cx.listener(
                                    |view, _, _, cx| {
                                        view.confirm_delete = None;
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                Button::new(
                                    "automation-delete-confirm",
                                    t!("automations.delete").to_string(),
                                )
                                .variant(ButtonVariant::Destructive)
                                .on_click(cx.listener(
                                    move |view, _, _, cx| {
                                        view.confirm_delete = None;
                                        match &id {
                                            DeleteTarget::Manage(id) => view.operation(
                                                Operation::Action(
                                                    json!({ "action": "delete", "id": id }),
                                                ),
                                                cx,
                                            ),
                                            DeleteTarget::Local(id) => view.local_command(
                                                *id,
                                                crate::agent_console_view::LocalTaskCommand::Delete,
                                                cx,
                                            ),
                                        }
                                    },
                                )),
                            ),
                    ),
            );
        }
        content
    }
}
