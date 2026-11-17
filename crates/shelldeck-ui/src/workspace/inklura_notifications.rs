//! Inklura Manage notifications: titlebar bell, panel, background poll and
//! OS notifications for arrivals.
//!
//! Unlike the surface polls in `polling.rs`, this one runs for the whole signed-in
//! session, including while the window is hidden in the tray: catching activity
//! the user is not looking at is the point. One GET per minute; the server merges
//! every space of the account. A Manage without the endpoint answers 404 and the
//! bell simply stays hidden.

use super::*;
use shelldeck_core::config::manage_notifications::{
    self, fresh_unread, seen_keys, InkluraNotification, NotificationFeed, NotificationKind,
};

const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);
const PANEL_WIDTH: f32 = 380.0;
const PANEL_MAX_HEIGHT: f32 = 480.0;

/// One OS notification per poll: the item itself when alone, a count for a burst.
pub(super) fn arrival_notification(fresh: &[&InkluraNotification]) -> Option<TrayNotification> {
    let first = fresh.first()?;
    Some(TrayNotification::Inklura {
        count: fresh.len(),
        title: first.title.clone(),
        body: first.body.clone(),
        site: first.site_label.clone(),
    })
}

fn kind_icon(kind: NotificationKind) -> (&'static str, Hsla) {
    match kind {
        NotificationKind::Success => ("circle-check", ShellDeckColors::success()),
        NotificationKind::Error => ("circle-alert", ShellDeckColors::error()),
        NotificationKind::Warning => ("triangle-alert", ShellDeckColors::warning()),
        NotificationKind::Info => ("info", ShellDeckColors::primary()),
    }
}

fn badge_text(unread: usize) -> String {
    if unread > 99 {
        "99+".to_string()
    } else {
        unread.to_string()
    }
}

/// Runs of the same notification next to each other (same space, kind,
/// title and text: an agent that waits for a validation every minute) are
/// shown as one row with a count. Each run is the index of its newest item
/// (the feed is newest first) and the indices of the whole run.
fn notification_runs(items: &[InkluraNotification]) -> Vec<Vec<usize>> {
    let mut runs: Vec<Vec<usize>> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let same = runs
            .last()
            .and_then(|run| run.first())
            .is_some_and(|&first| {
                let head = &items[first];
                head.site_id == item.site_id
                    && head.kind == item.kind
                    && head.title == item.title
                    && head.body == item.body
            });
        match runs.last_mut() {
            Some(run) if same => run.push(index),
            _ => runs.push(vec![index]),
        }
    }
    runs
}

impl Workspace {
    /// Start the session poll after login/startup; stop it when signed out.
    pub(super) fn sync_inklura_notifications_poll(&mut self, cx: &mut Context<Self>) {
        if !self.signed_in() {
            self.clear_inklura_notifications();
            return;
        }
        if self._inklura_notifications_poll.is_some() {
            return;
        }
        self.refresh_inklura_notifications(cx);
        let task = cx.spawn(async move |this, cx: &mut AsyncApp| loop {
            cx.background_executor().timer(POLL_INTERVAL).await;
            let keep = this
                .update(cx, |ws, cx| {
                    if ws.signed_in() {
                        ws.refresh_inklura_notifications(cx);
                        true
                    } else {
                        ws.clear_inklura_notifications();
                        false
                    }
                })
                .unwrap_or(false);
            if !keep {
                break;
            }
        });
        self._inklura_notifications_poll = Some(task);
    }

    /// Drop everything tied to the previous account (logout, expired token).
    pub(super) fn clear_inklura_notifications(&mut self) {
        self._inklura_notifications_poll = None;
        self.inklura_notifications = None;
        self.inklura_notifications_seen = None;
        self.inklura_notifications_open = false;
    }

