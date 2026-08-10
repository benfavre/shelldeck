use super::*;

impl SupportView {
    pub(crate) fn my_email(&self) -> &str {
        &self.me.email
    }

    /// Resolve the compact assignee label shared by Ticket and Request
    /// headers while each adapter keeps its original wire value.
    pub(super) fn assignee_label(&self, assignee: &str) -> String {
        let assignee = assignee.trim();
        if assignee.is_empty() {
            return t!("support.assignee.none").to_string();
        }
        if assignee.eq_ignore_ascii_case("me")
            || (!self.me.email.trim().is_empty()
                && assignee.eq_ignore_ascii_case(self.me.email.trim()))
        {
            return if self.me.name.trim().is_empty() {
                t!("support.assignee.me").to_string()
            } else {
                self.me.name.clone()
            };
        }
        self.agents
            .iter()
            .find(|agent| agent.email.eq_ignore_ascii_case(assignee))
            .and_then(|agent| (!agent.name.trim().is_empty()).then(|| agent.name.clone()))
            .unwrap_or_else(|| assignee.to_string())
    }

    pub(super) fn passes_filter(&self, t: &SupportTicket) -> bool {
        match self.filter {
            SupportFilter::All => true,
            SupportFilter::Unassigned => t.is_unassigned(),
            SupportFilter::Mine => !self.my_email().is_empty() && t.assignee == self.me.email,
            SupportFilter::Open => t.status == "open",
            SupportFilter::Pending => t.status == "pending",
            SupportFilter::Breaching => t.sla.breaching,
            SupportFilter::Closed => t.status == "closed",
        }
    }

    pub(super) fn search_query(&self, cx: &Context<Self>) -> String {
        self.search_state.read(cx).content().to_string()
    }

    pub(super) fn has_advanced_filters(&self) -> bool {
        self.adv_unread_only
            || self.adv_sla_only
            || self.adv_channel.is_some()
            || self.adv_priority.is_some()
            || self.adv_assignee.is_some()
    }

    pub(super) fn has_list_constraints(&self, cx: &Context<Self>) -> bool {
        !self.search_query(cx).trim().is_empty() || self.has_advanced_filters()
    }

    pub(super) fn sync_filter_draft_from_applied(&mut self) {
        self.adv_draft_channel = self.adv_channel.clone();
        self.adv_draft_priority = self.adv_priority.clone();
        self.adv_draft_unread_only = self.adv_unread_only;
        self.adv_draft_assignee = self.adv_assignee.clone();
        self.adv_draft_sla_only = self.adv_sla_only;
    }

    pub(super) fn open_filter_modal(&mut self, cx: &mut Context<Self>) {
        self.sync_filter_draft_from_applied();
        self.refresh_assignee_draft_select(cx);
        self.filter_modal_open = true;
        cx.notify();
    }

    pub(super) fn assignee_select_value(draft: &Option<String>) -> String {
        match draft {
            None => ASSIGNEE_SELECT_ALL.to_string(),
            Some(email) if email.is_empty() => ASSIGNEE_SELECT_NONE.to_string(),
            Some(email) => email.clone(),
        }
    }

    pub(super) fn assignee_from_select_value(value: &str) -> Option<String> {
        match value {
            ASSIGNEE_SELECT_ALL => None,
            ASSIGNEE_SELECT_NONE => Some(String::new()),
            other => Some(other.to_string()),
        }
    }

    pub(super) fn build_assignee_draft_select(
        draft: Option<String>,
        agents: &[SupportAgent],
        parent: Entity<SupportView>,
        cx: &mut Context<SupportView>,
    ) -> Entity<Select<String>> {
        let mut options = vec![
            SelectOption::new(
                ASSIGNEE_SELECT_ALL.to_string(),
                t!("support.assignee.all").to_string(),
            )
            .with_icon("icons/lucide/users.svg"),
            SelectOption::new(
                ASSIGNEE_SELECT_NONE.to_string(),
                t!("support.assignee.unassigned").to_string(),
            )
            .with_icon("icons/lucide/user.svg"),
        ];
        for agent in agents {
            let label = if agent.name.trim().is_empty() {
                agent.email.clone()
            } else {
                agent.name.clone()
            };
            options.push(
                SelectOption::new(agent.email.clone(), label)
                    .with_icon("icons/lucide/user-check.svg"),
            );
        }
        let selected_value = Self::assignee_select_value(&draft);
        let selected_index = options.iter().position(|o| o.value == selected_value);
        cx.new(|select_cx| {
            Select::new(select_cx)
                .options(options)
                .selected_index(selected_index)
                .placeholder(t!("support.assignee.placeholder").to_string())
                .on_change({
                    move |value, _window, cx| {
                        parent.update(cx, |this, cx| {
                            this.adv_draft_assignee = Self::assignee_from_select_value(value);
                            cx.notify();
                        });
                    }
                })
        })
    }

    pub(super) fn refresh_assignee_draft_select(&mut self, cx: &mut Context<Self>) {
        let parent = cx.entity();
        self.assignee_draft_select = Self::build_assignee_draft_select(
            self.adv_draft_assignee.clone(),
            &self.agents,
            parent,
            cx,
        );
    }

