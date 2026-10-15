use super::*;
#[cfg(target_os = "linux")]
type NativeCursor = std::os::raw::c_ulong;
#[cfg(target_os = "macos")]
type NativeCursor = *mut u8;
#[cfg(target_os = "windows")]
type NativeCursor = cef::sys::HCURSOR;
fn cursor_name(value: CursorType) -> &'static str {
    match value {
        CursorType::HAND => "pointer",
        CursorType::IBEAM => "text",
        CursorType::CROSS => "crosshair",
        CursorType::WAIT | CursorType::PROGRESS => "wait",
        CursorType::GRAB => "grab",
        CursorType::GRABBING => "grabbing",
        CursorType::EASTRESIZE | CursorType::WESTRESIZE | CursorType::EASTWESTRESIZE => "ew-resize",
        CursorType::NORTHRESIZE | CursorType::SOUTHRESIZE | CursorType::NORTHSOUTHRESIZE => {
            "ns-resize"
        }
        CursorType::NOTALLOWED => "not-allowed",
        CursorType::NONE => "none",
        _ => "default",
    }
}

wrap_render_handler! {
    struct PaintHandler { tab: SharedTab }
    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut cef::Rect>) {
            if let Some(rect) = rect { let (width, height, _) = self.tab.viewport.get(); *rect = cef::Rect { x: 0, y: 0, width: width as i32, height: height as i32 }; }
        }
        fn screen_info(&self, _browser: Option<&mut Browser>, screen_info: Option<&mut ScreenInfo>) -> i32 {
            if let Some(info) = screen_info {
                let (width, height, scale) = self.tab.viewport.get(); info.device_scale_factor = scale; info.depth = 32; info.depth_per_component = 8;
                info.rect = cef::Rect { x:0, y:0, width: width as i32, height: height as i32 }; info.available_rect = info.rect.clone(); return 1;
            } 0
        }
        fn screen_point(&self, _browser: Option<&mut Browser>, view_x: i32, view_y: i32, screen_x: Option<&mut i32>, screen_y: Option<&mut i32>) -> i32 {
            // Coordinates are virtual; no compositor window position is guessed.
            if let Some(x) = screen_x { *x = view_x; } if let Some(y) = screen_y { *y = view_y; } 1
        }
        fn on_paint(&self, _browser: Option<&mut Browser>, type_: PaintElementType, dirty_rects: Option<&[cef::Rect]>, buffer: *const u8, width: i32, height: i32) {
            if buffer.is_null() || width <= 0 || height <= 0 { return; }
            let Ok(length) = protocol::frame_byte_len(width as u32, height as u32) else { self.tab.error("Chromium emitted a frame larger than the configured limit"); return; };
            // CEF owns buffer for this callback only. Copy once and let writer
            // replace an older pending frame instead of stalling its UI thread.
            let pixels = unsafe { std::slice::from_raw_parts(buffer, length) }.to_vec();
            self.tab.output.paint(Event::Paint { browser_id: self.tab.id, kind: if type_ == PaintElementType::POPUP { PaintKind::Popup } else { PaintKind::View }, width: width as u32, height: height as u32, dirty_rects: dirty_rects.unwrap_or_default().iter().map(rect).collect() }, pixels);
        }
        fn on_popup_show(&self, _browser: Option<&mut Browser>, show: i32) { self.tab.popup_visible.set(show != 0); self.tab.popup_event(); }
        fn on_popup_size(&self, _browser: Option<&mut Browser>, value: Option<&cef::Rect>) { if let Some(value) = value { self.tab.popup.set(rect(value)); self.tab.popup_event(); } }
        fn on_ime_composition_range_changed(&self, _browser: Option<&mut Browser>, selected_range: Option<&Range>, character_bounds: Option<&[cef::Rect]>) {
            if let Some(value) = selected_range { self.tab.output.send(Event::ImeRangeChanged { browser_id: self.tab.id, range: protocol::Utf16Range { start:value.from,end:value.to }, character_bounds: character_bounds.unwrap_or_default().iter().take(4096).map(rect).collect() }); }
        }
        fn on_text_selection_changed(&self, _browser: Option<&mut Browser>, selected_text: Option<&CefString>, selected_range: Option<&Range>) {
            let value = selected_range.cloned().unwrap_or_default(); self.tab.output.send(Event::TextSelectionChanged { browser_id: self.tab.id, selected_text: clipped(string(selected_text), 65536), range: protocol::Utf16Range { start:value.from,end:value.to } });
        }
    }
}
fn rect(rect: &cef::Rect) -> protocol::Rect {
    protocol::Rect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}
