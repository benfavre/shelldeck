use super::*;

// ---------------------------------------------------------------------------
// Non-text tab rendering + PDF loader
// ---------------------------------------------------------------------------

impl FileEditorView {
    pub(super) fn render_image_viewer(path: &std::path::Path) -> impl IntoElement {
        div()
            .flex()
            .flex_grow()
            .items_center()
            .justify_center()
            .size_full()
            .bg(ShellDeckColors::bg_primary())
            .child(
                img(path.to_string_lossy().to_string())
                    .object_fit(ObjectFit::Contain)
                    .max_w_full()
                    .max_h_full(),
            )
    }

    #[allow(clippy::too_many_arguments)] // PDF metadata fields rendered as a flat list
    pub(super) fn render_pdf_info(
        filename: &str,
        path: Option<&std::path::Path>,
        page_count: usize,
        file_size: u64,
        title: Option<&str>,
        author: Option<&str>,
        creator: Option<&str>,
        _handle: WeakEntity<Self>,
    ) -> impl IntoElement {
        let mut card = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(12.0))
            .p(px(32.0))
            .max_w(px(420.0))
            .rounded(px(8.0))
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border());

        // PDF icon badge
        card = card.child(
            div()
                .px(px(12.0))
                .py(px(6.0))
                .rounded(px(4.0))
                .bg(hsla(0.0, 0.7, 0.55, 0.15))
                .text_color(hsla(0.0, 0.7, 0.55, 1.0))
                .text_size(px(13.0))
                .font_weight(FontWeight::BOLD)
                .child(t!("file_editor.pdf.badge").to_string()),
        );

        // Filename
        card = card.child(
            div()
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(ShellDeckColors::text_primary())
                .child(filename.to_string()),
        );

        // Info rows
        card = card.child(Self::info_row(
            t!("file_editor.info.pages").as_ref(),
            &page_count.to_string(),
        ));
        card = card.child(Self::info_row(
            t!("file_editor.info.size").as_ref(),
            &format_file_size(file_size),
        ));
        if let Some(t) = title {
            if !t.is_empty() {
                card = card.child(Self::info_row(t!("file_editor.info.title").as_ref(), t));
            }
        }
        if let Some(a) = author {
            if !a.is_empty() {
                card = card.child(Self::info_row(t!("file_editor.info.author").as_ref(), a));
            }
        }
        if let Some(c) = creator {
            if !c.is_empty() {
                card = card.child(Self::info_row(t!("file_editor.info.creator").as_ref(), c));
            }
        }

        // Open externally button
        if let Some(p) = path {
            let path_owned = p.to_path_buf();
            card = card.child(
                div()
                    .id("open-pdf-external")
                    .mt(px(8.0))
                    .px(px(16.0))
                    .py(px(6.0))
                    .rounded(px(4.0))
                    .bg(ShellDeckColors::primary())
                    .text_color(ShellDeckColors::bg_primary())
                    .text_size(px(12.0))
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.9))
                    .child(t!("file_editor.pdf.open_external").to_string())
                    .on_click(move |_event, _window, _cx| {
                        let _ = open::that(&path_owned);
                    }),
            );
        }

        div()
            .flex()
            .flex_grow()
            .items_center()
            .justify_center()
            .size_full()
            .bg(ShellDeckColors::bg_primary())
            .child(card)
    }

    pub(super) fn render_binary_info(
        filename: &str,
        path: Option<&std::path::Path>,
        file_size: u64,
        _handle: WeakEntity<Self>,
    ) -> impl IntoElement {
        let ext = std::path::Path::new(filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("BIN")
            .to_uppercase();

        let mut card = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(12.0))
            .p(px(32.0))
            .max_w(px(420.0))
            .rounded(px(8.0))
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border());

        // Extension badge
        card = card.child(
            div()
                .px(px(12.0))
                .py(px(6.0))
                .rounded(px(4.0))
                .bg(hsla(0.0, 0.0, 0.5, 0.15))
                .text_color(ShellDeckColors::text_muted())
                .text_size(px(13.0))
                .font_weight(FontWeight::BOLD)
                .child(ext),
        );

        // Filename
        card = card.child(
            div()
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(ShellDeckColors::text_primary())
                .child(filename.to_string()),
        );

        // Message
        card = card.child(
            div()
                .text_size(px(12.0))
                .text_color(ShellDeckColors::text_muted())
                .child(t!("file_editor.binary.not_text").to_string()),
        );

        // Info
        card = card.child(Self::info_row(
            t!("file_editor.info.size").as_ref(),
            &format_file_size(file_size),
        ));
        if let Some(p) = path {
            card = card.child(Self::info_row(
                t!("file_editor.info.path").as_ref(),
                &p.to_string_lossy(),
            ));
        }

        // Open externally button
        if let Some(p) = path {
            let path_owned = p.to_path_buf();
            card = card.child(
                div()
                    .id("open-binary-external")
                    .mt(px(8.0))
                    .px(px(16.0))
                    .py(px(6.0))
                    .rounded(px(4.0))
                    .bg(ShellDeckColors::primary())
                    .text_color(ShellDeckColors::bg_primary())
                    .text_size(px(12.0))
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.9))
                    .child(t!("file_editor.binary.open_external").to_string())
                    .on_click(move |_event, _window, _cx| {
                        let _ = open::that(&path_owned);
                    }),
            );
        }

        div()
            .flex()
            .flex_grow()
            .items_center()
            .justify_center()
            .size_full()
            .bg(ShellDeckColors::bg_primary())
            .child(card)
    }

    pub(super) fn info_row(label: &str, value: &str) -> impl IntoElement {
        div()
            .flex()
            .w_full()
            .gap(px(8.0))
            .text_size(px(12.0))
            .child(
                div()
                    .w(px(60.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!("{}:", label)),
            )
            .child(
                div()
                    .flex_grow()
                    .text_color(ShellDeckColors::text_primary())
                    .child(value.to_string()),
            )
    }

    pub(super) fn load_pdf_info(path: &std::path::Path) -> Option<PdfInfo> {
        let file_size = std::fs::metadata(path).ok()?.len();
        let doc = lopdf::Document::load(path).ok()?;
        let page_count = doc.get_pages().len();

        let (title, author, creator) = if let Ok(info_dict) = doc
            .trailer
            .get(b"Info")
            .and_then(|v| doc.dereference(v))
            .and_then(|(_, v)| v.as_dict())
        {
            let get_str = |key: &[u8]| -> Option<String> {
                info_dict.get(key).ok().and_then(|v| match v {
                    lopdf::Object::String(bytes, _) => String::from_utf8(bytes.clone()).ok(),
                    _ => None,
                })
            };
            (get_str(b"Title"), get_str(b"Author"), get_str(b"Creator"))
        } else {
            (None, None, None)
        };

        Some(PdfInfo {
            page_count,
            file_size,
            title,
            author,
            creator,
        })
    }
}

fn format_file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
