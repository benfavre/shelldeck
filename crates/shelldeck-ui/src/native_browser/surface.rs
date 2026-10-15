//! Native GPUI element for CEF's live off-screen view and popup surfaces.
use super::*;
use geometry::{event_modifiers, utf16_slice, viewport_point, virtual_key};

pub(super) struct BrowserSurface {
    view: Entity<NativeBrowserView>,
}

impl BrowserSurface {
    pub(super) fn new(view: Entity<NativeBrowserView>) -> Self {
        Self { view }
    }
}

impl IntoElement for BrowserSurface {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for BrowserSurface {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Hitbox {
        self.view.update(cx, |view, cx| {
            view.update_viewport(bounds, window.scale_factor(), cx)
        });
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        let view = self.view.read(cx);
        let focus = view.focus.clone();
        let cursor = view
            .selected_tab()
            .map(|tab| tab.cursor)
            .unwrap_or_default();
        let (image, popup_image, popup_rect) = view
            .selected_tab()
            .map(|tab| (tab.image.clone(), tab.popup_image.clone(), tab.popup_rect))
            .unwrap_or_default();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
        window.set_cursor_style(cursor, hitbox);
        let hover = hitbox.clone();
        let view = self.view.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            view.update(cx, |view, cx| {
                if hover.is_hovered(window) || view.pressed_buttons != 0 {
                    view.mouse_move(event, cx);
                } else if view.pointer_inside {
                    view.pointer_inside = false;
                    if let Some(browser_id) = view.active {
                        view.send(
                            Command::MouseMove {
                                browser_id,
                                x: 0,
                                y: 0,
                                modifiers: event_modifiers(event.modifiers),
                                leave: true,
                            },
                            cx,
                        );
                    }
                }
            });
        });
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(image) = image {
                if let Err(error) =
                    window.paint_image(bounds, bounds, Corners::default(), image, 0, false)
                {
                    tracing::warn!(%error, "browser view texture could not be painted");
                }
            }
            if let (Some(image), Some(rect)) = (popup_image, popup_rect) {
                // CEF popup origin/extent are browser-local logical pixels. A
                // dropdown is clipped to the same viewport as the page itself.
                let popup_bounds = Bounds::new(
                    bounds.origin + point(gpui::px(rect.x as f32), gpui::px(rect.y as f32)),
                    size(
                        gpui::px(rect.width.max(0) as f32),
                        gpui::px(rect.height.max(0) as f32),
                    ),
                );
                if let Err(error) = window.paint_image(
                    popup_bounds,
                    popup_bounds,
                    Corners::default(),
                    image,
                    0,
                    false,
                ) {
                    tracing::warn!(%error, "browser popup texture could not be painted");
                }
            }
        });
    }
}

impl NativeBrowserView {
    fn update_viewport(
        &mut self,
        bounds: Bounds<Pixels>,
        scale_factor: f32,
        cx: &mut Context<Self>,
    ) {
        self.viewport = Some(bounds);
        let width = f32::from(bounds.size.width).round().clamp(1.0, 4096.0) as u32;
        let height = f32::from(bounds.size.height).round().clamp(1.0, 4096.0) as u32;
        // Cap physical frame dimensions without changing CSS viewport/input
        // coordinates. This also bounds memory on very large HiDPI displays.
        let scale = scale_factor
            .clamp(0.25, 4.0)
            .min(4096.0 / width.max(height) as f32);
        let signature = (width, height, (scale * 1000.0).round() as u32);
        if self.viewport_size != Some(signature) {
            self.viewport_size = Some(signature);
            if let Some(browser_id) = self.active {
                self.send(
                    Command::Resize {
                        browser_id,
                        width,
                        height,
                        scale_factor: scale,
                    },
                    cx,
                );
            }
        }
    }

    pub(super) fn mouse_button(
        &mut self,
        position: Point<Pixels>,
        modifiers: Modifiers,
        button: MouseButton,
        pressed: bool,
        click_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(browser_id) = self.active else {
            return;
        };
        let Some(bounds) = self.viewport else {
            return;
        };
        let (button, flag) = match button {
            MouseButton::Left => (
                shelldeck_browser::MouseButton::Left,
                shelldeck_browser::modifiers::LEFT_MOUSE,
            ),
            MouseButton::Middle => (
                shelldeck_browser::MouseButton::Middle,
                shelldeck_browser::modifiers::MIDDLE_MOUSE,
            ),
            MouseButton::Right => (
                shelldeck_browser::MouseButton::Right,
                shelldeck_browser::modifiers::RIGHT_MOUSE,
            ),
            MouseButton::Navigate(_) => return,
        };
        if !pressed && self.pressed_buttons & flag == 0 {
            return;
        }
        let Some((x, y)) = viewport_point(bounds, position, !pressed) else {
            return;
        };
        if pressed {
            self.focus.focus(window);
            self.pressed_buttons |= flag;
            self.send(
                Command::Focus {
                    browser_id,
                    focused: true,
                },
                cx,
            );
        } else {
            self.pressed_buttons &= !flag;
        }
        self.send(
            Command::MouseButton {
                browser_id,
                x,
                y,
                button,
                pressed,
                click_count: click_count.min(3) as i32,
                modifiers: event_modifiers(modifiers) | self.pressed_buttons,
            },
            cx,
        );
        cx.stop_propagation();
    }