    /// Human label for the current draft assignee — used as the button
    /// text inside the filter modal's assignee row + the empty-state
    /// label of the picker. Recognises the special sentinels ("", "me",
    /// "unassigned") and otherwise resolves the raw email to the agent's
    /// display name.
    pub(super) fn issues_assignee_label(&self, assignee: &str) -> String {
        match assignee {
            "" => t!("support.issues.assignee.all").to_string(),
            "me" => t!("support.issues.assignee.me").to_string(),
            "unassigned" => t!("support.issues.assignee.unassigned").to_string(),
            email => self
                .agents
                .iter()
                .find(|a| a.email == email)
                .map(|a| {
                    if a.name.trim().is_empty() {
                        a.email.clone()
                    } else {
                        a.name.clone()
                    }
                })
                .unwrap_or_else(|| email.to_string()),
        }
    }

    pub(super) fn open_issues_assignee_modal(&mut self, cx: &mut Context<Self>) {
        // Reset the search input each open so the picker doesn't remember
        // a stale query from a prior session.
        self.issues_assignee_search_state = cx.new(InputState::new);
        self.issues_assignee_modal_open = true;
        cx.notify();
    }

    pub(super) fn close_issues_assignee_modal(&mut self, cx: &mut Context<Self>) {
        self.issues_assignee_modal_open = false;
        cx.notify();
    }

    pub(super) fn pick_issues_assignee(&mut self, value: String, cx: &mut Context<Self>) {
        self.issues_filter_draft.assignee = value;
        self.issues_assignee_modal_open = false;
        cx.notify();
    }

    pub(super) fn apply_filter_draft(&mut self, cx: &mut Context<Self>) {
        self.adv_channel = self.adv_draft_channel.clone();
        self.adv_priority = self.adv_draft_priority.clone();
        self.adv_unread_only = self.adv_draft_unread_only;
        self.adv_assignee = self.adv_draft_assignee.clone();
        self.adv_sla_only = self.adv_draft_sla_only;
        self.filter_modal_open = false;
        cx.notify();
    }

    pub(super) fn close_filter_modal(&mut self, cx: &mut Context<Self>) {
        self.filter_modal_open = false;
        cx.notify();
    }

    pub(super) fn reset_filter_draft(&mut self, cx: &mut Context<Self>) {
        self.adv_draft_channel = None;
        self.adv_draft_priority = None;
        self.adv_draft_unread_only = false;
        self.adv_draft_assignee = None;
        self.adv_draft_sla_only = false;
        self.refresh_assignee_draft_select(cx);
        cx.notify();
    }

    #[expect(dead_code)]
    pub(super) fn clear_advanced_filters(&mut self, cx: &mut Context<Self>) {
        self.adv_channel = None;
        self.adv_priority = None;
        self.adv_unread_only = false;
        self.adv_assignee = None;
        self.adv_sla_only = false;
        if self.filter_modal_open {
            self.reset_filter_draft(cx);
        }
        cx.notify();
    }

    pub(super) fn adv_channel_icon(value: &str) -> &'static str {
        ADV_CHANNELS
            .iter()
            .find(|o| o.value == Some(value))
            .map(|o| o.icon)
            .unwrap_or("inbox")
    }

    pub(super) fn adv_channel_label(value: &str) -> String {
        adv_channel_label(Some(value))
    }

    pub(super) fn adv_priority_label(value: &str) -> String {
        adv_priority_label(Some(value))
    }

    pub(super) fn assignee_filter_label(&self, email: &str) -> String {
        if email.is_empty() {
            return t!("support.assignee.unassigned").to_string();
        }
        self.agents
            .iter()
            .find(|a| a.email == email)
            .map(|a| {
                if a.name.trim().is_empty() {
                    a.email.clone()
                } else {
                    a.name.clone()
                }
            })
            .unwrap_or_else(|| email.to_string())
    }

    pub(super) fn passes_advanced(&self, t: &SupportTicket, query: &str) -> bool {
        if self.adv_unread_only && !t.unread {
            return false;
        }
        if let Some(ref ch) = self.adv_channel {
            if t.channel != *ch {
                return false;
            }
        }
        if let Some(ref p) = self.adv_priority {
            if t.priority != *p {
                return false;
            }
        }
        if let Some(ref assignee) = self.adv_assignee {
            if assignee.is_empty() {
                if !t.is_unassigned() {
                    return false;
                }
            } else if t.assignee != *assignee {
                return false;
            }
        }
        if self.adv_sla_only && !t.sla.breaching && !t.sla.breached {
            return false;
        }
        let q = query.trim();
        if q.is_empty() {
            return true;
        }
        let q = q.to_lowercase();
        let hay = format!(
            "{} {} {} {} {}",
            t.subject,
            t.contact.display(),
            t.contact.email.as_deref().unwrap_or(""),
            t.last_preview,
            t.id,
        )
        .to_lowercase();
        hay.contains(&q)
    }
}
