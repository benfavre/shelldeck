//! Transcript projection stores identities/indices, never copies of retained payloads.
//! GPUI's variable-height list measures only the viewport and a small overdraw.
use super::*;
use std::{cell::RefCell, rc::Rc};
mod markdown;

pub(super) enum TimelineItemRef<'a> {
    Message(&'a AgentMessage),
    Trace(&'a AgentTraceEvent),
    CompletedWork(Vec<&'a AgentTraceEvent>),
    Boundary(&'a shelldeck_core::agent_session::AgentHistoryBoundary),
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum Kind {
    Message(usize),
    MessagePart(usize, usize),
    Trace(usize),
    Boundary(usize),
    Group(Rc<Vec<usize>>),
    HistoryHeader,
    HistoryEntry(usize),
    HistoryFooter,
    Footer,
}
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Row {
    pub(super) id: Uuid,
    pub(super) kind: Kind,
    pub(super) group: Option<Uuid>,
    stamp: (usize, u64, bool),
}
type RowKey = (Uuid, std::mem::Discriminant<Kind>, usize);
struct RetainedRow {
    view: Entity<TimelineRowView>,
    seen: u64,
}
struct TimelineRowView {
    parent: WeakEntity<AgentConsoleView>,
    session: Uuid,
    row: Row,
    wide: bool,
    provider: AgentProvider,
    index: usize,
    last: bool,
}
impl Render for TimelineRowView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = self
            .parent
            .update(cx, |parent, cx| {
                if parent.sessions.selected_id() != Some(self.session) {
                    return div().into_any_element();
                }
                parent.render_indexed_timeline_row(
                    &self.row,
                    activity::ActivityFrame::default(),
                    false,
                    window,
                    cx,
                )
            })
            .unwrap_or_else(|_| div().into_any_element());
        row_shell(
            body,
            self.wide,
            self.index,
            self.last,
            matches!(self.row.kind, Kind::MessagePart(_, part) if part > 0),
        )
    }
}
fn row_shell(
    body: AnyElement,
    wide: bool,
    index: usize,
    last: bool,
    continuation: bool,
) -> AnyElement {
    div()
        .w_full()
        .px(if wide { px(24.0) } else { px(12.0) })
        .pt(px(if continuation {
            0.0
        } else if index == 0 {
            16.0
        } else {
            8.0
        }))
        .pb(px(if last { 16.0 } else { 0.0 }))
        .child(div().w_full().max_w(px(800.0)).mx_auto().child(body))
        .into_any_element()
}
fn row_key(row: &Row) -> RowKey {
    (
        row.id,
        std::mem::discriminant(if matches!(row.kind, Kind::MessagePart(_, _)) {
            &Kind::Message(0)
        } else {
            &row.kind
        }),
        match row.kind {
            Kind::MessagePart(_, part) => part,
            _ => 0,
        },
    )
}

type PaintedRows = Rc<RefCell<Vec<(usize, Bounds<Pixels>)>>>;

pub(super) struct TimelineUi {
    pub(super) list: ListState,
    pub(super) virtualized: bool,
    session: Option<Uuid>,
    pub(super) rows: Rc<Vec<Row>>,
    pub(super) following: bool,
    pub(super) epoch: u64,
    frame: u64,
    rem: Pixels,
    painted: PaintedRows,
    row_views: RefCell<HashMap<RowKey, RetainedRow>>,
    typography: Option<TextStyle>,
    locale: String,
    explicit_top: bool,
    markdown: markdown::MarkdownRows,
}
impl Default for TimelineUi {
    fn default() -> Self {
        Self {
            list: ListState::new(0, ListAlignment::Bottom, gpui::px(160.0)),
            virtualized: false,
            session: None,
            rows: Rc::default(),
            following: true,
            epoch: 0,
            frame: 0,
            rem: gpui::px(0.0),
            painted: Rc::default(),
            row_views: RefCell::default(),
            typography: None,
            locale: String::new(),
            explicit_top: false,
            markdown: Default::default(),
        }
    }
}

impl Row {
    fn estimated_height(&self, rem: Pixels) -> Pixels {
        let height = match self.kind {
            Kind::Message(_) => 54.0 + (self.stamp.0 / 80).clamp(1, 120) as f32 * 18.0,
            Kind::MessagePart(_, _) => 32.0 + (self.stamp.0 / 80).clamp(1, 120) as f32 * 18.0,
            Kind::Trace(_) => 150.0,
            Kind::Group(_) => 58.0,
            Kind::Boundary(_) => 36.0,
            Kind::HistoryHeader => 150.0,
            Kind::HistoryEntry(_) => 54.0 + (self.stamp.0 / 160).clamp(1, 120) as f32 * 18.0,
            Kind::HistoryFooter => 50.0,
            Kind::Footer => 100.0,
        };
        gpui::px(height * rem.to_f64() as f32 / 16.0)
    }
}

fn project(
    session: &AgentSession,
    epoch: u64,
    raw: &HashSet<transcript::TranscriptId>,
    focused: Option<Uuid>,
) -> Vec<Row> {
    let current = session.current_turn_id();
    let mut ordered =
        Vec::with_capacity(session.messages.len() + session.trace.len() + session.boundaries.len());
    for (index, message) in session.messages.iter().enumerate() {
        ordered.push((
            message.sequence,
            message.at_ms,
            Row {
                id: message.id,
                kind: Kind::Message(index),
                group: None,
                stamp: (
                    message.text.len(),
                    if message.turn_id == current { epoch } else { 0 },
                    raw.contains(&transcript::TranscriptId::Message(message.id)),
                ),
            },
        ));
    }
    for (index, trace) in session.trace.iter().enumerate() {
        if !trace_is_visible(session, trace) {
            continue;
        }
        let stamp = if trace.turn_id == current { epoch } else { 0 };
        ordered.push((
            trace.sequence,
            trace.at_ms,
            Row {
                id: trace.id,
                kind: Kind::Trace(index),
                group: None,
                stamp: (
                    usize::from(focused == Some(trace.id)),
                    stamp,
                    raw.contains(&transcript::TranscriptId::Trace(trace.id)),
                ),
            },
        ));
    }
    for (index, boundary) in session.boundaries.iter().enumerate() {
        ordered.push((
            boundary.sequence,
            boundary.at_ms,
            Row {
                id: boundary.id,
                kind: Kind::Boundary(index),
                group: None,
                stamp: (0, 0, false),
            },
        ));
    }
    ordered.sort_by(|a, b| match (a.0, b.0) {
        (0, 0) => a.1.cmp(&b.1),
        (0, _) => std::cmp::Ordering::Less,
        (_, 0) => std::cmp::Ordering::Greater,
        _ => a.0.cmp(&b.0),
    });
    let mut rows: Vec<Row> = Vec::with_capacity(ordered.len());
    for (_, _, mut row) in ordered {
        if let Kind::Trace(index) = row.kind {
            let trace = &session.trace[index];
            if activity::completed_work(trace) {
                if let Some(Row {
                    kind: Kind::Group(indices),
                    ..
                }) = rows.last_mut()
                {
                    let last = &session.trace[*indices.last().unwrap()];
                    if last.turn_id.is_some() && last.turn_id == trace.turn_id {
                        Rc::make_mut(indices).push(index);
                        continue;
                    }
                }
                row.kind = Kind::Group(Rc::new(vec![index]));
            }
        }
        rows.push(row);
    }
    rows
}

impl TimelineUi {
    #[cfg(test)]
    pub(super) fn take_markdown_work(&self) -> usize {
        self.markdown.rendered_blocks.replace(0)
    }
    #[cfg(test)]
    pub(super) fn row_cache_len(&self) -> usize {
        self.row_views.borrow().len()
    }
    pub(super) fn pin(&mut self, scroll: &ScrollHandle) {
        pin_to_latest(scroll);
        self.following = true;
        self.explicit_top = false;
        if self.virtualized {
            self.list.scroll_to(ListOffset {
                item_ix: self.rows.len(),
                offset_in_item: gpui::px(0.0),
            });
        }
    }
    pub(super) fn follow(&self, scroll: &ScrollHandle) {
        if self.virtualized {
            if self.following {
                self.list.scroll_to(ListOffset {
                    item_ix: self.rows.len(),
                    offset_in_item: gpui::px(0.0),
                });
            }
        } else {
            follow_latest_if_at_end(scroll);
        }
    }
    pub(super) fn at_end(&self, scroll: &ScrollHandle) -> bool {
        if self.virtualized {
            self.following
        } else {
            (scroll.max_offset().height + scroll.offset().y).abs() <= gpui::px(4.0)
        }
    }
    pub(super) fn top(&mut self, scroll: &ScrollHandle) {
        scroll.set_offset(point(gpui::px(0.0), gpui::px(0.0)));
        self.following = false;
        self.explicit_top = true;
        self.list.scroll_to(ListOffset {
            item_ix: 0,
            offset_in_item: gpui::px(0.0),
        });
    }
    pub(super) fn pause_following(&mut self) {
        self.following = false;
        // Bottom alignment can represent the position as the end sentinel.
        // Resolve that to a visible row before a disclosure changes its height.
        if self.list.logical_scroll_top().item_ix >= self.rows.len() {
            let viewport = self.list.viewport_bounds();
            if let Some((index, bounds)) = self
                .painted
                .borrow()
                .iter()
                .filter(|(_, bounds)| {
                    bounds.bottom() > viewport.top() && bounds.top() < viewport.bottom()
                })
                .min_by(|(_, a), (_, b)| a.top().partial_cmp(&b.top()).unwrap())
            {
                self.list.scroll_to(ListOffset {
                    item_ix: *index,
                    offset_in_item: (viewport.top() - bounds.top()).max(gpui::px(0.0)),
                });
            }
        }
    }
    pub(super) fn project(
        &mut self,
        session: &AgentSession,
        ui: &AgentSessionUi,
        focused: Option<Uuid>,
        rem: Pixels,
        native: Option<&native_history::NativeHistoryUi>,
        typography: &TextStyle,
    ) -> Vec<Row> {
        let rows = if native.is_some_and(|state| state.fresh_view) {
            Vec::new()
        } else {
            project(session, self.epoch, &ui.raw_transcripts, focused)
        };
        let native_page = native.and_then(|state| state.page.as_ref());
        let native_count = native_page.map_or(0, |page| page.entries.len());
        let expanded = |indices: &[usize]| {
            indices.iter().any(|index| {
                ui.expanded_activity.contains(&session.trace[*index].id)
                    || focused == Some(session.trace[*index].id)
            })
        };
        let displayed = rows
            .iter()
            .map(|row| match &row.kind {
                Kind::Group(indices) if expanded(indices) => 1 + indices.len(),
                _ => 1,
            })
            .sum::<usize>();
        if self.session != Some(session.id) {
            self.virtualized = false;
            self.following = true;
            self.rows = Rc::default();
            self.list.reset(0);
            self.session = Some(session.id);
            self.row_views.borrow_mut().clear();
            self.markdown = Default::default();
        }
        // The same measured-row path serves short chats: an eager small-chat
        // tree still rebuilds every message during each animation and scroll.
        self.virtualized |= displayed > 0 || session.remote_history.is_some();
        if !self.virtualized {
            return rows;
        }
        let mut flattened = Vec::with_capacity(displayed + 1);
        for mut row in rows {
            if let Kind::Message(index) = row.kind {
                let message = &session.messages[index];
                if !row.stamp.2 {
                    if let Some(parts) = self.markdown.parts(message, row.stamp.1) {
                        for part in 0..parts {
                            let mut fragment = row.clone();
                            fragment.kind = Kind::MessagePart(index, part);
                            fragment.stamp.0 = message.text.len() / parts;
                            flattened.push(fragment);
                        }
                        continue;
                    }
                }
            }
            if let Kind::Group(indices) = &row.kind {
                row.stamp.0 = indices.len();
                row.stamp.2 = expanded(indices);
                let children = if row.stamp.2 {
                    Some(indices.clone())
                } else {
                    None
                };
                let group = row.id;
                flattened.push(row);
                if let Some(indices) = children {
                    for index in indices.iter().copied() {
                        let trace = &session.trace[index];
                        flattened.push(Row {
                            id: trace.id,
                            kind: Kind::Trace(index),
                            group: Some(group),
                            stamp: (
                                usize::from(focused == Some(trace.id)),
                                if trace.turn_id == session.current_turn_id() {
                                    self.epoch
                                } else {
                                    0
                                },
                                ui.raw_transcripts
                                    .contains(&transcript::TranscriptId::Trace(trace.id)),
                            ),
                        });
                    }
                }
            } else {
                flattened.push(row);
            }
        }
        self.frame = self.frame.wrapping_add(1);
        flattened.push(Row {
            id: Uuid::nil(),
            kind: Kind::Footer,
            group: None,
            stamp: (0, self.frame, false),
        });
        if session.remote_history.is_some() {
            let mut prefix = Vec::with_capacity(native_count + 2);
            prefix.push(Row {
                id: Uuid::nil(),
                kind: Kind::HistoryHeader,
                group: None,
                stamp: (0, self.frame, false),
            });
            if let Some(page) = native_page {
                use std::hash::{Hash, Hasher};
                let mut hash = std::collections::hash_map::DefaultHasher::new();
                page.snapshot.hash(&mut hash);
                let revision = hash.finish();
                for (index, entry) in page.entries.iter().enumerate() {
                    prefix.push(Row {
                        id: Uuid::from_u128(entry.index as u128),
                        kind: Kind::HistoryEntry(index),
                        group: None,
                        stamp: (
                            entry.text.len() * 2
                                + usize::from(native.is_some_and(|state| {
                                    state.expanded_entries.contains(&entry.index)
                                })),
                            revision,
                            ui.raw_transcripts
                                .contains(&transcript::TranscriptId::Native {
                                    snapshot: page.snapshot.clone(),
                                    index: entry.index,
                                }),
                        ),
                    });
                }
            }
            prefix.push(Row {
                id: Uuid::nil(),
                kind: Kind::HistoryFooter,
                group: None,
                stamp: (0, self.frame, false),
            });
            prefix.extend(flattened);
            flattened = prefix;
        }
        let old_top = self.list.logical_scroll_top();
        let anchor = self.rows.get(old_top.item_ix).map(row_key);
        let current_locale = rust_i18n::locale();
        let locale: &str = current_locale.as_ref();
        if self.rem != rem || self.typography.as_ref() != Some(typography) || self.locale != locale
        {
            self.list.splice_with_estimates(
                0..self.rows.len(),
                flattened.iter().map(|row| row.estimated_height(rem)),
            );
            self.rem = rem;
            self.typography = Some(typography.clone());
            self.locale = locale.to_string();
        } else if self.rows.len() == flattened.len()
            && self.rows.iter().zip(&flattened).all(|(a, b)| {
                a.id == b.id && std::mem::discriminant(&a.kind) == std::mem::discriminant(&b.kind)
            })
        {
            // Header/footer controls change independently of immutable native
            // entries. Keep measured sizes between separate changed regions.
            let mut start = None;
            for index in 0..=flattened.len() {
                let changed = index < flattened.len() && self.rows[index] != flattened[index];
                if changed {
                    start.get_or_insert(index);
                } else if let Some(start) = start.take() {
                    self.list.splice_with_estimates(
                        start..index,
                        flattened[start..index]
                            .iter()
                            .map(|row| row.estimated_height(rem)),
                    );
                }
            }
        } else {
            let prefix = self
                .rows
                .iter()
                .zip(&flattened)
                .take_while(|(a, b)| a == b)
                .count();
            let suffix = self.rows[prefix..]
                .iter()
                .rev()
                .zip(flattened[prefix..].iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
            self.list.splice_with_estimates(
                prefix..self.rows.len() - suffix,
                flattened[prefix..flattened.len() - suffix]
                    .iter()
                    .map(|row| row.estimated_height(rem)),
            );
        }
        if self.explicit_top {
            self.list.scroll_to(ListOffset {
                item_ix: 0,
                offset_in_item: gpui::px(0.0),
            });
            self.following = false;
            self.explicit_top = false;
        } else if self.following {
            self.list.scroll_to(ListOffset {
                item_ix: flattened.len(),
                offset_in_item: gpui::px(0.0),
            });
        } else if let Some(index) =
            anchor.and_then(|key| flattened.iter().position(|row| row_key(row) == key))
        {
            self.list.scroll_to(ListOffset {
                item_ix: index,
                offset_in_item: old_top.offset_in_item,
            });
        }
        let valid = flattened.iter().map(row_key).collect::<HashSet<_>>();
        self.row_views
            .borrow_mut()
            .retain(|key, _| valid.contains(key));
        self.markdown.retain(session);
        self.rows = Rc::new(flattened);
        Vec::new()
    }
    pub(super) fn floating_group(&self) -> Option<Uuid> {
        let viewport = self.list.viewport_bounds();
        self.painted
            .borrow()
            .iter()
            .filter(|(_, bounds)| {
                bounds.bottom() > viewport.top() && bounds.top() < viewport.bottom()
            })
            .min_by(|(_, a), (_, b)| {
                a.top()
                    .partial_cmp(&b.top())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .and_then(|(index, _)| self.rows.get(*index))
            .and_then(|row| row.group)
    }
    pub(super) fn reveal(&mut self, id: Uuid, group: bool) {
        if let Some(index) = self.rows.iter().position(|row| {
            row.id == id
                && if group {
                    matches!(row.kind, Kind::Group(_))
                } else {
                    matches!(row.kind, Kind::Trace(_))
                }
        }) {
            self.following = false;
            self.list.scroll_to(ListOffset {
                item_ix: index,
                offset_in_item: gpui::px(0.0),
            });
        }
    }
}

impl AgentConsoleView {
    pub(super) fn render_indexed_timeline_row(
        &self,
        row: &Row,
        frame: activity::ActivityFrame,
        inline: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(session) = self.sessions.selected() else {
            return div().into_any_element();
        };
        let item = match &row.kind {
            Kind::Message(index) => session.messages.get(*index).map(TimelineItemRef::Message),
            Kind::MessagePart(index, part) => {
                return self.render_message_part(
                    &session.messages[*index],
                    *part,
                    row.stamp.1,
                    window,
                    cx,
                );
            }
            Kind::Trace(index) => session.trace.get(*index).map(TimelineItemRef::Trace),
            Kind::Boundary(index) => session
                .boundaries
                .get(*index)
                .map(TimelineItemRef::Boundary),
            Kind::Group(indices) => Some(TimelineItemRef::CompletedWork(
                indices
                    .iter()
                    .filter_map(|index| session.trace.get(*index))
                    .collect(),
            )),
            Kind::HistoryHeader | Kind::HistoryFooter => {
                return self.render_native_history_part(
                    session,
                    if matches!(row.kind, Kind::HistoryHeader) {
                        native_history::HistoryPart::Header
                    } else {
                        native_history::HistoryPart::Footer
                    },
                    window,
                    cx,
                )
            }
            Kind::HistoryEntry(index) => {
                return self.render_native_history_entry(session.id, *index, window, cx)
            }
            Kind::Footer => return self.render_timeline_footer(session, cx),
        };
        match item {
            Some(item) => self.render_timeline_item_ref(item, frame, inline, window, cx),
            None => div().into_any_element(),
        }
    }
    pub(super) fn render_virtual_timeline(&self, wide: bool, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.timeline.rows.clone();
        self.timeline.painted.borrow_mut().clear();
        let painted = self.timeline.painted.clone();
        let session = self.sessions.selected_id();
        let parent = cx.entity().downgrade();
        let provider = self.displayed_context(cx).provider;
        self.timeline.list.set_scroll_handler({
            let parent = parent.clone();
            move |event, _, cx| {
                let _ = parent.update(cx, |view, cx| {
                    if view.sessions.selected_id() == session {
                        let following = !event.is_scrolled;
                        if view.timeline.following != following {
                            view.timeline.following = following;
                            cx.notify();
                        }
                    }
                });
            }
        });
        let transcript = list_with_cached_sizes(
            self.timeline.list.clone(),
            move |index, measured, window, cx| {
                let item = parent
                    .update(cx, |view, cx| {
                        if view.sessions.selected_id() != session {
                            return div().into_any_element();
                        }
                        let Some(row) = rows.get(index) else {
                            return div().into_any_element();
                        };
                        let cacheable = match row.kind {
                            Kind::Footer | Kind::HistoryHeader | Kind::HistoryFooter => false,
                            Kind::Trace(trace) => view
                                .sessions
                                .selected()
                                .and_then(|session| session.trace.get(trace))
                                .is_some_and(|trace| {
                                    !matches!(
                                        activity::operation_status(&trace.detail),
                                        Some(AgentTraceStatus::Running | AgentTraceStatus::Pending)
                                    )
                                }),
                            _ => true,
                        };
                        if !cacheable {
                            return row_shell(
                                view.render_indexed_timeline_row(
                                    row,
                                    activity::ActivityFrame::default(),
                                    false,
                                    window,
                                    cx,
                                ),
                                wide,
                                index,
                                index + 1 == rows.len(),
                                matches!(row.kind, Kind::MessagePart(_, part) if part > 0),
                            );
                        }
                        let mut retained = view.timeline.row_views.borrow_mut();
                        if retained.len() >= 64 && !retained.contains_key(&row_key(row)) {
                            if let Some(key) = retained
                                .iter()
                                .min_by_key(|(_, entry)| entry.seen)
                                .map(|(key, _)| *key)
                            {
                                retained.remove(&key);
                            }
                        }
                        let entry = retained.entry(row_key(row)).or_insert_with(|| RetainedRow {
                            seen: view.timeline.frame,
                            view: cx.new(|_| TimelineRowView {
                                parent: parent.clone(),
                                session: session.expect("selected session"),
                                row: row.clone(),
                                wide,
                                provider,
                                index,
                                last: index + 1 == rows.len(),
                            }),
                        });
                        entry.seen = view.timeline.frame;
                        let changed = entry.view.read(cx).row != *row
                            || entry.view.read(cx).wide != wide
                            || entry.view.read(cx).provider != provider
                            || entry.view.read(cx).index != index
                            || entry.view.read(cx).last != (index + 1 == rows.len());
                        if changed {
                            entry.view.update(cx, |item, cx| {
                                item.row = row.clone();
                                item.wide = wide;
                                item.provider = provider;
                                item.index = index;
                                item.last = index + 1 == rows.len();
                                cx.notify();
                            });
                        }
                        let item = AnyView::from(entry.view.clone());
                        if let Some(measured) = measured.filter(|_| !changed) {
                            item.cached(StyleRefinement::default().w_full().h(measured.height))
                                .into_any_element()
                        } else {
                            item.into_any_element()
                        }
                    })
                    .unwrap_or_else(|_| div().into_any_element());
                let frame = painted.clone();
                div()
                    .w_full()
                    .child(item)
                    .on_children_prepainted(move |bounds, _, _| {
                        if let Some(bounds) = bounds.first() {
                            frame.borrow_mut().push((index, *bounds));
                        }
                    })
                    .into_any_element()
            },
        )
        .flex_1()
        .min_h_0()
        .w_full();
        div()
            .debug_selector(|| "agent-timeline-scroll".into())
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(transcript)
            .into_any_element()
    }
}

impl AgentConsoleView {
    fn render_timeline_item_ref(
        &self,
        item: TimelineItemRef<'_>,
        activity_frame: activity::ActivityFrame,
        inline: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let session_id = self.sessions.selected_id().expect("selected session");
        let provider = self.displayed_context(cx).provider;
        match item {
            TimelineItemRef::Message(message) => {
                let color = match message.role {
                    AgentMessageRole::User => ShellDeckColors::text_muted(),
                    AgentMessageRole::Agent => ShellDeckColors::text_muted(),
                    AgentMessageRole::Error => ShellDeckColors::error(),
                };
                let label = match message.role {
                    AgentMessageRole::User => t!("agents.role.you").to_string(),
                    AgentMessageRole::Agent => message
                        .provider
                        .unwrap_or(provider)
                        .display_name()
                        .to_string(),
                    AgentMessageRole::Error => t!("agents.activity.title").to_string(),
                };

                div()
                    .id(("agent-message", uuid_key(message.id)))
                    .debug_selector(|| format!("agent-message-{}", message.id))
                    .flex_shrink_0()
                    .w_full()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .py(px(10.0))
                    .when(message.role == AgentMessageRole::User, |message| {
                        message
                            .w(relative(0.85))
                            .ml_auto()
                            .px(px(16.0))
                            .rounded(px(12.0))
                            .bg(ShellDeckColors::bg_surface())
                    })
                    .when(message.role == AgentMessageRole::Error, |message| {
                        message
                            .px(px(9.0))
                            .rounded(px(5.0))
                            .bg(ShellDeckColors::error().opacity(0.10))
                    })
                    .child(
                        div()
                            .mb(px(4.0))
                            .text_size(px(10.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(color)
                            .child(label),
                    )
                    .child(self.render_transcript_text(
                        transcript::TranscriptId::Message(message.id),
                        &message.text,
                        true,
                        window,
                        cx,
                    ))
                    .child(self.render_message_actions(message, cx))
                    .into_any_element()
            }
            TimelineItemRef::Boundary(boundary) => {
                let key=match boundary.reason {
                            shelldeck_core::agent_session::AgentHistoryBoundaryReason::ContextChanged=>"agents.parity.context_boundary",
                            shelldeck_core::agent_session::AgentHistoryBoundaryReason::ApplicationRestart=>"agents.parity.restart_boundary",
                            shelldeck_core::agent_session::AgentHistoryBoundaryReason::ConversationNotResumed=>"agents.parity.fresh_boundary",
                            shelldeck_core::agent_session::AgentHistoryBoundaryReason::ForkedConversation=>"agents.parity.fork_boundary",
                        };

                div()
                    .py(px(9.0))
                    .border_y_1()
                    .border_color(ShellDeckColors::border())
                    .text_size(px(10.5))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!(key).to_string())
                    .into_any_element()
            }
            TimelineItemRef::CompletedWork(traces) => {
                self.render_completed_work(session_id, &traces, activity_frame, inline, window, cx)
            }
            TimelineItemRef::Trace(trace) => {
                self.render_trace(trace, self.focused_trace == Some(trace.id), window, cx)
            }
        }
    }
}

impl AgentConsoleView {
    pub(super) fn render_timeline_footer(
        &self,
        session: &AgentSession,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let status = session.status;
        let received_output = self
            .session_ui
            .get(&session.id)
            .is_some_and(|ui| ui.received_output);
        let mut footer = div().flex().flex_col().min_w_0().gap(px(8.0));
        if status.is_active() && !received_output {
            footer = footer.child(
                div()
                    .debug_selector(|| "agent-preparing-output".into())
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap(px(7.0))
                    .py(px(4.0))
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(animated_monolith(
                        "agent-console-thinking",
                        28.0,
                        MonolithMotion::Thinking,
                        cx,
                    ))
                    .child(t!("agents.status.preparing").to_string()),
            );
        }
        footer = footer
            .children(self.render_auth_recovery(session, cx))
            .child(self.render_run_progress(session, cx))
            .child(self.render_turn_change_cards(session, cx));
        footer.into_any_element()
    }
}
