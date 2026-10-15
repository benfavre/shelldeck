//! Browser commands remain scoped to the browser subtree when its address,
//! find field, or Chromium viewport owns focus. Raw key handlers run too late
//! to override Input or application keymap actions in GPUI.
use super::*;

gpui::actions!(
    native_browser,
    [
        BrowserFocusAddress,
        BrowserNewTab,
        BrowserCloseTab,
        BrowserReload,
        BrowserHardReload,
        BrowserFind,
        BrowserNextTab,
        BrowserPreviousTab,
        BrowserBack,
        BrowserForward,
        BrowserEscape,
        BrowserAcceptDialog,
    ]
);

struct BrowserBindings;
impl Global for BrowserBindings {}

pub(super) fn init(cx: &mut App) {
    if cx.has_global::<BrowserBindings>() {
        return;
    }
    cx.set_global(BrowserBindings);
    let primary = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let mut bindings = vec![
        KeyBinding::new(
            &format!("{primary}-l"),
            BrowserFocusAddress,
            Some("NativeBrowserChrome"),
        ),
        KeyBinding::new(
            &format!("{primary}-t"),
            BrowserNewTab,
            Some("NativeBrowserChrome"),
        ),
        KeyBinding::new(
            &format!("{primary}-w"),
            BrowserCloseTab,
            Some("NativeBrowserChrome"),
        ),
        KeyBinding::new(
            &format!("{primary}-r"),
            BrowserReload,
            Some("NativeBrowserChrome"),
        ),
        KeyBinding::new(
            &format!("{primary}-shift-r"),
            BrowserHardReload,
            Some("NativeBrowserChrome"),
        ),
        KeyBinding::new(
            &format!("{primary}-f"),
            BrowserFind,
            Some("NativeBrowserChrome"),
        ),
        KeyBinding::new("ctrl-tab", BrowserNextTab, Some("NativeBrowserChrome")),
        KeyBinding::new(
            "ctrl-shift-tab",
            BrowserPreviousTab,
            Some("NativeBrowserChrome"),
        ),
        KeyBinding::new("alt-left", BrowserBack, Some("NativeBrowserChrome")),
        KeyBinding::new("alt-right", BrowserForward, Some("NativeBrowserChrome")),
        KeyBinding::new("escape", BrowserEscape, Some("NativeBrowserChrome")),
        KeyBinding::new("enter", BrowserAcceptDialog, Some("NativeBrowserDialog")),
    ];
    if !cfg!(target_os = "macos") {
        bindings.push(KeyBinding::new(
            "f5",
            BrowserReload,
            Some("NativeBrowserChrome"),
        ));
    }
    cx.bind_keys(bindings);
}

pub(super) fn handlers(root: Stateful<Div>, cx: &mut Context<NativeBrowserView>) -> Stateful<Div> {
    root.on_action(cx.listener(|view, _: &BrowserFocusAddress, window, cx| {
        view.focus_address(window, cx);
        cx.stop_propagation();
    }))
    .on_action(cx.listener(|view, _: &BrowserNewTab, window, cx| {
        view.new_tab("about:blank".into(), cx);
        view.focus_address(window, cx);
        cx.stop_propagation();
    }))
    .on_action(cx.listener(|view, _: &BrowserCloseTab, _, cx| {
        if let Some(browser_id) = view.active {
            view.close_tab(browser_id, cx);
        }
        cx.stop_propagation();
    }))
    .on_action(cx.listener(|view, _: &BrowserReload, _, cx| {
        if let Some(browser_id) = view.active {
            view.send(
                Command::Reload {
                    browser_id,
                    ignore_cache: false,
                },
                cx,
            );
        }
        cx.stop_propagation();
    }))
    .on_action(cx.listener(|view, _: &BrowserHardReload, _, cx| {
        if let Some(browser_id) = view.active {
            view.send(
                Command::Reload {
                    browser_id,
                    ignore_cache: true,
                },
                cx,
            );
        }
        cx.stop_propagation();
    }))
    .on_action(cx.listener(|view, _: &BrowserFind, window, cx| {
        view.find_visible = true;
        view.find_input.read(cx).focus_handle(cx).focus(window);
        cx.notify();
        cx.stop_propagation();
    }))
    .on_action(cx.listener(|view, _: &BrowserNextTab, _, cx| {
        view.cycle_tab(false, cx);
        cx.stop_propagation();
    }))
    .on_action(cx.listener(|view, _: &BrowserPreviousTab, _, cx| {
        view.cycle_tab(true, cx);
        cx.stop_propagation();
    }))
    .on_action(cx.listener(|view, _: &BrowserBack, _, cx| {
        if let Some(browser_id) = view.active {
            view.send(Command::Back { browser_id }, cx);
        }
        cx.stop_propagation();
    }))
    .on_action(cx.listener(|view, _: &BrowserForward, _, cx| {
        if let Some(browser_id) = view.active {
            view.send(Command::Forward { browser_id }, cx);
        }
        cx.stop_propagation();
    }))
    .on_action(cx.listener(|view, _: &BrowserEscape, window, cx| {
        if view.context_menu.take().is_some() {
            cx.notify();
        } else if view.page_dialog.is_some() {
            view.resolve_dialog(false, cx);
        } else if view.find_visible {
            view.close_find(window, cx);
        } else if let Some(browser_id) = view.active {
            view.send(Command::Stop { browser_id }, cx);
        }
        cx.stop_propagation();
    }))
    .on_action(cx.listener(|view, _: &BrowserAcceptDialog, _, cx| {
        view.resolve_dialog(true, cx);
        cx.stop_propagation();
    }))
}

impl NativeBrowserView {
    fn cycle_tab(&mut self, reverse: bool, cx: &mut Context<Self>) {
        let Some(index) = self
            .tabs
            .iter()
            .position(|tab| Some(tab.state.browser_id) == self.active)
        else {
            return;
        };
        let next = if reverse {
            (index + self.tabs.len() - 1) % self.tabs.len()
        } else {
            (index + 1) % self.tabs.len()
        };
        self.select_tab(self.tabs[next].state.browser_id, cx);
    }
}