    pub(super) fn mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(browser_id) = self.active else {
            return;
        };
        let Some(bounds) = self.viewport else {
            return;
        };
        let local = viewport_point(bounds, event.position, self.pressed_buttons != 0);
        if let Some((x, y)) = local {
            self.pointer_inside = true;
            self.send(
                Command::MouseMove {
                    browser_id,
                    x,
                    y,
                    modifiers: event_modifiers(event.modifiers) | self.pressed_buttons,
                    leave: false,
                },
                cx,
            );
        } else if self.pointer_inside {
            self.pointer_inside = false;
            self.send(
                Command::MouseMove {
                    browser_id,
                    x: 0,
                    y: 0,
                    modifiers: event_modifiers(event.modifiers),
                    leave: true,
                },
                cx,
            );
        }
    }

    pub(super) fn mouse_wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let Some(browser_id) = self.active else {
            return;
        };
        let Some(bounds) = self.viewport else {
            return;
        };
        let Some((x, y)) = viewport_point(bounds, event.position, false) else {
            return;
        };
        let delta = event.delta.pixel_delta(gpui::px(40.0));
        self.send(
            Command::MouseWheel {
                browser_id,
                x,
                y,
                delta_x: f32::from(delta.x).round() as i32,
                delta_y: f32::from(delta.y).round() as i32,
                modifiers: event_modifiers(event.modifiers),
            },
            cx,
        );
        cx.stop_propagation();
    }

    pub(super) fn key(
        &mut self,
        stroke: Keystroke,
        released: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(browser_id) = self.active else {
            return;
        };
        if !self.focus.is_focused(window) || self.page_dialog.is_some() {
            return;
        }
        if self.context_menu.is_some() && !released && stroke.key == "escape" {
            self.context_menu = None;
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if !released && stroke.modifiers.secondary() && stroke.key == "l" {
            self.focus_address(window, cx);
            cx.stop_propagation();
            return;
        }
        if !released && stroke.modifiers.secondary() {
            match stroke.key.as_str() {
                "c" | "x" => {
                    // The headless CEF helper has its own clipboard backend.
                    // Bridge explicit user copy/cut into the desktop clipboard;
                    // raw Cut still reaches Chromium to remove the selection.
                    if !self.selected_text.is_empty() {
                        cx.write_to_clipboard(ClipboardItem::new_string(
                            self.selected_text.clone(),
                        ));
                    }
                }
                "v" => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        self.send(Command::Text { browser_id, text }, cx);
                    }
                    cx.stop_propagation();
                    return;
                }
                "t" => {
                    self.new_tab("about:blank".into(), cx);
                    self.focus_address(window, cx);
                    cx.stop_propagation();
                    return;
                }
                "w" => {
                    self.close_tab(browser_id, cx);
                    cx.stop_propagation();
                    return;
                }
                "r" => {
                    self.send(
                        Command::Reload {
                            browser_id,
                            ignore_cache: stroke.modifiers.shift,
                        },
                        cx,
                    );
                    cx.stop_propagation();
                    return;
                }
                "f" => {
                    self.find_visible = true;
                    self.find_input.read(cx).focus_handle(cx).focus(window);
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                "tab" => {
                    if let Some(index) = self
                        .tabs
                        .iter()
                        .position(|tab| tab.state.browser_id == browser_id)
                    {
                        let next = if stroke.modifiers.shift {
                            (index + self.tabs.len() - 1) % self.tabs.len()
                        } else {
                            (index + 1) % self.tabs.len()
                        };
                        self.select_tab(self.tabs[next].state.browser_id, cx);
                    }
                    cx.stop_propagation();
                    return;
                }
                _ => {}
            }
        }
        if !released && stroke.modifiers.alt && matches!(stroke.key.as_str(), "left" | "right") {
            self.send(
                if stroke.key == "left" {
                    Command::Back { browser_id }
                } else {
                    Command::Forward { browser_id }
                },
                cx,
            );
            cx.stop_propagation();
            return;
        }
        // GPUI's platform backend invokes EntityInputHandler only when the key
        // event propagates. Keep printable text flowing to that single path;
        // consume navigation/shortcuts so they cannot reach the app's editor.
        let native_text = !released && stroke.keystroke_text_candidate();
        self.send(
            Command::Key {
                browser_id,
                event: KeyEvent {
                    kind: if released {
                        KeyEventKind::KeyUp
                    } else {
                        KeyEventKind::RawKeyDown
                    },
                    windows_key_code: virtual_key(&stroke.key),
                    native_key_code: 0,
                    modifiers: event_modifiers(stroke.modifiers),
                    // Printable text is sent exclusively by EntityInputHandler. Sending
                    // CHAR here as well would duplicate Unicode text and IME commits.
                    character: None,
                    unmodified_character: None,
                    is_system_key: stroke.modifiers.alt,
                },
            },
            cx,
        );
        if !native_text {
            cx.stop_propagation();
        }
    }
}

