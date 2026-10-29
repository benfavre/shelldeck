//! Read-only remote directory picker. A loaded canonical listing is selection authority.
use adabraka_ui::components::input::{Input, InputSize};
use adabraka_ui::components::input_state::InputState;
use adabraka_ui::prelude::{Alert, AlertVariant, Button, ButtonSize, ButtonVariant};
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::config::workspace_catalog::RemotePosixPath;
use shelldeck_core::models::connection::Connection;
use shelldeck_ssh::agent_files::Directory;
use shelldeck_ssh::directory_browser::DirectoryBrowser;
use std::ops::Range;

use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RemoteFolderPickerEvent {
    Selected(String),
    Cancelled,
}

#[derive(Default)]
struct ListingState {
    generation: u64,
    pending: bool,
    closed: bool,
    requested: Option<String>,
    current: Option<Directory>,
    error: Option<String>,
}

impl ListingState {
    fn begin(&mut self, path: Option<String>) -> Option<u64> {
        if self.closed {
            return None;
        }
        self.generation = self.generation.checked_add(1)?;
        self.pending = true;
        self.requested = path;
        self.error = None;
        Some(self.generation)
    }

    fn finish(&mut self, generation: u64, result: Result<Directory, String>) -> bool {
        if self.closed || !self.pending || generation != self.generation {
            return false;
        }
        self.pending = false;
        match result {
            Ok(mut directory) if valid_remote_path(&directory.path) => {
                // Folder-only rendering also makes entry clicks an exact allowlist.
                directory.entries.retain(|entry| entry.is_dir);
                directory.entries.sort_by(|a, b| {
                    a.name
                        .to_lowercase()
                        .cmp(&b.name.to_lowercase())
                        .then_with(|| a.path.cmp(&b.path))
                });
                self.current = Some(directory);
                self.error = None;
            }
            Ok(_) => self.error = Some(t!("workspaces.remote_picker.invalid_path").to_string()),
            Err(error) => self.error = Some(error),
        }
        true
    }

    fn accepts_action(&self, generation: u64) -> bool {
        !self.closed && !self.pending && self.generation == generation
    }

    fn selected_path(&self, generation: u64) -> Option<String> {
        (self.accepts_action(generation) && self.error.is_none())
            .then(|| {
                self.current
                    .as_ref()
                    .map(|directory| directory.path.clone())
            })
            .flatten()
    }

    fn selected_input_path(&self, generation: u64, input: &str) -> Option<String> {
        self.selected_path(generation).filter(|path| path == input)
    }

    fn accepts_entry(&self, generation: u64, parent: &str, path: &str) -> bool {
        self.selected_path(generation).as_deref() == Some(parent)
            && self.current.as_ref().is_some_and(|directory| {
                directory
                    .entries
                    .iter()
                    .any(|entry| entry.is_dir && entry.path == path)
            })
    }

    fn close(&mut self) {
        self.closed = true;
        self.pending = false;
    }
}

fn valid_remote_path(path: &str) -> bool {
    path.len() <= 4096 && RemotePosixPath::new(path.to_owned()).is_ok()
}

fn parent_directory(path: &str) -> Option<String> {
    let path = path.trim_end_matches('/');
    if path.is_empty() {
        return None;
    }
    path.rsplit_once('/').map(|(parent, _)| {
        if parent.is_empty() {
            "/".to_owned()
        } else {
            parent.to_owned()
        }
    })
}

pub(super) struct RemoteFolderPicker {
    connection: Connection,
    browser: Option<DirectoryBrowser>,
    path_input: Entity<InputState>,
    listing: ListingState,
    connecting: bool,
}

impl EventEmitter<RemoteFolderPickerEvent> for RemoteFolderPicker {}

impl RemoteFolderPicker {
    pub(super) fn new(
        connection: Connection,
        initial_path: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let path_input = cx.new(|cx| {
            let mut input = InputState::new(cx);
            input.content = initial_path.clone().unwrap_or_default().into();
            input
        });
        cx.observe(&path_input, |_, _, cx| cx.notify()).detach();
        let mut picker = Self {
            connection,
            browser: None,
            path_input,
            listing: ListingState::default(),
            connecting: false,
        };
        picker.load(initial_path, false, cx);
        picker
    }

