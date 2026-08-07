use super::*;

impl Workspace {
    pub(super) fn sync_support_poll(&mut self, cx: &mut Context<Self>) {
        let want = !self.settings_open
            && self.effective_mode() == AppMode::Support
            && self.app_config.cloud_sync.is_configured();
        if want {
            if self._support_poll_task.is_none() {
                let task = cx.spawn(async move |this, cx: &mut AsyncApp| loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(30))
                        .await;
                    let keep_going = this
                        .update(cx, |ws, cx| {
                            if !ws.settings_open && ws.effective_mode() == AppMode::Support {
                                ws.refresh_support(cx);
                                true
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);
                    if !keep_going {
                        break;
                    }
                });
                self._support_poll_task = Some(task);
            }
        } else {
            self._support_poll_task = None;
        }
    }

    pub(super) fn refresh_support(&mut self, cx: &mut Context<Self>) {
        if !self.app_config.cloud_sync.is_configured() {
            return;
        }
        let base = self.account_base_url();
        let token = self.app_config.cloud_sync.token.clone();
        let need_agents = !self.support.read(cx).has_agents();
        self.support.update(cx, |v, cx| {
            v.set_loading(true);
            cx.notify();
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let (list, agents) = cx
                .background_executor()
                .spawn(async move {
                    let list = manage_support::support_list(&base, &token);
                    let agents = if need_agents {
                        manage_support::support_agents(&base, &token).ok()
                    } else {
                        None
                    };
                    (list, agents)
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                ws.support.update(cx, |v, cx| {
                    match list {
                        Ok(r) => v.set_list(r.tickets, r.counts, r.me),
                        Err(e) => v.set_error(cloud_account::user_message(&e)),
                    }
                    if let Some(a) = agents {
                        v.set_agents(a);
                    }
                    cx.notify();
                });
                // Support poll changed unread_ticket_count → refresh
                // tray counters. Fires every 30 s while Support is
                // active; the tray dedups.
                ws.publish_tray_state(cx);
            });
        })
        .detach();
    }

    pub(super) fn select_support_ticket(&mut self, id: String, cx: &mut Context<Self>) {
        let base = self.account_base_url();
        let token = self.app_config.cloud_sync.token.clone();
        self.add_activity_entry(
            ActivityEntry::new(
                ActivityKind::Support,
                t!("activity.support.open_ticket", id = id.as_str()).to_string(),
            )
            .with_target(id.clone(), id.clone())
            .with_action(ActivityAction::OpenTicket),
            cx,
        );
        self.support.update(cx, |v, cx| {
            v.set_loading(true);
            cx.notify();
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let detail = cx
                .background_executor()
                .spawn(async move {
                    let detail = manage_support::support_ticket(&base, &token, &id);
                    // Best-effort mark-read; ignore result.
                    let _ = manage_support::support_read(&base, &token, &id);
                    detail
                })
                .await;
            let _ = this.update(cx, |ws, cx| match detail {
                Ok(t) => {
                    ws.support.update(cx, |v, cx| {
                        v.set_detail(t, cx);
                        cx.notify();
                    });
                    // Unread counts drift ≤30 s until the poll runs — an
                    // eager `refresh_support` here doubled the HTTP round
                    // trips on every selection.
                }
                Err(e) => {
                    let msg = cloud_account::user_message(&e);
                    ws.support.update(cx, |v, cx| {
                        v.set_error(msg);
                        cx.notify();
                    });
                }
            });
        })
        .detach();
    }