    pub(super) fn refresh_inklura_notifications(&mut self, cx: &mut Context<Self>) {
        let Some((base, token)) = self.manage_base_token() else {
            return;
        };
        let request = self.manage_requests.begin(ManageRead::Notifications);
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { manage_notifications::fetch_notifications(&base, &token) })
                .await;
            let _ = this.update(cx, |ws, cx| {
                if !ws.accept_manage_result(request, result.as_ref().err(), cx) {
                    return;
                }
                match result {
                    Ok(feed) => ws.apply_inklura_feed(feed, cx),
                    Err(e) => {
                        // 404 = this Manage predates the endpoint: keep the bell hidden.
                        if cloud_account::classify_api_error(&e)
                            == cloud_account::ApiFailure::NotFound
                        {
                            ws.inklura_notifications = None;
                            cx.notify();
                        } else {
                            tracing::debug!("Inklura notifications unavailable: {e}");
                        }
                    }
                }
            });
        })
        .detach();
    }

    fn apply_inklura_feed(&mut self, feed: NotificationFeed, cx: &mut Context<Self>) {
        let fresh = fresh_unread(self.inklura_notifications_seen.as_ref(), &feed);
        if self.app_config.tray.notify_inklura {
            if let Some(notification) = arrival_notification(&fresh) {
                self.emit_tray_notification(notification);
            }
        }
        let mut seen = self.inklura_notifications_seen.take().unwrap_or_default();
        seen.extend(seen_keys(&feed));
        // Bound memory over a long session: forget keys that left the feed.
        if seen.len() > 1000 {
            seen = seen_keys(&feed);
        }
        self.inklura_notifications_seen = Some(seen);
        if self.inklura_notifications.as_ref() != Some(&feed) {
            self.inklura_notifications = Some(feed);
            cx.notify();
        }
    }

    fn toggle_inklura_notifications(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.inklura_notifications_pos = Some(position);
        let open = !self.inklura_notifications_open;
        self.close_titlebar_menus();
        self.inklura_notifications_open = open;
        if open {
            // Opening the panel is the moment the user wants fresh data.
            self.refresh_inklura_notifications(cx);
        }
        cx.notify();
    }

    /// Mark the item read, then open its signed Manage link in the browser.
    fn open_inklura_notification(&mut self, site_id: String, id: String, cx: &mut Context<Self>) {
        let Some((base, token)) = self.manage_base_token() else {
            return;
        };
        let was_unread = self.inklura_notifications.as_ref().is_some_and(|feed| {
            feed.items
                .iter()
                .any(|n| n.site_id == site_id && n.id == id && !n.read)
        });
        if let Some(feed) = self.inklura_notifications.as_mut() {
            feed.mark_read_locally(&site_id, &id);
        }
        self.inklura_notifications_open = false;
        cx.notify();
        let request = self.manage_requests.session();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if was_unread {
                        if let Err(e) =
                            manage_notifications::mark_read(&base, &token, &site_id, &id)
                        {
                            tracing::warn!("Failed to mark Inklura notification read: {e}");
                        }
                    }
                    manage_notifications::open_url(&base, &token, &site_id, Some(&id))
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                if !ws.accept_manage_result(request, result.as_ref().err(), cx) {
                    return;
                }
                let opened = result.and_then(|url| cloud_account::open_in_browser(&url));
                match opened {
                    Ok(_) => ws.show_toast(
                        t!("toast.opening_browser").to_string(),
                        ToastLevel::Info,
                        cx,
                    ),
                    Err(e) => {
                        tracing::warn!("Failed to open Inklura notification: {e}");
                        ws.show_toast(
                            t!("inklura_notifications.open_failed").to_string(),
                            ToastLevel::Error,
                            cx,
                        );
                    }
                }
            });
        })
        .detach();
    }

    fn mark_all_inklura_notifications_read(&mut self, cx: &mut Context<Self>) {
        let Some((base, token)) = self.manage_base_token() else {
            return;
        };
        // Cut off at the newest item on screen: an alert that lands meanwhile stays unread.
        let before = self
            .inklura_notifications
            .as_ref()
            .and_then(|feed| feed.items.iter().map(|n| n.ts).max());
        if let Some(feed) = self.inklura_notifications.as_mut() {
            feed.mark_all_read_locally();
        }
        cx.notify();
        let request = self.manage_requests.session();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { manage_notifications::mark_all_read(&base, &token, before) })
                .await;
            let _ = this.update(cx, |ws, cx| {
                if !ws.accept_manage_result(request, result.as_ref().err(), cx) {
                    return;
                }
                if let Err(e) = result {
                    tracing::warn!("Failed to mark Inklura notifications read: {e}");
                    ws.show_toast(
                        t!("inklura_notifications.mark_failed").to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                    ws.refresh_inklura_notifications(cx);
                }
            });
        })
        .detach();
    }

    /// Titlebar bell with an unread badge. Hidden until the first feed arrives.
    pub(super) fn render_inklura_bell(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.signed_in() {
            return None;
        }
        let feed = self.inklura_notifications.as_ref()?;
        let unread = feed.unread;
        let tooltip: SharedString = t!("inklura_notifications.tooltip").to_string().into();
        let icon_color = if unread > 0 {
            ShellDeckColors::text_primary()
        } else {
            ShellDeckColors::text_muted()
        };
        let mut bell = div()
            .id("titlebar-inklura-notifications")
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .size(px(super::presentation::TITLEBAR_CONTROL))
            .rounded(px(super::presentation::TITLEBAR_RADIUS))
            .cursor_pointer()
            .hover(|el| el.bg(ShellDeckColors::hover_bg()))
            .tooltip(move |_, cx| {
                cx.new(|_| WorkspaceTooltip {
                    label: tooltip.clone(),
                })
                .into()
            })
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                this.toggle_inklura_notifications(event.position(), cx);
            }))
            .child(lucide_icon("bell", 16.0, icon_color));
        if self.inklura_notifications_open {
            bell = bell.bg(ShellDeckColors::hover_bg());
        }
        if unread > 0 {
            bell = bell.child(
                div()
                    .absolute()
                    .top(px(1.0))
                    .right(px(0.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .min_w(px(14.0))
                    .h(px(14.0))
                    .px(px(3.0))
                    .rounded_full()
                    .bg(ShellDeckColors::error())
                    .border_1()
                    .border_color(ShellDeckColors::bg_sidebar())
                    .text_size(px(8.5))
                    .font_weight(FontWeight::BOLD)
                    .text_color(white())
                    .child(badge_text(unread)),
            );
        }
        Some(bell.into_any_element())
    }

    /// One row of the panel: the newest item of a run, with the size of the
    /// run when it repeats. A row is unread while any item of its run is.
    fn render_inklura_notification_row(
        &self,
        items: &[InkluraNotification],
        run: &[usize],
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let item = &items[run[0]];
        let unread = run.iter().any(|&index| !items[index].read);
        let (icon, color) = kind_icon(item.kind);
        let mut text = div()
            .flex_1()
            .min_w(px(0.0))
            .overflow_hidden()
            .flex()
            .flex_col()
            .gap(px(3.0))
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(if unread {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::MEDIUM
                    })
                    .text_color(ShellDeckColors::text_primary())
                    .line_clamp(2)
                    .child(item.title.clone()),
            );
        if !item.body.trim().is_empty() {
            text = text.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .line_clamp(2)
                    .child(item.body.clone()),
            );
        }
        let when = rel_time(item.ts as f64);
        let meta = match (item.site_label.trim().is_empty(), when.is_empty()) {
            (false, false) => format!("{} · {}", item.site_label, when),
            (false, true) => item.site_label.clone(),
            (true, _) => when,
        };
        let mut meta_row = div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(6.0))
            .min_w(px(0.0))
            .overflow_hidden();
        if !meta.is_empty() {
            meta_row = meta_row.child(
                div()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(meta),
            );
        }
        if run.len() > 1 {
            meta_row = meta_row.child(
                div()
                    .flex_shrink_0()
                    .px(px(6.0))
                    .rounded_full()
                    .bg(ShellDeckColors::badge_bg())
                    .text_size(px(10.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!("×{}", run.len())),
            );
        }
        text = text.child(meta_row);

        let site_id = item.site_id.clone();
        let id = item.id.clone();
        let others: Vec<String> = run[1..]
            .iter()
            .filter(|&&index| !items[index].read)
            .map(|&index| items[index].id.clone())
            .collect();
        let mut row = div()
            .id(SharedString::from(format!(
                "inklura-notification-{}-{}",
                item.site_id, item.id
            )))
            // The list caps its height and scrolls: a row that may shrink is
            // squashed into its neighbours instead.
            .flex_shrink_0()
            .flex()
            .items_start()
            .gap(px(12.0))
            .px(px(12.0))
            .py(px(10.0))
            .rounded(px(10.0))
            .cursor_pointer()
            .hover(|el| el.bg(ShellDeckColors::hover_bg()))
            .child(
                div()
                    .flex_shrink_0()
                    .size(px(30.0))
                    .rounded(px(8.0))
                    .bg(color.opacity(0.12))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(lucide_icon(icon, 15.0, color)),
            )
            .child(text)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.mark_inklura_notifications_read_quietly(&site_id, &others, cx);
                this.open_inklura_notification(site_id.clone(), id.clone(), cx);
            }));
        if unread {
            row = row.bg(ShellDeckColors::primary().opacity(0.05)).child(
                div()
                    .flex_shrink_0()
                    .mt(px(6.0))
                    .size(px(8.0))
                    .rounded_full()
                    .bg(ShellDeckColors::primary()),
            );
        }
        row
    }

    /// The hidden items of a run that was opened: read locally at once, and
    /// on the server in the background without a toast (opening the newest
    /// one reports its own failure).
    fn mark_inklura_notifications_read_quietly(
        &mut self,
        site_id: &str,
        ids: &[String],
        cx: &mut Context<Self>,
    ) {
        if ids.is_empty() {
            return;
        }
        let Some((base, token)) = self.manage_base_token() else {
            return;
        };
        if let Some(feed) = self.inklura_notifications.as_mut() {
            for id in ids {
                feed.mark_read_locally(site_id, id);
            }
        }
        let site_id = site_id.to_string();
        let ids = ids.to_vec();
        cx.background_executor()
            .spawn(async move {
                for id in ids {
                    if let Err(e) = manage_notifications::mark_read(&base, &token, &site_id, &id) {
                        tracing::warn!("Failed to mark Inklura notification read: {e}");
                    }
                }
            })
            .detach();
    }

    /// Anchored panel under the bell plus a dismiss backdrop.
    pub(super) fn render_inklura_notifications_menu(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let feed = self.inklura_notifications.clone().unwrap_or_default();
        let shadow = vec![BoxShadow {
            color: hsla(0.0, 0.0, 0.0, 0.35),
            offset: point(gpui::px(0.0), gpui::px(4.0)),
            blur_radius: gpui::px(16.0),
            spread_radius: gpui::px(0.0),
            inset: false,
        }];

        let mut title = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .flex_1()
            .min_w(px(0.0))
            .child(
                div()
                    .text_size(px(15.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(t!("inklura_notifications.title").to_string()),
            );
        if feed.unread > 0 {
            title = title.child(
                div()
                    .px(px(7.0))
                    .py(px(1.0))
                    .rounded_full()
                    .bg(ShellDeckColors::primary().opacity(0.12))
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::primary())
                    .child(badge_text(feed.unread)),
            );
        }
        let mut header = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(16.0))
            .pt(px(12.0))
            .pb(px(10.0))
            .border_b_1()
            .border_color(ShellDeckColors::border().opacity(0.7))
            .child(title);
        if feed.unread > 0 {
            header = header.child(
                Button::new(
                    "inklura-notifications-mark-all",
                    t!("inklura_notifications.mark_all").to_string(),
                )
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Sm)
                .icon(IconSource::from("check-check"))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.mark_all_inklura_notifications_read(cx);
                })),
            );
        }

        let mut list = div()
            .id("inklura-notifications-list")
            .flex()
            .flex_col()
            .gap(px(2.0))
            .p(px(6.0))
            .max_h(px(PANEL_MAX_HEIGHT))
            .overflow_y_scroll();
        if feed.items.is_empty() {
            list = list.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(16.0))
                    .py(px(28.0))
                    .child(lucide_icon("bell", 22.0, ShellDeckColors::text_muted()))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("inklura_notifications.empty").to_string()),
                    )
                    .child(
                        div()
                            .max_w(px(PANEL_WIDTH - 48.0))
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .text_center()
                            .child(t!("inklura_notifications.empty_hint").to_string()),
                    ),
            );
        } else {
            for run in notification_runs(&feed.items) {
                list = list.child(self.render_inklura_notification_row(&feed.items, &run, cx));
            }
        }

        let panel = div()
            .id("inklura-notifications-panel")
            .w(px(PANEL_WIDTH))
            .bg(ShellDeckColors::popover_bg())
            .border_1()
            .border_color(ShellDeckColors::border())
            .rounded(px(12.0))
            .shadow(shadow.into())
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_event, _window, cx: &mut App| {
                cx.stop_propagation();
            })
            .child(header)
            .child(list);

        let drop = px(WORKSPACE_TITLEBAR_HEIGHT + 4.0).to_pixels(window.rem_size());
        let anchor_x = self
            .inklura_notifications_pos
            .map(|position| position.x)
            .unwrap_or_else(|| window.bounds().size.width - gpui::px(200.0));
        let anchor = point(
            anchor_x - px(PANEL_WIDTH - 40.0).to_pixels(window.rem_size()),
            drop,
        );

        div()
            .id("inklura-notifications-backdrop")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _event, _window, cx| {
                    this.inklura_notifications_open = false;
                    cx.notify();
                }),
            )
            .child(
                deferred(
                    anchored()
                        .position(anchor)
                        .anchor(gpui::Corner::TopLeft)
                        .snap_to_window_with_margin(gpui::px(8.0))
                        .child(panel),
                )
                .with_priority(2),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        arrival_notification, notification_runs, InkluraNotification, NotificationKind,
        TrayNotification,
    };

    fn item(id: &str, title: &str, site: &str) -> InkluraNotification {
        InkluraNotification {
            id: id.into(),
            site_id: "s1".into(),
            tenant_id: String::new(),
            site_label: site.into(),
            kind: NotificationKind::Info,
            title: title.into(),
            body: "Corps".into(),
            ts: 1,
            read: false,
            has_link: true,
        }
    }

    // SDTEST-2903
    #[test]
    fn a_single_arrival_names_itself_and_a_burst_collapses_to_one_count() {
        let a = item("a", "Export prêt", "Acme");
        let b = item("b", "Autre", "Blog");
        assert_eq!(arrival_notification(&[]), None);
        let one = arrival_notification(&[&a]).unwrap();
        let (summary, body) = one.localized_text();
        assert!(summary.ends_with("· Acme"), "{summary}");
        assert_eq!(body, "Export prêt\nCorps");
        let many = arrival_notification(&[&a, &b]).unwrap();
        assert!(matches!(many, TrayNotification::Inklura { count: 2, .. }));
        assert!(!many.localized_text().1.contains("Export"));
    }

    // SDTEST-3186 / SDUC-593. Repeats next to each other collapse into one
    // row; the same title elsewhere in the feed, from another space or with
    // another text stays its own row.
    #[test]
    fn sdtest_3186_repeated_notifications_collapse_into_runs() {
        let wait = item("1", "Jcode attend une validation", "Acme");
        let mut other_site = item("4", "Jcode attend une validation", "Blog");
        other_site.site_id = "s2".into();
        let items = vec![
            item("0", "Export prêt", "Acme"),
            wait.clone(),
            item("2", "Jcode attend une validation", "Acme"),
            item("3", "Jcode attend une validation", "Acme"),
            other_site,
            item("5", "Échec d'une demande", "Acme"),
            item("6", "Jcode attend une validation", "Acme"),
        ];
        assert_eq!(
            notification_runs(&items),
            vec![vec![0], vec![1, 2, 3], vec![4], vec![5], vec![6]]
        );
        assert!(notification_runs(&[]).is_empty());
    }
}