trait NativeTextCandidate {
    fn keystroke_text_candidate(&self) -> bool;
}

impl NativeTextCandidate for Keystroke {
    fn keystroke_text_candidate(&self) -> bool {
        self.key_char
            .as_ref()
            .is_some_and(|text| text.chars().any(|character| !character.is_control()))
            && !self.modifiers.control
            && !self.modifiers.platform
    }
}

fn wire_range(range: Option<Range<usize>>) -> Option<Utf16Range> {
    range
        .filter(|range| range.start <= range.end && range.end <= u32::MAX as usize)
        .map(|range| Utf16Range {
            start: range.start as u32,
            end: range.end as u32,
        })
}

impl EntityInputHandler for NativeBrowserView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        if let Some(marked) = &self.marked_range {
            if range.start >= marked.start && range.end <= marked.end {
                let text = utf16_slice(
                    &self.composition_text,
                    range.start - marked.start..range.end - marked.start,
                )?;
                *adjusted = Some(range);
                return Some(text);
            }
        }
        if range.start >= self.selected_range.start && range.end <= self.selected_range.end {
            let text = utf16_slice(
                &self.selected_text,
                range.start - self.selected_range.start..range.end - self.selected_range.start,
            )?;
            *adjusted = Some(range);
            return Some(text);
        }
        None
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        self.active.map(|_| UTF16Selection {
            range: self.selected_range.clone(),
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range.clone()
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(browser_id) = self.active {
            // Unmark means finish the composition; cancelling would silently
            // discard text when the native IME finishes without another commit.
            if self.marked_range.take().is_some() {
                let text = std::mem::take(&mut self.composition_text);
                self.send(
                    Command::Ime {
                        browser_id,
                        event: ImeEvent::Commit {
                            text,
                            replacement_range: None,
                        },
                    },
                    cx,
                );
            }
        }
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(browser_id) = self.active else {
            return;
        };
        self.send(
            Command::Ime {
                browser_id,
                event: ImeEvent::Commit {
                    text: text.to_owned(),
                    replacement_range: wire_range(range),
                },
            },
            cx,
        );
        self.composition_text.clear();
        self.marked_range = None;
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(browser_id) = self.active else {
            return;
        };
        let start = range
            .as_ref()
            .map(|range| range.start)
            .or_else(|| self.marked_range.as_ref().map(|range| range.start))
            .unwrap_or(self.selected_range.start);
        let length = text.encode_utf16().count();
        let selected = selected.unwrap_or(length..length);
        self.composition_text = text.to_owned();
        self.marked_range = (!text.is_empty()).then_some(start..start + length);
        self.selected_range = start + selected.start..start + selected.end;
        self.send(
            Command::Ime {
                browser_id,
                event: ImeEvent::SetComposition {
                    text: text.to_owned(),
                    selection_start: selected.start.min(i32::MAX as usize) as i32,
                    selection_end: selected.end.min(i32::MAX as usize) as i32,
                    replacement_range: wire_range(range),
                },
            },
            cx,
        );
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let composition_start = self
            .marked_range
            .as_ref()
            .map(|range| range.start)
            .unwrap_or(self.selected_range.start);
        let index = range
            .start
            .saturating_sub(composition_start)
            .min(self.ime_character_bounds.len().saturating_sub(1));
        let rect = self.ime_character_bounds.get(index)?;
        Some(Bounds::new(
            element_bounds.origin + point(gpui::px(rect.x as f32), gpui::px(rect.y as f32)),
            size(
                gpui::px(rect.width.max(1) as f32),
                gpui::px(rect.height.max(1) as f32),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let (x, y) = viewport_point(self.viewport?, position, false)?;
        let start = self
            .marked_range
            .as_ref()
            .map(|range| range.start)
            .unwrap_or(self.selected_range.start);
        self.ime_character_bounds
            .iter()
            .position(|rect| {
                x >= rect.x && y >= rect.y && x < rect.x + rect.width && y < rect.y + rect.height
            })
            .map(|index| start + index)
    }
}