    fn load(&mut self, path: Option<String>, reconnect: bool, cx: &mut Context<Self>) {
        let Some(generation) = self.listing.begin(path.clone()) else {
            return;
        };
        if path.as_ref().is_some_and(|path| !valid_remote_path(path)) {
            self.listing.finish(
                generation,
                Err(t!("workspaces.remote_picker.invalid_path").to_string()),
            );
            cx.notify();
            return;
        }
        if reconnect {
            self.browser.take();
        }
        self.connecting = self.browser.is_none();
        if self.browser.is_none() {
            match DirectoryBrowser::start(self.connection.clone()) {
                Ok(browser) => self.browser = Some(browser),
                Err(error) => {
                    self.listing.finish(generation, Err(error));
                    cx.notify();
                    return;
                }
            }
        }
        let receiver = self.browser.as_ref().unwrap().list(path);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err(t!("workspaces.remote_picker.worker_closed").to_string()));
            let _ = this.update(cx, |view, cx| {
                if view.listing.finish(generation, result) {
                    view.connecting = false;
                    if view.listing.error.is_none() {
                        let path = view.listing.current.as_ref().unwrap().path.clone();
                        view.path_input
                            .update(cx, |input, cx| input.replace_content(path, cx));
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn go(&mut self, generation: u64, cx: &mut Context<Self>) {
        if self.listing.accepts_action(generation) {
            let path = self.path_input.read(cx).content().to_owned();
            self.load(Some(path), false, cx);
        }
    }

    fn navigate_entry(
        &mut self,
        generation: u64,
        parent: &str,
        path: &str,
        cx: &mut Context<Self>,
    ) {
        if self.listing.accepts_entry(generation, parent, path) {
            self.load(Some(path.to_owned()), false, cx);
        }
    }

    pub(super) fn close(&mut self, cx: &mut Context<Self>) {
        self.listing.close();
        self.browser.take();
        cx.notify();
    }

    fn select(&mut self, generation: u64, cx: &mut Context<Self>) {
        let Some(path) = self
            .listing
            .selected_input_path(generation, self.path_input.read(cx).content().as_ref())
        else {
            return;
        };
        self.close(cx);
        cx.emit(RemoteFolderPickerEvent::Selected(path));
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.listing.closed {
            return;
        }
        self.close(cx);
        cx.emit(RemoteFolderPickerEvent::Cancelled);
        cx.notify();
    }
}

impl Render for RemoteFolderPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let generation = self.listing.generation;
        let busy = self.listing.pending || self.listing.closed;
        let selected = self
            .listing
            .selected_input_path(generation, self.path_input.read(cx).content().as_ref());
        let current_path = self
            .listing
            .current
            .as_ref()
            .map(|directory| directory.path.clone());
        let parent = current_path.as_deref().and_then(parent_directory);
        let enter_target = cx.entity().downgrade();
        let mut root = div()
            .flex()
            .flex_col()
            .w_full()
            .min_w_0()
            .gap(px(12.0))
            .child(
                div()
                    .text_size(px(16.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t!("workspaces.remote_picker.title").to_string()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .min_w_0()
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.path_input)
                                .size(InputSize::Sm)
                                .disabled(busy)
                                .placeholder(
                                    t!("workspaces.remote_picker.path_placeholder").to_string(),
                                )
                                .on_enter(move |_, cx| {
                                    let _ =
                                        enter_target.update(cx, |view, cx| view.go(generation, cx));
                                }),
                        ),
                    )
                    .child(
                        Button::new(
                            "remote-folder-go",
                            t!("workspaces.remote_picker.go").to_string(),
                        )
                        .variant(ButtonVariant::Outline)
                        .size(ButtonSize::Sm)
                        .disabled(busy)
                        .on_click(cx.listener(move |view, _, _, cx| view.go(generation, cx))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        Button::new(
                            "remote-folder-home",
                            t!("workspaces.remote_picker.home").to_string(),
                        )
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .icon("house")
                        .disabled(busy)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            if view.listing.accepts_action(generation) {
                                view.load(None, false, cx);
                            }
                        })),
                    )
                    .child(
                        Button::new(
                            "remote-folder-up",
                            t!("workspaces.remote_picker.up").to_string(),
                        )
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .icon("arrow-up")
                        .disabled(busy || parent.is_none())
                        .on_click(cx.listener(move |view, _, _, cx| {
                            if view.listing.accepts_action(generation) {
                                if let Some(path) = &parent {
                                    view.load(Some(path.clone()), false, cx);
                                }
                            }
                        })),
                    ),
            );
        if self.listing.pending {
            root = root.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(if self.connecting {
                        t!(
                            "workspaces.remote_picker.connecting",
                            host = self.connection.display_name()
                        )
                        .to_string()
                    } else {
                        t!("workspaces.remote_picker.loading").to_string()
                    }),
            );
        }
        if let Some(error) = &self.listing.error {
            root = root
                .child(
                    Alert::new()
                        .variant(AlertVariant::Error)
                        .description(error.clone()),
                )
                .child(
                    Button::new(
                        "remote-folder-retry",
                        t!("workspaces.remote_picker.retry").to_string(),
                    )
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Outline)
                    .disabled(busy)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if view.listing.accepts_action(generation) {
                            view.load(view.listing.requested.clone(), true, cx);
                        }
                    })),
                );
        }
        let count = self
            .listing
            .current
            .as_ref()
            .map_or(0, |directory| directory.entries.len());
        // Reserve room for the connection selector and footer at short window heights.
        let reserved = px(540.0).to_pixels(window.rem_size());
        let list_height = (window.viewport_size().height - reserved)
            .max(px(80.0).to_pixels(window.rem_size()))
            .min(px(220.0).to_pixels(window.rem_size()));
        let list_height = if self.listing.error.is_some() {
            px(32.0).to_pixels(window.rem_size())
        } else {
            list_height
        };
        let list: AnyElement = if count == 0 {
            div()
                .flex()
                .items_center()
                .justify_center()
                .h(list_height)
                .w_full()
                .text_size(px(12.0))
                .text_color(ShellDeckColors::text_muted())
                .child(if self.listing.current.is_some() && !self.listing.pending {
                    t!("workspaces.remote_picker.empty").to_string()
                } else {
                    String::new()
                })
                .into_any_element()
        } else {
            uniform_list(
                "remote-folder-list",
                count,
                cx.processor(move |view, range: Range<usize>, _, cx| {
                    let Some(directory) = view.listing.current.as_ref() else {
                        return Vec::new();
                    };
                    range
                        .filter_map(|index| {
                            directory.entries.get(index).map(|entry| (index, entry))
                        })
                        .map(|(index, entry)| {
                            let parent = directory.path.clone();
                            let path = entry.path.clone();
                            let label = entry.name.chars().take(80).collect::<String>();
                            div()
                                .h(px(34.0))
                                .w_full()
                                .min_w_0()
                                .child(
                                    Button::new(("remote-folder-entry", index), label)
                                        .variant(ButtonVariant::Ghost)
                                        .size(ButtonSize::Sm)
                                        .icon("folder")
                                        .w_full()
                                        .h(px(32.0))
                                        .justify_start()
                                        .tooltip(entry.path.clone())
                                        .disabled(view.listing.selected_path(generation).is_none())
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            view.navigate_entry(generation, &parent, &path, cx);
                                        })),
                                )
                                .into_any_element()
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .h(list_height)
            .w_full()
            .into_any_element()
        };
        root = root.child(
            div()
                .min_h_0()
                .min_w_0()
                .border_1()
                .border_color(ShellDeckColors::border())
                .rounded(px(6.0))
                .overflow_hidden()
                .child(list),
        );
        if self
            .listing
            .current
            .as_ref()
            .is_some_and(|directory| directory.truncated)
        {
            root = root.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("workspaces.remote_picker.truncated").to_string()),
            );
        }
        root.child(
            div()
                .flex()
                .justify_end()
                .flex_wrap()
                .gap(px(8.0))
                .child(
                    Button::new(
                        "remote-folder-cancel",
                        t!("workspaces.remote_picker.cancel").to_string(),
                    )
                    .variant(ButtonVariant::Ghost)
                    .disabled(self.listing.closed)
                    .on_click(cx.listener(|view, _, _, cx| view.cancel(cx))),
                )
                .child(
                    Button::new(
                        "remote-folder-select",
                        t!("workspaces.remote_picker.select").to_string(),
                    )
                    .disabled(selected.is_none())
                    .on_click(cx.listener(move |view, _, _, cx| view.select(generation, cx))),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{parent_directory, Directory, ListingState};
    use shelldeck_ssh::agent_files::Entry;

    fn directory(path: &str) -> Directory {
        Directory {
            root: "/".into(),
            path: path.into(),
            entries: vec![
                Entry {
                    name: "child".into(),
                    path: format!("{path}/child"),
                    is_dir: true,
                    is_symlink: false,
                    size: 0,
                },
                Entry {
                    name: "file".into(),
                    path: format!("{path}/file"),
                    is_dir: false,
                    is_symlink: false,
                    size: 1,
                },
            ],
            truncated: false,
        }
    }

    // SDTEST-2717 — stale failures/successes cannot replace loaded selection authority.
    #[test]
    fn stale_listings_and_failed_navigation_never_authorize_an_unloaded_folder() {
        let mut state = ListingState::default();
        let first = state.begin(None).unwrap();
        assert!(state.finish(first, Ok(directory("/home/test"))));
        assert_eq!(state.selected_path(first).as_deref(), Some("/home/test"));
        assert_eq!(state.current.as_ref().unwrap().entries.len(), 1);
        assert_eq!(
            state.selected_input_path(first, "/home/test").as_deref(),
            Some("/home/test")
        );
        assert!(state
            .selected_input_path(first, "/typed-but-not-loaded")
            .is_none());
        let slow = state.begin(Some("/slow".into())).unwrap();
        let latest = state.begin(Some("/new".into())).unwrap();
        assert!(!state.finish(slow, Ok(directory("/wrong"))));
        assert!(state.selected_path(latest).is_none());
        assert!(state.finish(latest, Err("permission denied".into())));
        assert_eq!(state.current.as_ref().unwrap().path, "/home/test");
        assert!(state.selected_path(latest).is_none());
        let retry = state.begin(Some("/new".into())).unwrap();
        assert!(state.finish(retry, Ok(directory("/canonical/new"))));
        assert_eq!(
            state.selected_path(retry).as_deref(),
            Some("/canonical/new")
        );
        let spaced = state.begin(Some("/srv/project ".into())).unwrap();
        state.finish(spaced, Ok(directory("/srv/project ")));
        assert_eq!(
            state
                .selected_input_path(spaced, "/srv/project ")
                .as_deref(),
            Some("/srv/project ")
        );
        assert!(state.selected_input_path(spaced, "/srv/project").is_none());
        assert!(!state.finish(latest, Err("late failure".into())));
        assert_eq!(
            state.selected_path(spaced).as_deref(),
            Some("/srv/project ")
        );
    }

    // SDTEST-2718 — old row/selection callbacks and cancelled responses are inert.
    #[test]
    fn old_directory_clicks_and_cancelled_results_cannot_navigate_or_select() {
        let mut state = ListingState::default();
        let first = state.begin(None).unwrap();
        state.finish(first, Ok(directory("/home/test")));
        assert!(state.accepts_entry(first, "/home/test", "/home/test/child"));
        assert!(!state.accepts_entry(first, "/home/test", "/home/test/file"));
        assert!(!state.accepts_entry(first, "/foreign", "/home/test/child"));
        let next = state.begin(Some("/next".into())).unwrap();
        assert!(!state.accepts_entry(first, "/home/test", "/home/test/child"));
        state.finish(next, Ok(directory("/next")));
        assert!(state.selected_path(first).is_none());
        assert!(state.selected_path(next).is_some());
        let pending = state.begin(None).unwrap();
        state.close();
        assert!(!state.finish(pending, Ok(directory("/late"))));
        assert!(state.selected_path(pending).is_none());
        assert!(state.begin(None).is_none());
        assert_eq!(parent_directory("/home/test"), Some("/home".into()));
        assert_eq!(parent_directory("/home"), Some("/".into()));
        assert_eq!(parent_directory("/"), None);
    }
}