fn string(value: Option<&CefString>) -> String {
    value.map(ToString::to_string).unwrap_or_default()
}

wrap_display_handler! {
    struct PageDisplayHandler { tab: SharedTab }
    impl DisplayHandler {
        fn on_cursor_change(&self, _browser: Option<&mut Browser>, _cursor: NativeCursor, type_: CursorType, _custom_cursor_info: Option<&CursorInfo>) -> i32 { self.tab.output.send(Event::CursorChanged { browser_id:self.tab.id,cursor:cursor_name(type_).into() }); 1 }
        fn on_address_change(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, url: Option<&CefString>) {
            if frame.is_some_and(|frame| frame.is_main() == 1) { self.tab.state.borrow_mut().url = clipped(string(url), 16384); self.tab.emit_state(); }
        }
        fn on_title_change(&self, _browser: Option<&mut Browser>, title: Option<&CefString>) { self.tab.state.borrow_mut().title = clipped(string(title), 4096); self.tab.emit_state(); }
        fn on_console_message(&self, _browser: Option<&mut Browser>, _level: LogSeverity, _message: Option<&CefString>, _source: Option<&CefString>, _line: i32) -> i32 { 1 }
    }
}
wrap_load_handler! {
    struct PageLoadHandler { tab: SharedTab }
    impl LoadHandler {
        fn on_loading_state_change(&self, _browser: Option<&mut Browser>, is_loading: i32, can_go_back: i32, can_go_forward: i32) {
            { let mut state = self.tab.state.borrow_mut(); state.loading = is_loading != 0; state.can_go_back = can_go_back != 0; state.can_go_forward = can_go_forward != 0; }
            self.tab.emit_state();
        }
        fn on_load_error(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, error_code: Errorcode, error_text: Option<&CefString>, _failed_url: Option<&CefString>) {
            if error_code != Errorcode::ABORTED && frame.is_some_and(|frame| frame.is_main() == 1) { self.tab.error(format!("Page failed to load: {}", string(error_text))); }
        }
    }
}
wrap_life_span_handler! {
    struct PageLifeHandler { tab: SharedTab }
    impl LifeSpanHandler {
        fn on_before_popup(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, popup_id: i32, target_url: Option<&CefString>, _target_frame_name: Option<&CefString>, _target_disposition: WindowOpenDisposition, _user_gesture: i32, _popup_features: Option<&PopupFeatures>, window_info: Option<&mut WindowInfo>, client: Option<&mut Option<Client>>, _settings: Option<&mut BrowserSettings>, _extra_info: Option<&mut Option<DictionaryValue>>, no_javascript_access: Option<&mut i32>) -> i32 {
            if self.tab.tab_count.get() >= protocol::MAX_TABS { self.tab.error("Browser tab limit reached"); return 1; }
            let raw_url = string(target_url);
            let url = match protocol::navigation_url(if raw_url.is_empty() { "about:blank" } else { &raw_url }) { Ok(url) => url, Err(error) => { self.tab.error(error); return 1; } };
            let (Some(info), Some(client)) = (window_info, client) else { return 1; };
            let id = NEXT_POPUP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let tab = self.tab.child(id, url, popup_id);
            self.tab.pending_popups.borrow_mut().insert(popup_id, id);
            self.tab.tab_count.set(self.tab.tab_count.get() + 1);
            info.windowless_rendering_enabled = 1;
            info.shared_texture_enabled = 0;
            info.external_begin_frame_enabled = 0;
            info.runtime_style = RuntimeStyle::ALLOY;
            *client = Some(make_client(tab));
            if let Some(flag) = no_javascript_access { *flag = 0; }
            // Let CEF create its real related WebContents. Recreating the URL in
            // an independent tab would destroy window.opener and postMessage.
            0
        }
        fn on_before_popup_aborted(&self, _browser: Option<&mut Browser>, popup_id: i32) {
            if self.tab.pending_popups.borrow_mut().remove(&popup_id).is_some() { self.tab.tab_count.set(self.tab.tab_count.get().saturating_sub(1)); }
        }
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            if self.tab.parent_id.is_none() { return; }
            if let Some((pending, popup_id)) = &self.tab.popup_origin { pending.borrow_mut().remove(popup_id); }
            if let Some(browser) = browser {
                PENDING_POPUPS.with_borrow_mut(|pending| { pending.insert(self.tab.id, (browser.clone(), self.tab.clone())); });
                post(Work::PopupCreated(self.tab.id));
            }
        }
        fn on_before_close(&self, _browser: Option<&mut Browser>) { post(Work::Closed(self.tab.id)); }
    }
}
wrap_dev_tools_message_observer! {
    pub(super) struct DevToolsObserver { tab: SharedTab }
    impl DevToolsMessageObserver {
        fn on_dev_tools_method_result(&self, _browser: Option<&mut Browser>, message_id: i32, success: i32, result: Option<&[u8]>) {
            let Some(request_id) = self.tab.pending_cdp.borrow_mut().remove(&message_id) else { return; };
            let result = result.and_then(|bytes| serde_json::from_slice(bytes).ok()).unwrap_or(JsonValue::Null);
            let error = if success == 1 { None } else { Some(result.get("message").and_then(JsonValue::as_str).unwrap_or("Chromium DevTools command failed").into()) };
            self.tab.output.send(Event::DevToolsResult { browser_id:self.tab.id,request_id,result,error });
        }
        fn on_dev_tools_agent_detached(&self, _browser: Option<&mut Browser>) {
            for (_, request_id) in self.tab.pending_cdp.borrow_mut().drain() { self.tab.output.send(Event::DevToolsResult { browser_id:self.tab.id,request_id,result:JsonValue::Null,error:Some("Chromium DevTools agent detached".into()) }); }
        }
    }
}
wrap_jsdialog_handler! {
    struct PageDialogHandler { tab: SharedTab }
    impl JsdialogHandler {
        fn on_jsdialog(&self, _browser: Option<&mut Browser>, _origin_url: Option<&CefString>, dialog_type: JsdialogType, message_text: Option<&CefString>, default_prompt_text: Option<&CefString>, callback: Option<&mut JsdialogCallback>, _suppress_message: Option<&mut i32>) -> i32 {
            if let Some(callback) = callback {
                let id = self.tab.next_id(); self.tab.dialogs.borrow_mut().insert(id, callback.clone());
                let kind = if dialog_type == JsdialogType::ALERT { "alert" } else if dialog_type == JsdialogType::PROMPT { "prompt" } else { "confirm" };
                self.tab.output.send(Event::Dialog { browser_id:self.tab.id,dialog_id:id,kind:kind.into(),message:clipped(string(message_text),65536),default_prompt:clipped(string(default_prompt_text),65536) });
                return 1;
            } 0
        }
        fn on_before_unload_dialog(&self, _browser: Option<&mut Browser>, message_text: Option<&CefString>, _is_reload: i32, callback: Option<&mut JsdialogCallback>) -> i32 {
            if let Some(callback) = callback { let id=self.tab.next_id(); self.tab.dialogs.borrow_mut().insert(id,callback.clone()); self.tab.output.send(Event::Dialog { browser_id:self.tab.id,dialog_id:id,kind:"before_unload".into(),message:clipped(string(message_text),65536),default_prompt:String::new() }); return 1; } 0
        }
        fn on_reset_dialog_state(&self, _browser: Option<&mut Browser>) { self.tab.dialogs.borrow_mut().clear(); }
    }
}
wrap_dialog_handler! {
    struct PageFileHandler { tab: SharedTab }
    impl DialogHandler {
        fn on_file_dialog(&self, _browser: Option<&mut Browser>, mode: FileDialogMode, title: Option<&CefString>, _default_file_path: Option<&CefString>, accept_filters: Option<&mut CefStringList>, _accept_extensions: Option<&mut CefStringList>, _accept_descriptions: Option<&mut CefStringList>, callback: Option<&mut FileDialogCallback>) -> i32 {
            if let Some(callback) = callback {
                let id = self.tab.next_id(); self.tab.choosers.borrow_mut().insert(id,callback.clone());
                self.tab.output.send(Event::FileChooser { browser_id:self.tab.id,chooser_id:id,title:clipped(string(title),4096),accept_filters:accept_filters.map(|values| values.clone().into_iter().take(64).map(|value| clipped(value,4096)).collect()).unwrap_or_default(),multiple:mode == FileDialogMode::OPEN_MULTIPLE }); return 1;
            } 0
        }
    }
}
wrap_download_handler! {
    struct PageDownloadHandler { tab: SharedTab }
    impl DownloadHandler {
        fn can_download(&self, _browser: Option<&mut Browser>, _url: Option<&CefString>, _request_method: Option<&CefString>) -> i32 { 1 }
        fn on_before_download(&self, _browser: Option<&mut Browser>, download_item: Option<&mut DownloadItem>, suggested_name: Option<&CefString>, callback: Option<&mut BeforeDownloadCallback>) -> i32 {
            if let (Some(item), Some(callback)) = (download_item, callback) {
                let id = item.id() as u64; self.tab.downloads.borrow_mut().insert(id,callback.clone());
                self.tab.output.send(Event::Download { browser_id:self.tab.id,download_id:id,suggested_name:clipped(string(suggested_name),4096),url:clipped(CefString::from(&item.url()).to_string(),16384) }); return 1;
            } 0
        }
        fn on_download_updated(&self, _browser: Option<&mut Browser>, download_item: Option<&mut DownloadItem>, _callback: Option<&mut DownloadItemCallback>) {
            if let Some(item) = download_item { self.tab.output.send(Event::DownloadProgress { browser_id:self.tab.id,download_id:item.id() as u64,received_bytes:item.received_bytes(),total_bytes:item.total_bytes(),complete:item.is_complete()==1,canceled:item.is_canceled()==1 }); }
        }
    }
}
wrap_permission_handler! {
    struct PagePermissionHandler { tab: SharedTab }
    impl PermissionHandler {
        fn on_request_media_access_permission(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _requesting_origin: Option<&CefString>, _requested_permissions: u32, callback: Option<&mut MediaAccessCallback>) -> i32 {
            if let Some(callback) = callback { callback.cont(0); } self.tab.error("Camera, microphone, and screen capture permissions are disabled in the embedded browser"); 1
        }
        fn on_show_permission_prompt(&self, _browser: Option<&mut Browser>, _prompt_id: u64, _requesting_origin: Option<&CefString>, _requested_permissions: u32, callback: Option<&mut PermissionPromptCallback>) -> i32 {
            if let Some(callback) = callback { callback.cont(PermissionRequestResult::DENY); } self.tab.error("This page requested a browser permission that is disabled"); 1
        }
    }
}
wrap_client! {
    struct PageClient { tab: SharedTab }
    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> { Some(PaintHandler::new(self.tab.clone())) }
        fn display_handler(&self) -> Option<DisplayHandler> { Some(PageDisplayHandler::new(self.tab.clone())) }
        fn load_handler(&self) -> Option<LoadHandler> { Some(PageLoadHandler::new(self.tab.clone())) }
        fn life_span_handler(&self) -> Option<LifeSpanHandler> { Some(PageLifeHandler::new(self.tab.clone())) }
        fn jsdialog_handler(&self) -> Option<JsdialogHandler> { Some(PageDialogHandler::new(self.tab.clone())) }
        fn dialog_handler(&self) -> Option<DialogHandler> { Some(PageFileHandler::new(self.tab.clone())) }
        fn download_handler(&self) -> Option<DownloadHandler> { Some(PageDownloadHandler::new(self.tab.clone())) }
        fn permission_handler(&self) -> Option<PermissionHandler> { Some(PagePermissionHandler::new(self.tab.clone())) }
        fn request_handler(&self) -> Option<RequestHandler> { Some(PageRequestHandler::new(self.tab.clone())) }
        fn find_handler(&self) -> Option<FindHandler> { Some(PageFindHandler::new(self.tab.clone())) }
        fn context_menu_handler(&self) -> Option<ContextMenuHandler> { Some(PageContextMenuHandler::new(self.tab.clone())) }
    }
}
pub(super) fn make_client(tab: SharedTab) -> Client {
    PageClient::new(tab)
}

