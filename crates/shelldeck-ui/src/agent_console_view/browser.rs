//! Explicit browser snapshots, rendered-DOM inspection and visual feedback.
//! The shell uses the shared image lightbox and screenshot annotator; there is
//! deliberately no claim that a snapshot accepts live page interactions.
use super::*;
use crate::attachment_annotator::AttachmentAnnotator;
use crate::issue_attachments::{AttachmentDraft, AttachmentLightbox, LightboxItem};
use shelldeck_core::agent_browser::{
    browser_context_data, capture_browser_preview, validate_browser_url, BrowserCapture,
    BrowserCaptureRequest, BrowserHistory,
};
use std::sync::Arc;

const MAX_RETAINED_CAPTURES: usize = 8;

#[derive(Clone, Copy)]
enum BrowserNavigation {
    Visit,
    Back,
    Forward,
    Reload,
}

struct BrowserPageUi {
    url: Entity<InputState>,
    filter: Entity<InputState>,
    note: Entity<InputState>,
    history: BrowserHistory,
    capture: Option<Arc<BrowserCapture>>,
    screenshot: Option<AttachmentDraft>,
    dom_lines: Vec<String>,
    mobile: bool,
    show_dom: bool,
    annotated: bool,
    loading: bool,
    request_id: Uuid,
    error: Option<String>,
}

impl BrowserPageUi {
    fn new(cx: &mut Context<AgentConsoleView>) -> Self {
        Self {
            url: input_state(cx, String::new()),
            filter: input_state(cx, String::new()),
            note: cx.new(|cx| InputState::new(cx).multi_line(true)),
            history: BrowserHistory::default(),
            capture: None,
            screenshot: None,
            dom_lines: Vec::new(),
            mobile: false,
            show_dom: false,
            annotated: false,
            loading: false,
            request_id: Uuid::new_v4(),
            error: None,
        }
    }
}

pub(super) struct AgentBrowserUi {
    pages: HashMap<Uuid, BrowserPageUi>,
    lightbox: Option<Entity<AttachmentLightbox>>,
    annotator: Option<Entity<AttachmentAnnotator>>,
}

impl AgentBrowserUi {
    pub(super) fn new(_cx: &mut Context<AgentConsoleView>) -> Self {
        Self {
            pages: HashMap::new(),
            lightbox: None,
            annotator: None,
        }
    }
}

impl AgentConsoleView {
    pub(super) fn show_agent_browser(
        &mut self,
        url: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.sessions.selected_id() else {
            return;
        };
        self.browser
            .pages
            .retain(|id, _| self.sessions.get(*id).is_some());
        self.browser
            .pages
            .entry(id)
            .or_insert_with(|| BrowserPageUi::new(cx));
        self.tools.active = Some(tools::AgentToolSurface::Browser);
        self.tools.panel_visible = true;
        self.tools.panel_width = self.tools.panel_width.max(440.0);
        if let Some(url) = url {
            if let Some(page) = self.browser.pages.get(&id) {
                page.url
                    .update(cx, |state, cx| state.set_value(url, window, cx));
            }
            self.capture_agent_browser(BrowserNavigation::Visit, cx);
        }
        cx.notify();
    }

