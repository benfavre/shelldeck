use super::*;
use crate::native_browser::NativeBrowserView;
use shelldeck_core::config::pro_web::{normalize_url, profile_scope};

impl Workspace {
    pub(super) fn reconcile_pro_account(&mut self, cx: &mut Context<Self>) {
        let expected = self.app_config.account.as_ref().and_then(|account| {
            if account.email.trim().is_empty() {
                return None;
            }
            normalize_url(&self.app_config.general.pro_web_url)
                .ok()
                .map(|url| profile_scope(&self.account_base_url(), &account.email, &url))
        });
        if !self.signed_in()
            || self
                .pro_browser
                .as_ref()
                .is_some_and(|browser| expected.as_deref() != Some(browser.read(cx).scope()))
        {
            self.close_pro_session(cx);
        }
    }

    pub(super) fn close_pro_session(&mut self, cx: &mut Context<Self>) {
        if let Some(browser) = self.pro_browser.take() {
            browser.update(cx, |browser, cx| browser.close_all(cx));
        }
        self.pro_open = false;
        self.pro_url_input = None;
        self.pro_error = false;
    }

    fn connect_pro(&mut self, cx: &mut Context<Self>) {
        if !self.signed_in() {
            return;
        }
        let raw = self
            .pro_url_input
            .as_ref()
            .map(|input| input.read(cx).content().to_string())
            .unwrap_or_else(|| self.app_config.general.pro_web_url.clone());
        let Ok(url) = normalize_url(&raw) else {
            self.pro_error = true;
            cx.notify();
            return;
        };
        let Some(account) = self.app_config.account.as_ref() else {
            return;
        };
        // A missing account identity would otherwise share one retained CEF
        // profile between unrelated legacy sessions.
        if account.email.trim().is_empty() {
            self.pro_error = true;
            cx.notify();
            return;
        }
        let scope = profile_scope(&self.account_base_url(), &account.email, &url);
        if let Some(previous) = self.pro_browser.take() {
            previous.update(cx, |view, cx| view.close_all(cx));
        }
        let browser = cx.new(|cx| {
            let mut view = NativeBrowserView::new_workspace(scope, cx);
            view.navigate(url.clone(), cx);
            view
        });
        self.pro_browser = Some(browser);
        self.pro_error = false;
        self.app_config.general.pro_web_url = url;
        if self.app_config.save().is_err() {
            self.show_toast(t!("pro.save_failed").to_string(), ToastLevel::Warning, cx);
        }
        self.sync_settings_config(cx);
        cx.notify();
    }

    pub(super) fn toggle_pro(&mut self, cx: &mut Context<Self>) {
        if !self.signed_in() {
            return;
        }
        self.close_titlebar_menus();
        self.pro_open = !self.pro_open;
        self.presentation_focus_pending = false;
        if self.pro_open && self.pro_browser.is_none() {
            let restore_saved_address = self.pro_url_input.is_none();
            if restore_saved_address {
                let value = self.app_config.general.pro_web_url.clone();
                self.pro_url_input = Some(cx.new(|cx| {
                    let mut input = InputState::new(cx);
                    input.replace_content(value, cx);
                    input
                }));
            }
            // Returning to an address form must not submit unconfirmed edits.
            if restore_saved_address && !self.app_config.general.pro_web_url.is_empty() {
                self.connect_pro(cx);
            }
        }
        if self.pro_open {
            if let Some(browser) = &self.pro_browser {
                browser.update(cx, |view, cx| view.request_focus(cx));
            }
        } else {
            match self.active_view {
                ActiveView::Agents => self
                    .agent_console
                    .update(cx, |view, cx| view.request_surface_focus(cx)),
                ActiveView::Terminal => self.terminal.update(cx, |view, cx| {
                    view.request_focus();
                    cx.notify();
                }),
                _ => {}
            }
        }
        self.on_active_view_changed(cx);
        cx.notify();
    }

    pub(super) fn render_pro_button(
        &self,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        self.signed_in().then(|| {
            Button::new(
                "open-inklura-pro",
                if self.pro_open && compact {
                    t!("pro.return_short").to_string()
                } else if self.pro_open {
                    t!("pro.return").to_string()
                } else if compact {
                    "Pro".into()
                } else {
                    "Inklura Pro".into()
                },
            )
            .debug_selector(|| "open-inklura-pro".into())
            .size(ButtonSize::Sm)
            .h(px(28.0))
            .px(px(8.0))
            .tooltip(if self.pro_open {
                t!("pro.return").to_string()
            } else {
                "Inklura Pro".into()
            })
            .variant(if self.pro_open {
                ButtonVariant::Default
            } else {
                ButtonVariant::Ghost
            })
            .on_click(cx.listener(|this, _, _, cx| this.toggle_pro(cx)))
            .into_any_element()
        })
    }

    pub(super) fn render_pro_workspace(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(browser) = &self.pro_browser {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .child(
                    Button::new("pro-change-address", t!("pro.change_address").to_string())
                        .debug_selector(|| "pro-change-address".into())
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(browser) = this.pro_browser.take() {
                                browser.update(cx, |view, cx| view.close_all(cx));
                            }
                            this.pro_error = false;
                            cx.notify();
                        })),
                )
                .child(browser.clone())
                .into_any_element();
        }
        let input = self
            .pro_url_input
            .get_or_insert_with(|| cx.new(InputState::new))
            .clone();
        let error = if self.pro_error {
            Some(
                div().text_color(ShellDeckColors::error()).child(
                    t!(if self
                        .app_config
                        .account
                        .as_ref()
                        .is_none_or(|account| account.email.trim().is_empty())
                    {
                        "pro.account_missing"
                    } else {
                        "pro.invalid_url"
                    })
                    .to_string(),
                ),
            )
        } else {
            None
        };
        div()
            .id("pro-setup")
            .debug_selector(|| "pro-setup".into())
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .items_center()
            .px(px(24.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .pt(px(24.0))
                    .w_full()
                    .max_w(px(520.0))
                    .gap(px(16.0))
                    .child(
                        div()
                            .text_xl()
                            .text_color(ShellDeckColors::text_primary())
                            .child("Inklura Pro"),
                    )
                    .child(
                        div()
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("pro.intro").to_string()),
                    )
                    .child(
                        Input::new(&input)
                            .debug_selector("pro-url-input")
                            .aria_label(t!("pro.address").to_string())
                            .placeholder(t!("pro.address").to_string())
                            .on_enter({
                                let workspace = cx.weak_entity();
                                move |_, cx| {
                                    let _ = workspace.update(cx, |this, cx| this.connect_pro(cx));
                                }
                            }),
                    )
                    .children(error)
                    .child(
                        Button::new("connect-inklura-pro", t!("pro.open").to_string())
                            .debug_selector(|| "connect-inklura-pro".into())
                            .on_click(cx.listener(|this, _, _, cx| this.connect_pro(cx))),
                    )
                    // Keep breathing room after the final scrollable control.
                    .child(div().h(px(8.0)).flex_shrink_0()),
            )
            .into_any_element()
    }
}

#[cfg(test)]
#[path = "pro_web_tests.rs"]
mod tests;
