//! The opt-in full Chrome preview owns its browser chrome and password dialogs.
use super::*;

impl NativeBrowserView {
    /// Start a persistent app-owned Chrome profile. This never attaches to or
    /// copies the user's regular desktop Chrome profile.
    pub fn use_chrome_preview(&mut self, cx: &mut Context<Self>) {
        self.switch_engine(true, cx);
    }

    pub(super) fn switch_engine(&mut self, chrome: bool, cx: &mut Context<Self>) {
        if self.chrome_wrapper == chrome {
            return;
        }
        if chrome && !cfg!(target_os = "linux") {
            self.set_error(t!("native_browser.chrome_requirements").to_string(), cx);
            return;
        }
        let url = self
            .selected_tab()
            .map(|tab| tab.state.url.clone())
            .unwrap_or_else(|| "about:blank".into());
        self.close_all(cx);
        self.chrome_wrapper = chrome;
        self.copy_requested = false;
        self.engine_version = None;
        self.pending_urls.push(url);
        self.ensure_engine(cx);
        cx.notify();
    }

    pub(super) fn chrome_shortcut(&mut self, key: i32, modifiers: u32, cx: &mut Context<Self>) {
        if let Some(browser_id) = self.active {
            for (kind, modifiers) in [
                (shelldeck_browser::KeyEventKind::RawKeyDown, modifiers),
                (shelldeck_browser::KeyEventKind::KeyUp, 0),
            ] {
                self.send(
                    Command::Key {
                        browser_id,
                        event: shelldeck_browser::KeyEvent {
                            kind,
                            windows_key_code: key,
                            native_key_code: 0,
                            modifiers,
                            character: None,
                            unmodified_character: None,
                            is_system_key: false,
                        },
                    },
                    cx,
                );
            }
            self.request_focus(cx);
        }
    }

    pub(super) fn chrome_toolbar(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id("native-browser-chrome-toolbar")
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(8.0))
            .py(px(5.0))
            .flex_shrink_0()
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                Button::new(
                    "native-browser-use-chromium",
                    t!("native_browser.use_chromium").to_string(),
                )
                .debug_selector(|| "native-browser-use-chromium".into())
                .icon("chevron-left")
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Sm)
                .on_click(cx.listener(|view, _, _, cx| view.switch_engine(false, cx))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.0))
                    .child(t!("native_browser.chrome_preview").to_string()),
            )
            .child(
                Button::new("native-browser-chrome-tab", "")
                    .icon("plus")
                    .tooltip(t!("native_browser.new_tab").to_string())
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .on_click(cx.listener(|view, _, _, cx| {
                        if view.active.is_some() {
                            view.chrome_shortcut(84, shelldeck_browser::modifiers::CONTROL, cx);
                        } else {
                            view.new_tab("about:blank".into(), cx);
                        }
                    })),
            )
            .child(
                Button::new("native-browser-close-pane", "")
                    .icon("x")
                    .tooltip(t!("agents.live_browser.close").to_string())
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .on_click(
                        cx.listener(|_, _, _, cx| cx.emit(NativeBrowserEvent::CloseRequested)),
                    ),
            )
    }
}