    fn capture_agent_browser(&mut self, navigation: BrowserNavigation, cx: &mut Context<Self>) {
        let Some(id) = self.sessions.selected_id() else {
            return;
        };
        let Some(page) = self.browser.pages.get_mut(&id) else {
            return;
        };
        if page.loading {
            return;
        }
        let raw = match navigation {
            BrowserNavigation::Visit => Some(page.url.read(cx).content().trim().to_string()),
            BrowserNavigation::Back => page.history.back_url().map(str::to_owned),
            BrowserNavigation::Forward => page.history.forward_url().map(str::to_owned),
            BrowserNavigation::Reload => page.history.current().map(str::to_owned),
        };
        let Some(raw) = raw else {
            return;
        };
        let url = match validate_browser_url(&raw) {
            Ok(url) => url,
            Err(error) => {
                page.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        page.loading = true;
        page.error = None;
        let request_id = Uuid::new_v4();
        page.request_id = request_id;
        let request = BrowserCaptureRequest {
            url,
            width: if page.mobile { 412 } else { 1280 },
            height: if page.mobile { 892 } else { 900 },
        };
        let task = cx
            .background_executor()
            .spawn(async move { capture_browser_preview(request) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let Some(page) = this.browser.pages.get_mut(&id) else {
                    return;
                };
                if page.request_id != request_id || this.sessions.get(id).is_none() {
                    return;
                }
                page.loading = false;
                match result {
                    Ok(capture) => {
                        // Validate/decode in the existing attachment pipeline,
                        // then retain only a bounded set of page captures.
                        match AttachmentDraft::from_bytes(
                            "browser-capture.png",
                            capture.png.clone(),
                        ) {
                            Ok(screenshot) => {
                                match navigation {
                                    BrowserNavigation::Visit => {
                                        page.history.visit(capture.requested_url.clone())
                                    }
                                    BrowserNavigation::Back => {
                                        page.history.back();
                                    }
                                    BrowserNavigation::Forward => {
                                        page.history.forward();
                                    }
                                    BrowserNavigation::Reload => {}
                                }
                                // New entities avoid retaining stale selection
                                // ranges after a redirect/back navigation.
                                page.url = input_state(cx, capture.requested_url.clone());
                                page.dom_lines = dom_display_lines(&capture.dom);
                                page.capture = Some(Arc::new(capture));
                                page.screenshot = Some(screenshot);
                                page.annotated = false;
                            }
                            Err(error) => page.error = Some(error),
                        }
                    }
                    Err(error) => page.error = Some(error.to_string()),
                }
                this.trim_browser_captures(id);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn trim_browser_captures(&mut self, keep: Uuid) {
        let mut captures: Vec<_> = self
            .browser
            .pages
            .iter()
            .filter_map(|(id, page)| {
                page.capture
                    .as_ref()
                    .map(|capture| (*id, capture.captured_at_ms))
            })
            .collect();
        captures.sort_by_key(|(_, at)| *at);
        let count = captures.len().saturating_sub(MAX_RETAINED_CAPTURES);
        for (id, _) in captures
            .into_iter()
            .filter(|(id, _)| *id != keep)
            .take(count)
        {
            if let Some(page) = self.browser.pages.get_mut(&id) {
                page.capture = None;
                page.screenshot = None;
                page.dom_lines.clear();
            }
        }
    }

    fn append_browser_context(
        &mut self,
        include_dom: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.sessions.selected_id() else {
            return;
        };
        let Some(page) = self.browser.pages.get(&id) else {
            return;
        };
        let Some(capture) = &page.capture else {
            return;
        };
        let note = page.note.read(cx).content().to_string();
        let observation = browser_context_data(capture, &note, include_dom);
        let Some(ui) = self.session_ui.get(&id) else {
            return;
        };
        let current = ui.prompt.read(cx).content().to_string();
        let next = format!(
            "{current}\n\n{}\n{observation}",
            t!("agents.composer.context_data")
        );
        if next.len() > MAX_AGENT_PROMPT_BYTES {
            if let Some(page) = self.browser.pages.get_mut(&id) {
                page.error = Some(t!("agents.error.prompt_too_long").to_string());
            }
            cx.notify();
            return;
        }
        ui.prompt.update(cx, |input, cx| {
            input.set_value(next.trim_start().to_string(), window, cx)
        });
        self.schedule_library_save(cx);
        cx.notify();
    }

    fn annotate_browser_capture(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.sessions.selected_id() else {
            return;
        };
        let Some(page) = self.browser.pages.get(&id) else {
            return;
        };
        let Some(draft) = page.screenshot.clone() else {
            return;
        };
        let request_id = page.request_id;
        let cancel = cx.entity().downgrade();
        let apply = cancel.clone();
        self.browser.annotator = Some(cx.new(|cx| {
            AttachmentAnnotator::new(
                draft,
                move |cx| {
                    if let Some(parent) = cancel.upgrade() {
                        parent.update(cx, |view, cx| {
                            view.browser.annotator = None;
                            cx.notify();
                        });
                    }
                },
                move |draft, cx| {
                    if let Some(parent) = apply.upgrade() {
                        parent.update(cx, |view, cx| {
                            view.browser.annotator = None;
                            if let Some(page) = view.browser.pages.get_mut(&id) {
                                if page.request_id == request_id {
                                    page.screenshot = Some(draft);
                                    page.annotated = true;
                                }
                            }
                            cx.notify();
                        });
                    }
                },
                cx,
            )
        }));
        cx.notify();
    }

    fn inspect_browser_capture(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.sessions.selected_id() else {
            return;
        };
        let Some(page) = self.browser.pages.get(&id) else {
            return;
        };
        let Some(draft) = &page.screenshot else {
            return;
        };
        let item = LightboxItem::from_image(
            "browser-capture",
            t!("agents.browser.screenshot").to_string(),
            draft.image.clone(),
        );
        let close = cx.entity().downgrade();
        self.browser.lightbox = Some(cx.new(|cx| {
            AttachmentLightbox::new(
                vec![item],
                0,
                move |cx| {
                    if let Some(parent) = close.upgrade() {
                        parent.update(cx, |view, cx| {
                            view.browser.lightbox = None;
                            cx.notify();
                        });
                    }
                },
                cx,
            )
        }));
        cx.notify();
    }

    pub(super) fn render_browser_overlay(&self) -> Option<AnyElement> {
        if let Some(annotator) = &self.browser.annotator {
            return Some(annotator.clone().into_any_element());
        }
        self.browser
            .lightbox
            .as_ref()
            .map(|lightbox| lightbox.clone().into_any_element())
    }

    pub(super) fn render_browser_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut shell = div()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .text_size(px(11.0))
            .text_color(ShellDeckColors::text_primary());
        shell = shell.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .p(px(8.0))
                .border_b_1()
                .border_color(ShellDeckColors::border())
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(t!("agents.browser.title").to_string()),
                )
                .child(
                    Button::new("agent-browser-panel-more", "")
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Icon)
                        .icon(IconSource::from("plus"))
                        .tooltip(t!("agents.parity.widen_panel").to_string())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.tools.panel_width = (this.tools.panel_width + 120.0).min(1000.0);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("agent-browser-panel-close", "")
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Icon)
                        .icon(IconSource::from("x"))
                        .tooltip(t!("agents.parity.close_panel").to_string())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.tools.panel_visible = false;
                            cx.notify();
                        })),
                ),
        );
        let Some(id) = self.sessions.selected_id() else {
            return shell.into_any_element();
        };
        let Some(page) = self.browser.pages.get(&id) else {
            return shell
                .child(
                    div()
                        .p(px(12.0))
                        .child(t!("agents.browser.description").to_string()),
                )
                .child(
                    Button::new(
                        "agent-browser-start",
                        t!("agents.browser.start").to_string(),
                    )
                    .variant(ButtonVariant::Outline)
                    .size(ButtonSize::Sm)
                    .on_click(
                        cx.listener(|this, _, window, cx| {
                            this.show_agent_browser(None, window, cx)
                        }),
                    ),
                )
                .into_any_element();
        };
        let loading = page.loading;
        let selected_mobile = page.mobile;
        let selected_dom = page.show_dom;
        let parent = cx.entity().downgrade();
        let url_input = Input::new(&page.url)
            .size(InputSize::Sm)
            .variant(InputVariant::Outline)
            .placeholder(t!("agents.browser.url_placeholder").to_string())
            .disabled(loading)
            .on_enter(move |_, cx| {
                if let Some(parent) = parent.upgrade() {
                    parent.update(cx, |view, cx| {
                        view.capture_agent_browser(BrowserNavigation::Visit, cx)
                    });
                }
            });
        shell = shell.child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .p(px(8.0))
                .border_b_1()
                .border_color(ShellDeckColors::border())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .child(div().flex_1().min_w_0().child(url_input))
                        .child(
                            Button::new(
                                "agent-browser-capture",
                                t!("agents.browser.capture").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .disabled(loading)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.capture_agent_browser(BrowserNavigation::Visit, cx)
                            })),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(4.0))
                        .child(
                            Button::new(
                                "agent-browser-back",
                                t!("agents.browser.back").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .disabled(loading || page.history.back_url().is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.capture_agent_browser(BrowserNavigation::Back, cx)
                            })),
                        )
                        .child(
                            Button::new(
                                "agent-browser-forward",
                                t!("agents.browser.forward").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .disabled(loading || page.history.forward_url().is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.capture_agent_browser(BrowserNavigation::Forward, cx)
                            })),
                        )
                        .child(
                            Button::new(
                                "agent-browser-reload",
                                t!("agents.browser.reload").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .disabled(loading || page.history.current().is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.capture_agent_browser(BrowserNavigation::Reload, cx)
                            })),
                        )
                        .child(
                            Button::new(
                                "agent-browser-desktop",
                                t!("agents.browser.viewport_desktop").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(if selected_mobile {
                                ButtonVariant::Ghost
                            } else {
                                ButtonVariant::Secondary
                            })
                            .disabled(loading)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if let Some(page) = this.browser.pages.get_mut(&id) {
                                        page.mobile = false;
                                    }
                                    cx.notify();
                                },
                            )),
                        )
                        .child(
                            Button::new(
                                "agent-browser-mobile",
                                t!("agents.browser.viewport_mobile").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(if selected_mobile {
                                ButtonVariant::Secondary
                            } else {
                                ButtonVariant::Ghost
                            })
                            .disabled(loading)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if let Some(page) = this.browser.pages.get_mut(&id) {
                                        page.mobile = true;
                                    }
                                    cx.notify();
                                },
                            )),
                        ),
                )
                .child(
                    div()
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("agents.browser.description").to_string()),
                ),
        );
        let mut body = div()
            .id("agent-browser-scroll")
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(10.0))
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll();
        if loading {
            body = body.child(
                div()
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("agents.browser.loading").to_string()),
            );
        }
        if let Some(error) = &page.error {
            body = body.child(
                div()
                    .text_color(ShellDeckColors::error())
                    .child(error.clone()),
            );
        }
        if let Some(capture) = &page.capture {
            body = body.child(
                div()
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!(
                        "{} · {} × {} · {}",
                        crate::i18n::rel_time(capture.captured_at_ms as f64),
                        capture.width,
                        capture.height,
                        capture.browser_name
                    )),
            );
            let desired_width = if page.mobile { 412 } else { 1280 };
            if capture.width != desired_width {
                body = body.child(
                    div()
                        .text_color(ShellDeckColors::warning())
                        .child(t!("agents.browser.stale").to_string()),
                );
            }
            body = body.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(4.0))
                    .child(
                        Button::new(
                            "agent-browser-image-tab",
                            t!("agents.browser.screenshot").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(if selected_dom {
                            ButtonVariant::Ghost
                        } else {
                            ButtonVariant::Secondary
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(page) = this.browser.pages.get_mut(&id) {
                                page.show_dom = false;
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        Button::new(
                            "agent-browser-dom-tab",
                            t!("agents.browser.dom").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(if selected_dom {
                            ButtonVariant::Secondary
                        } else {
                            ButtonVariant::Ghost
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(page) = this.browser.pages.get_mut(&id) {
                                page.show_dom = true;
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        Button::new(
                            "agent-browser-external",
                            t!("agents.browser.open_external").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(capture) = this
                                .browser
                                .pages
                                .get(&id)
                                .and_then(|page| page.capture.as_ref())
                            {
                                cx.open_url(&capture.requested_url);
                            }
                        })),
                    ),
            );
            if selected_dom {
                let parent = cx.entity().downgrade();
                body = body.child(
                    Input::new(&page.filter)
                        .size(InputSize::Sm)
                        .variant(InputVariant::Outline)
                        .placeholder(t!("agents.browser.find_dom").to_string())
                        .on_change(move |_, cx| {
                            if let Some(parent) = parent.upgrade() {
                                parent.update(cx, |_, cx| cx.notify());
                            }
                        }),
                );
                let query = page.filter.read(cx).content().to_lowercase();
                let mut code = div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .font_family("JetBrains Mono")
                    .text_size(px(10.0))
                    .p(px(8.0))
                    .bg(ShellDeckColors::bg_primary())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .overflow_hidden();
                let mut matches = 0;
                for (index, line) in page
                    .dom_lines
                    .iter()
                    .enumerate()
                    .filter(|(_, line)| query.is_empty() || line.to_lowercase().contains(&query))
                    .take(300)
                {
                    code = code.child(
                        div()
                            .flex()
                            .gap(px(6.0))
                            .min_w_0()
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_color(ShellDeckColors::text_muted())
                                    .child((index + 1).to_string()),
                            )
                            .child(div().flex_1().min_w_0().child(line.clone())),
                    );
                    matches += 1;
                }
                if matches == 0 {
                    code = code.child(t!("agents.browser.no_match").to_string());
                }
                body = body
                    .child(
                        div()
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("agents.browser.dom_limit").to_string()),
                    )
                    .child(
                        Button::new(
                            "agent-browser-copy-dom",
                            t!("agents.browser.copy_dom").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(capture) = this
                                .browser
                                .pages
                                .get(&id)
                                .and_then(|page| page.capture.as_ref())
                            {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    capture.dom.clone(),
                                ));
                            }
                        })),
                    )
                    .child(code);
            } else if let Some(draft) = &page.screenshot {
                // adabraka has no page-snapshot component; reuse the same
                // GPUI image primitive and full-window lightbox as attachments.
                body = body.child(
                    div()
                        .id("agent-browser-screenshot")
                        .w_full()
                        .min_w_0()
                        .cursor_pointer()
                        .child(
                            img(draft.image.clone())
                                .w_full()
                                .object_fit(ObjectFit::Contain),
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.inspect_browser_capture(cx))),
                );
                if page.annotated {
                    body = body.child(
                        div()
                            .text_color(ShellDeckColors::success())
                            .child(t!("agents.browser.annotated_ready").to_string()),
                    );
                }
                body = body.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(4.0))
                        .child(
                            Button::new(
                                "agent-browser-inspect",
                                t!("agents.browser.inspect_image").to_string(),
                            )
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .on_click(
                                cx.listener(|this, _, _, cx| this.inspect_browser_capture(cx)),
                            ),
                        )
                        .child(
                            Button::new(
                                "agent-browser-annotate",
                                t!("agents.browser.annotate").to_string(),
                            )
                            .variant(ButtonVariant::Outline)
                            .size(ButtonSize::Sm)
                            .disabled(loading)
                            .on_click(
                                cx.listener(|this, _, _, cx| this.annotate_browser_capture(cx)),
                            ),
                        ),
                );
            }
        } else if !loading {
            body = body.child(
                div()
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("agents.browser.no_capture").to_string()),
            );
        }
        shell = shell.child(body);
        if page.capture.is_some() {
            let context = self.displayed_context(cx);
            let image_supported = context.target == AgentTarget::Local
                && context.provider.capabilities().image_attachments;
            let footer = div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .p(px(10.0))
                .flex_shrink_0()
                .border_t_1()
                .border_color(ShellDeckColors::border())
                .child(
                    Input::new(&page.note)
                        .size(InputSize::Sm)
                        .variant(InputVariant::Outline)
                        .multi_line(true)
                        .min_rows(2)
                        .max_rows(3)
                        .placeholder(t!("agents.browser.note_placeholder").to_string()),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(4.0))
                        .child(
                            Button::new(
                                "agent-browser-add-note",
                                t!("agents.browser.add_note").to_string(),
                            )
                            .variant(ButtonVariant::Outline)
                            .size(ButtonSize::Sm)
                            .disabled(loading)
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.append_browser_context(false, window, cx)
                                },
                            )),
                        )
                        .child(
                            Button::new(
                                "agent-browser-add-dom",
                                t!("agents.browser.add_dom").to_string(),
                            )
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .disabled(loading)
                            .on_click(cx.listener(
                                |this, _, window, cx| this.append_browser_context(true, window, cx),
                            )),
                        )
                        .child(
                            Button::new(
                                "agent-browser-add-image",
                                t!("agents.browser.add_image").to_string(),
                            )
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .disabled(loading || !image_supported)
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    if this.sessions.selected_id() != Some(id) {
                                        return;
                                    }
                                    if let Some(draft) = this
                                        .browser
                                        .pages
                                        .get(&id)
                                        .and_then(|page| page.screenshot.clone())
                                    {
                                        this.append_browser_context(false, window, cx);
                                        this.stage_agent_image(id, draft, cx);
                                    }
                                },
                            )),
                        ),
                );
            shell = shell.child(footer);
            if !image_supported {
                shell = shell.child(
                    div()
                        .px(px(10.0))
                        .pb(px(8.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("agents.browser.image_unsupported").to_string()),
                );
            }
        }
        shell.into_any_element()
    }
}

/// Format on capture, not every paint. The DOM inspector intentionally shows a
/// bounded excerpt; copying and composer context use the original bounded DOM.
fn dom_display_lines(dom: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut count = 0;
    let mut previous = '\0';
    for character in dom.chars() {
        if character == '\n' || count >= 180 || (previous == '>' && character == '<') {
            lines.push(std::mem::take(&mut line));
            count = 0;
            if lines.len() >= 2000 {
                break;
            }
            if character == '\n' {
                previous = character;
                continue;
            }
        }
        line.push(character);
        count += 1;
        previous = character;
    }
    if !line.is_empty() && lines.len() < 2000 {
        lines.push(line);
    }
    lines
}