    pub(super) fn handle_support_event(&mut self, event: SupportViewEvent, cx: &mut Context<Self>) {
        use manage_support as ms;
        match event {
            SupportViewEvent::Refresh => self.refresh_support(cx),
            SupportViewEvent::SelectTicket(id) => self.select_support_ticket(id, cx),
            SupportViewEvent::SuggestReply(ticket_id) => {
                self.open_ai_workflow(AiWorkflowTarget::SupportReply { ticket_id }, cx)
            }
            SupportViewEvent::SummarizeTicket(ticket_id) => {
                self.open_ai_workflow(AiWorkflowTarget::SupportSummary { ticket_id }, cx)
            }
            SupportViewEvent::TriageTicket(ticket_id) => {
                self.open_ai_workflow(AiWorkflowTarget::SupportTriage { ticket_id }, cx)
            }
            SupportViewEvent::SuggestIssueReply(issue_id) => {
                // Let the composer show a pending state on the ✦ button while
                // the workflow runs — until the draft comes back.
                self.support.update(cx, |view, cx| view.set_issue_ai_pending(true, cx));
                self.open_ai_workflow(AiWorkflowTarget::IssueReply { issue_id }, cx)
            }
            SupportViewEvent::PublishIssueAiDraft => {
                self.support.update(cx, |view, cx| view.publish_issue_ai_draft(cx));
            }
            SupportViewEvent::DiscardIssueAiDraft => {
                self.support.update(cx, |view, cx| view.discard_issue_ai_draft(cx));
            }
            SupportViewEvent::SummarizeIssue(issue_id) => {
                self.open_ai_workflow(AiWorkflowTarget::IssueSummary { issue_id }, cx)
            }
            SupportViewEvent::TriageIssue(issue_id) => {
                self.open_ai_workflow(AiWorkflowTarget::IssueTriage { issue_id }, cx)
            }
            SupportViewEvent::Send {
                id,
                text,
                note,
                attachments,
            } => {
                self.support_action(cx, move |base, token| {
                    if attachments.is_empty() && note {
                        ms::support_note(&base, &token, &id, &text)
                    } else if attachments.is_empty() {
                        ms::support_reply(&base, &token, &id, &text)
                    } else {
                        let uploads = attachments
                            .iter()
                            .map(AttachmentDraft::upload)
                            .collect::<Vec<_>>();
                        let receipts =
                            ms::upload_support_attachments(&base, &token, &id, &uploads)?;
                        if note {
                            ms::support_note_with_attachments(&base, &token, &id, &text, &receipts)
                        } else {
                            ms::support_reply_with_attachments(&base, &token, &id, &text, &receipts)
                        }
                    }
                });
            }
            SupportViewEvent::SetStatus { id, status } => {
                self.support_action(cx, move |b, t| ms::support_status(&b, &t, &id, &status));
            }
            SupportViewEvent::SetPriority { id, priority } => {
                self.support_action(cx, move |b, t| ms::support_priority(&b, &t, &id, &priority));
            }
            SupportViewEvent::Assign { id, assignee } => {
                self.support_action(cx, move |b, t| ms::support_assign(&b, &t, &id, &assignee));
            }
            SupportViewEvent::Resolve { id, resolution } => {
                self.support_action(cx, move |b, t| {
                    ms::support_resolve(&b, &t, &id, &resolution)
                });
            }
            SupportViewEvent::JeanConfirm(thread) => {
                self.jean_action(cx, move |c| jeanclaude::confirm(&c, &thread));
            }
            SupportViewEvent::JeanReject(thread) => {
                self.jean_action(cx, move |c| jeanclaude::reject(&c, &thread));
            }
            SupportViewEvent::SendToJean(text) => self.prepare_jean_dispatch(text, cx),
            SupportViewEvent::ConvertToIssue { title, body } => {
                self.open_prefilled_request(title, body, "support", cx)
            }
            SupportViewEvent::IssuesRefresh => self.refresh_issues(cx),
            SupportViewEvent::SelectIssue(id) => self.select_issue(id, cx),
            SupportViewEvent::IssueComment {
                id,
                body,
                attachments,
            } => self.comment_issue_with_images(id, body, attachments, cx),
            SupportViewEvent::ImportAttachmentUrl { url, generation } => {
                cx.spawn(async move |this, cx: &mut AsyncApp| {
                    let result = cx
                        .background_executor()
                        .spawn(async move { issues::download_issue_image_url(&url) })
                        .await
                        .map_err(|e| cloud_account::user_message(&e))
                        .and_then(|upload| {
                            AttachmentDraft::from_bytes(upload.filename, upload.bytes)
                        });
                    let _ = this.update(cx, |ws, cx| {
                        ws.support.update(cx, |view, cx| {
                            view.finish_attachment_url_import(generation, result, cx)
                        });
                    });
                })
                .detach();
            }
            SupportViewEvent::IssueStatus { id, status } => {
                self.issue_staff_action(cx, move |b, t| issues::set_status(&b, &t, &id, &status))
            }
            SupportViewEvent::IssueAssign { id, assignee } => {
                self.issue_staff_action(cx, move |b, t| issues::assign(&b, &t, &id, &assignee))
            }
            SupportViewEvent::IssuePriority { id, priority } => self
                .issue_staff_action(cx, move |b, t| issues::set_priority(&b, &t, &id, &priority)),
            SupportViewEvent::IssueDispatch { id, instance_id } => {
                self.prepare_fleet_dispatch(id, instance_id, cx)
            }
            // `ai.*` belongs to Settings — same route as the assistant and the
            // request sheet, so the three cannot drift.
            SupportViewEvent::SelectAiBackend(backend) => {
                self.settings.update(cx, |settings, cx| {
                    settings.set_ai_backend(backend, cx);
                });
            }
            SupportViewEvent::IssueGithubPush(id) => {
                self.issue_staff_action(cx, move |b, t| issues::github_push(&b, &t, &id))
            }
            SupportViewEvent::IssueGithubRefresh(id) => {
                self.issue_staff_action(cx, move |b, t| issues::github_refresh(&b, &t, &id))
            }
            SupportViewEvent::IssueDelete(id) => self.delete_issue_now(id, cx),
            SupportViewEvent::IssueAttachmentDelete { id, attachment_id } => {
                self.delete_issue_attachment_now(id, attachment_id, cx)
            }
            SupportViewEvent::SupportAttachmentDelete { id, attachment_id } => {
                self.delete_support_attachment_now(id, attachment_id, cx)
            }
            SupportViewEvent::IssuesFilterChanged { filter } => {
                self.issues_filter = filter;
                self.refresh_issues(cx);
            }
        }
    }

    /// Run a support write action on the background executor; on success install
    /// the updated ticket + refresh the list, on failure toast the error.
    pub(super) fn support_action<F>(&mut self, cx: &mut Context<Self>, f: F)
    where
        F: FnOnce(String, String) -> shelldeck_core::Result<manage_support::SupportTicket>
            + Send
            + 'static,
    {
        if !self.app_config.cloud_sync.is_configured() {
            return;
        }
        let base = self.account_base_url();
        let token = self.app_config.cloud_sync.token.clone();
        self.support.update(cx, |v, cx| {
            v.set_loading(true);
            cx.notify();
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { f(base, token) })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(t) => {
                    ws.support.update(cx, |v, cx| {
                        v.set_detail(t, cx);
                        cx.notify();
                    });
                    ws.refresh_support(cx);
                }
                Err(e) => {
                    let msg = cloud_account::user_message(&e);
                    ws.support.update(cx, |v, cx| {
                        v.set_error(msg.clone());
                        cx.notify();
                    });
                    ws.show_toast(msg, ToastLevel::Error, cx);
                }
            });
        })
        .detach();
    }
}
