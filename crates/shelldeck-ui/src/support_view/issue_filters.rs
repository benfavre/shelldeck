use super::*;

impl SupportView {
    /// Count of *advanced* filters currently active (everything except the
    /// simple bar's `status` + `q`). Drives the badge on the "Filtres"
    /// button so the user knows a hidden filter is narrowing their list.
    pub(super) fn advanced_filter_count(&self) -> usize {
        let f = &self.issues_filter;
        let mut n = 0;
        if !f.priority.is_empty() {
            n += 1;
        }
        if !f.source.is_empty() {
            n += 1;
        }
        if !f.assignee.is_empty() {
            n += 1;
        }
        if f.mine {
            n += 1;
        }
        if !f.tenant_id.is_empty() {
            n += 1;
        }
        if f.has_github.is_some() {
            n += 1;
        }
        if !f.since.is_empty() {
            n += 1;
        }
        n
    }

    pub(super) fn open_issues_filter_modal(&mut self, cx: &mut Context<Self>) {
        self.issues_filter_draft = self.issues_filter.clone();
        self.issues_filter_modal_open = true;
        cx.notify();
    }

    pub(super) fn close_issues_filter_modal(&mut self, cx: &mut Context<Self>) {
        self.issues_filter_modal_open = false;
        cx.notify();
    }

    pub(super) fn reset_issues_filter_draft(&mut self, cx: &mut Context<Self>) {
        // Preserve status + q (simple bar) — Reset here only clears the
        // *advanced* fields, matching the badge scope.
        let status = self.issues_filter_draft.status.clone();
        let q = self.issues_filter_draft.q.clone();
        self.issues_filter_draft = shelldeck_core::config::issues::IssueListFilter {
            status,
            q,
            ..Default::default()
        };
        cx.notify();
    }

    pub(super) fn apply_issues_filter_draft(&mut self, cx: &mut Context<Self>) {
        self.issues_filter = self.issues_filter_draft.clone();
        self.issues_filter_modal_open = false;
        let filter = self.issues_filter.clone();
        cx.emit(SupportViewEvent::IssuesFilterChanged { filter });
        cx.notify();
    }

    /// One-line summary label for the "since" ISO bound — approximated from
    /// the delta between the stored ISO and now. Chips only ever set 24h/7d/30d
    /// so the buckets match the picker.
    pub(super) fn since_bucket_label(iso: &str) -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        // Cheap ISO → epoch: (yyyy, mm, dd, hh, mm, ss) → days via Hinnant.
        let bytes = iso.as_bytes();
        if bytes.len() < 19 {
            return t!("support.issues.since.h24").to_string();
        }
        let n = |s: usize, e: usize| -> i64 { iso[s..e].parse::<i64>().unwrap_or(0) };
        let (y, mo, d, h, mi, s) = (n(0, 4), n(5, 7), n(8, 10), n(11, 13), n(14, 16), n(17, 19));
        let (yy, mm) = if mo <= 2 { (y - 1, mo + 12) } else { (y, mo) };
        let era = yy.div_euclid(400);
        let yoe = yy - era * 400;
        let doy = (153 * (mm - 3) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        let then = days * 86_400 + h * 3600 + mi * 60 + s;
        let delta_h = (now - then) / 3600;
        if delta_h <= 25 {
            t!("support.issues.since.h24").to_string()
        } else if delta_h <= 24 * 8 {
            t!("support.issues.since.d7").to_string()
        } else {
            t!("support.issues.since.d30").to_string()
        }
    }

