//! Display-only folding of encoded payloads. Retained/provider text stays exact.
use super::*;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum TranscriptId {
    Message(Uuid),
    Trace(Uuid),
    Native { snapshot: String, index: u64 },
}

impl TranscriptId {
    pub(super) fn selector(&self, part: &str) -> String {
        match self {
            Self::Message(id) => format!("agent-message-{id}-{part}"),
            Self::Trace(id) => format!("agent-trace-{id}-{part}"),
            Self::Native { snapshot, index } => format!("agent-native-{snapshot}-{index}-{part}"),
        }
    }
}

/// Recognize long base64/hex tokens, including wrapped and diff-prefixed lines.
/// Require enough data to avoid folding hashes, paths and ordinary source text.
fn encoded_spans(text: &str) -> Vec<Range<usize>> {
    fn encoded_byte(byte: u8) -> bool {
        byte.is_ascii_alphanumeric() || b"+/=_-".contains(&byte)
    }
    fn opaque(text: &str, minimum: usize) -> bool {
        let bytes = text.bytes().filter(|byte| !byte.is_ascii_whitespace());
        let mut count = 0;
        let (mut lower, mut upper, mut digit, mut hex) = (false, false, false, true);
        for byte in bytes {
            count += 1;
            lower |= byte.is_ascii_lowercase();
            upper |= byte.is_ascii_uppercase();
            digit |= byte.is_ascii_digit();
            hex &= byte.is_ascii_hexdigit() || b"+-=".contains(&byte);
        }
        count >= minimum && (hex || (lower && upper && digit))
    }
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut pending: Option<Range<usize>> = None;
    let mut offset = 0;
    while offset < bytes.len() {
        let start = offset;
        while offset < bytes.len() && encoded_byte(bytes[offset]) {
            offset += 1;
        }
        let continues_line = pending.as_ref().is_some_and(|range| {
            let gap = &text[range.end..start];
            gap.contains('\n') && gap.bytes().all(|byte| byte.is_ascii_whitespace())
        });
        // A wrapped block can finish with fewer than 64 characters. Include
        // that tail only when it fills its line and follows encoded lines;
        // never consume the first word of the explanation after a payload.
        let short_tail = offset - start < 64
            && continues_line
            && opaque(&text[start..offset], 8)
            && text[offset..]
                .split(['\r', '\n'])
                .next()
                .is_some_and(|line| line.trim().is_empty());
        if offset - start >= 64 || short_tail {
            if let Some(range) = pending.as_mut().filter(|_| continues_line) {
                range.end = offset;
            } else {
                if let Some(range) = pending
                    .take()
                    .filter(|range| opaque(&text[range.clone()], 256))
                {
                    spans.push(range);
                }
                pending = Some(start..offset);
            }
        } else if let Some(range) = pending
            .take()
            .filter(|range| opaque(&text[range.clone()], 256))
        {
            spans.push(range);
        }
        // Token boundaries are ASCII, so slicing never splits UTF-8.
        while offset < bytes.len() && !encoded_byte(bytes[offset]) {
            offset += 1;
        }
    }
    if let Some(range) = pending.filter(|range| opaque(&text[range.clone()], 256)) {
        spans.push(range);
    }
    spans
}

fn fold_encoded(text: &str, marker: impl Fn(usize) -> String) -> Option<String> {
    let spans = encoded_spans(text);
    if spans.is_empty() {
        return None;
    }
    let mut folded = String::new();
    let mut start = 0;
    for span in spans {
        folded.push_str(&text[start..span.start]);
        folded.push_str(&marker(span.len()));
        start = span.end;
    }
    folded.push_str(&text[start..]);
    Some(folded)
}

impl AgentConsoleView {
    /// Resolve the currently retained source, rather than a string captured by
    /// an earlier render. Delayed clicks cannot reveal/copy another page or chat.
    fn original_transcript(&self, session_id: Uuid, id: &TranscriptId) -> Option<&str> {
        let session = self.sessions.get(session_id)?;
        match id {
            TranscriptId::Message(id) => session
                .messages
                .iter()
                .find(|message| message.id == *id)
                .map(|message| message.text.as_str()),
            TranscriptId::Trace(id) => {
                match &session.trace.iter().find(|trace| trace.id == *id)?.detail {
                    AgentTraceKind::Command { summary, .. }
                    | AgentTraceKind::Test { summary, .. }
                    | AgentTraceKind::Tool { summary, .. } => summary.as_deref(),
                    AgentTraceKind::Diff { preview, .. } => preview.as_deref(),
                    _ => None,
                }
            }
            TranscriptId::Native { snapshot, index } => {
                let reference = session.remote_history.as_ref()?;
                let page = self.native_history.get(&session_id)?.page.as_ref()?;
                if page.snapshot != *snapshot
                    || page.provider != reference.provider
                    || page.session_id != reference.session_id
                    || page.workdir != reference.workdir
                {
                    return None;
                }
                page.entries
                    .iter()
                    .find(|entry| entry.index == *index)
                    .map(|entry| entry.text.as_str())
            }
        }
    }