wrap_request_handler! {
    struct PageRequestHandler { tab: SharedTab }
    impl RequestHandler {
        fn on_before_browse(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, request: Option<&mut Request>, _user_gesture: i32, _is_redirect: i32) -> i32 {
            if !frame.is_some_and(|frame| frame.is_main() == 1) { return 0; }
            if let Some(request) = request {
                let url = CefString::from(&request.url()).to_string();
                if protocol::navigation_url(&url).is_err() { self.tab.error("This page tried to open an unsupported browser address"); return 1; }
            } 0
        }
        fn on_open_urlfrom_tab(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, target_url: Option<&CefString>, target_disposition: WindowOpenDisposition, _user_gesture: i32) -> i32 {
            let url = match protocol::navigation_url(&string(target_url)) { Ok(url) => url, Err(error) => { self.tab.error(error); return 1; } };
            match target_disposition {
                WindowOpenDisposition::NEW_FOREGROUND_TAB | WindowOpenDisposition::NEW_BACKGROUND_TAB | WindowOpenDisposition::NEW_WINDOW | WindowOpenDisposition::NEW_POPUP | WindowOpenDisposition::SINGLETON_TAB => {
                    self.tab.output.send(Event::NewWindow { browser_id:self.tab.id,url }); 1
                }
                _ => 0,
            }
        }
    }
}

wrap_find_handler! {
    struct PageFindHandler { tab: SharedTab }
    impl FindHandler {
        fn on_find_result(&self, _browser: Option<&mut Browser>, _identifier: i32, count: i32, _selection_rect: Option<&cef::Rect>, active_match_ordinal: i32, final_update: i32) {
            self.tab.output.send(Event::FindResult { browser_id:self.tab.id,count,active_match:active_match_ordinal,final_update:final_update!=0 });
        }
    }
}

wrap_context_menu_handler! {
    struct PageContextMenuHandler { tab: SharedTab }
    impl ContextMenuHandler {
        fn on_before_context_menu(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, params: Option<&mut ContextMenuParams>, model: Option<&mut MenuModel>) {
            // No native Chromium parent exists. The visible GPUI browser pane
            // owns its context menu instead of creating an unmanaged CEF window.
            if let Some(model) = model { model.clear(); }
            if let Some(params) = params { self.tab.output.send(Event::ContextMenu { browser_id:self.tab.id,x:params.xcoord(),y:params.ycoord(),link_url:clipped(CefString::from(&params.link_url()).to_string(),16384),selected_text:clipped(CefString::from(&params.selection_text()).to_string(),65536),editable:params.is_editable()==1 }); }
        }
    }
}