    /// Applied filter chips row — one chip per active advanced field, each
    /// removable via a trailing X. Reuses the tickets' `render_applied_filter_chip`
    /// helper (icon + Outline Badge + Ghost X IconButton) so the two surfaces
    /// share the same visual language. Rendered only when `advanced_filter_count > 0`.
    pub(super) fn render_applied_issues_filter_chips(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut row = div()
            .flex()
            .flex_wrap()
            .gap(px(4.0))
            .px(px(10.0))
            .pb(px(6.0));

        let f = self.issues_filter.clone();

        if !f.priority.is_empty() {
            row = row.child(self.render_applied_filter_chip_with_badge(
                "iss-applied-pri".to_string(),
                "flag",
                priority_badge(&f.priority),
                cx,
                |this, cx| {
                    this.issues_filter.priority.clear();
                    let filter = this.issues_filter.clone();
                    cx.emit(SupportViewEvent::IssuesFilterChanged { filter });
                    cx.notify();
                },
            ));
        }
        if !f.source.is_empty() {
            let key = format!("support.issues.source.{}", f.source);
            let label = t!(&key).to_string();
            row = row.child(self.render_applied_filter_chip(
                "iss-applied-src".to_string(),
                "tag",
                label,
                cx,
                |this, cx| {
                    this.issues_filter.source.clear();
                    let filter = this.issues_filter.clone();
                    cx.emit(SupportViewEvent::IssuesFilterChanged { filter });
                    cx.notify();
                },
            ));
        }
        if !f.assignee.is_empty() {
            let label = match f.assignee.as_str() {
                "me" => t!("support.issues.assignee.me").to_string(),
                "unassigned" => t!("support.issues.assignee.unassigned").to_string(),
                other => other.to_string(),
            };
            row = row.child(self.render_applied_filter_chip(
                "iss-applied-as".to_string(),
                "user-check",
                label,
                cx,
                |this, cx| {
                    this.issues_filter.assignee.clear();
                    let filter = this.issues_filter.clone();
                    cx.emit(SupportViewEvent::IssuesFilterChanged { filter });
                    cx.notify();
                },
            ));
        }
        if let Some(gh) = f.has_github {
            let label = if gh {
                t!("support.issues.github.linked").to_string()
            } else {
                t!("support.issues.github.unlinked").to_string()
            };
            row = row.child(self.render_applied_filter_chip(
                "iss-applied-gh".to_string(),
                "upload",
                label,
                cx,
                |this, cx| {
                    this.issues_filter.has_github = None;
                    let filter = this.issues_filter.clone();
                    cx.emit(SupportViewEvent::IssuesFilterChanged { filter });
                    cx.notify();
                },
            ));
        }
        if !f.since.is_empty() {
            let label = Self::since_bucket_label(&f.since);
            row = row.child(self.render_applied_filter_chip(
                "iss-applied-sc".to_string(),
                "clock",
                label,
                cx,
                |this, cx| {
                    this.issues_filter.since.clear();
                    let filter = this.issues_filter.clone();
                    cx.emit(SupportViewEvent::IssuesFilterChanged { filter });
                    cx.notify();
                },
            ));
        }
        if f.mine {
            row = row.child(self.render_applied_filter_chip(
                "iss-applied-mine".to_string(),
                "user",
                t!("support.issues.mine").to_string(),
                cx,
                |this, cx| {
                    this.issues_filter.mine = false;
                    let filter = this.issues_filter.clone();
                    cx.emit(SupportViewEvent::IssuesFilterChanged { filter });
                    cx.notify();
                },
            ));
        }
        if !f.tenant_id.is_empty() {
            row = row.child(self.render_applied_filter_chip(
                "iss-applied-tn".to_string(),
                "users",
                f.tenant_id.clone(),
                cx,
                |this, cx| {
                    this.issues_filter.tenant_id.clear();
                    let filter = this.issues_filter.clone();
                    cx.emit(SupportViewEvent::IssuesFilterChanged { filter });
                    cx.notify();
                },
            ));
        }
        row
    }

    /// The "Filtres" trigger — same shape as the tickets bar: an
    /// `IconButton` (`filter` glyph) whose variant flips to `Default` when
    /// ≥1 advanced field is active, with a `Badge` next to it showing the
    /// count. Kept identical to `render_filters` so the two surfaces don't
    /// drift (see `.agents/ui-components.md` § harmonization).
    pub(super) fn render_issues_filter_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.advanced_filter_count();
        let entity = cx.entity();
        let filter_btn = IconButton::new("filter")
            .variant(if count > 0 {
                ButtonVariant::Default
            } else {
                ButtonVariant::Outline
            })
            .size(gpui::px(28.0))
            .icon_size(gpui::px(12.0))
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| this.open_issues_filter_modal(cx));
            });
        div()
            .flex()
            .items_center()
            .gap(px(4.0))
            .child(filter_btn)
            .when(count > 0, |el| {
                el.child(Badge::new(count.to_string()).variant(BadgeVariant::Default))
            })
    }

    /// Trimmed + case-insensitive check that the signed-in account is the
    /// filer of `iss`. Mirrors the server's owner gate
    /// (`requested_by === actor || user_name || user_email`).
    pub(super) fn is_my_issue(&self, iss: &Issue) -> bool {
        let rb = iss.requested_by.trim().to_ascii_lowercase();
        if rb.is_empty() {
            return false;
        }
        (!self.account_name_lc.is_empty() && rb == self.account_name_lc)
            || (!self.account_email_lc.is_empty() && rb == self.account_email_lc)
    }

    /// Count exactly the rows the Support request list can render. Keeping the
    /// tab badge on this same predicate prevents a raw API count from claiming
    /// there are requests while the defensive owner filter hides every row.
    pub(super) fn visible_issue_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|issue| self.issues_staff || self.is_my_issue(issue))
            .count()
    }

    /// Total authorized request universe advertised by the server. The API
    /// may cap the loaded page, so navigation and dashboard labels must not
    /// collapse this total to the number of rows currently in memory.
    pub(super) fn issue_total_count(&self) -> usize {
        super::reconciled_issue_total(self.issue_counts.all, self.visible_issue_count())
    }
}