    fn toggle_raw_transcript(
        &mut self,
        session_id: Uuid,
        id: &TranscriptId,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.sessions.selected_id() != Some(session_id)
            || self
                .original_transcript(session_id, id)
                .is_none_or(|text| encoded_spans(text).is_empty())
        {
            return false;
        }
        let Some(ui) = self.session_ui.get_mut(&session_id) else {
            return false;
        };
        if !ui.raw_transcripts.remove(id) {
            ui.raw_transcripts.insert(id.clone());
        }
        cx.notify();
        true
    }

    fn copy_original_transcript(
        &self,
        session_id: Uuid,
        id: &TranscriptId,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.sessions.selected_id() != Some(session_id) {
            return false;
        }
        let Some(original) = self.original_transcript(session_id, id) else {
            return false;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(original.to_owned()));
        true
    }

    pub(super) fn render_transcript_text(
        &self,
        id: TranscriptId,
        text: &str,
        prose: bool,
        window: &Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let folded = fold_encoded(text, |bytes| {
            t!("agents.transcript.encoded_hidden", bytes = bytes).to_string()
        });
        let session_id = self.sessions.selected_id();
        let show_raw = session_id
            .and_then(|session| self.session_ui.get(&session))
            .is_some_and(|ui| ui.raw_transcripts.contains(&id));
        let display = if show_raw {
            text
        } else {
            folded.as_deref().unwrap_or(text)
        };
        let mut body = div()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .debug_selector(|| id.selector(if show_raw { "raw" } else { "folded" }));
        if prose && !show_raw {
            body = body.child(
                Markdown::new(display.to_owned())
                    .base_font_size(px(14.0).to_pixels(window.rem_size()))
                    .compact()
                    .w_full()
                    .min_w_0()
                    .whitespace_normal(),
            );
        } else {
            body = body
                .font_family("JetBrains Mono")
                .text_size(px(if matches!(&id, TranscriptId::Trace(_)) {
                    10.0
                } else {
                    12.0
                }))
                .line_height(relative(1.4))
                .text_color(ShellDeckColors::text_muted())
                .whitespace_normal()
                .child(display.to_owned());
        }
        if folded.is_some() {
            let label = if show_raw {
                t!("agents.transcript.hide_raw").to_string()
            } else {
                t!("agents.transcript.show_raw").to_string()
            };
            let copy_id = id.clone();
            let toggle = div()
                .flex()
                .flex_wrap()
                .gap(px(6.0))
                .child(
                    // The containing message/trace supplies the stable ID scope.
                    Button::new("encoded-transcript-toggle", label)
                        .debug_selector(|| id.selector("toggle"))
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(session_id) = session_id {
                                this.toggle_raw_transcript(session_id, &id, cx);
                            }
                        })),
                )
                .child(
                    Button::new(
                        "encoded-transcript-copy",
                        t!("agents.transcript.copy_raw").to_string(),
                    )
                    .debug_selector(|| copy_id.selector("copy"))
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(session_id) = session_id {
                            this.copy_original_transcript(session_id, &copy_id, cx);
                        }
                    })),
                );
            body = div()
                .w_full()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(toggle)
                .child(body);
        }
        body.into_any_element()
    }
}

#[cfg(test)]
#[path = "transcript_scenarios.rs"]
mod scenarios;

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-2842 / SDUC-563, SDUC-564
    #[::core::prelude::v1::test]
    fn encoded_transcripts_fold_diff_data_without_losing_surrounding_text() {
        let data = "aB3+/xY9".repeat(40);
        let patch = format!("diff --git a/image.txt b/image.txt\n@@ -1 +1,4 @@\n-old\n+{data}\n+{data}\n+{data}\n+{data}\nKeep this explanation: é🌍\n");
        let folded = fold_encoded(&patch, |bytes| format!("[hidden {bytes} bytes]")).unwrap();
        assert!(folded.starts_with("diff --git a/image.txt b/image.txt\n@@ -1 +1,4 @@\n-old\n"));
        assert_eq!(folded.matches("[hidden ").count(), 1);
        assert!(folded.ends_with("\nKeep this explanation: é🌍\n"));
        assert!(!folded.contains(&data));

        for text in [
            ("aB3+/xY9".repeat(9) + "\r\n").repeat(8),
            format!("{{\"image_url\":\"data:image/png;base64,{data}\"}}"),
            format!("Before {} after {} end", "ab12".repeat(80), data),
        ] {
            let folded = fold_encoded(&text, |_| "[hidden]".into()).unwrap();
            assert!(!folded.contains(&data));
            assert!(folded.len() < text.len());
        }
        for text in [
            "Ordinary text, é🌍, a link https://example.com and src/main.rs".to_string(),
            format!("commit {}\n+let message = \"Hello\";", "a1b2".repeat(16)),
            "- normal diff\n+ readable changes\n```rust\nlet value = 123;\n```".into(),
        ] {
            assert!(fold_encoded(&text, |_| panic!("readable text must stay intact")).is_none());
        }
    }
}
