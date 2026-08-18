use super::*;

impl Workspace {
    /// Fixture réservée aux phases de test visuel du fil Ticket.
    ///
    /// Garder ce code disponible pour les futures validations UI, mais laisser
    /// cet interrupteur désactivé en utilisation normale afin que les listes et
    /// compteurs ne contiennent que les données réellement renvoyées par Manage.
    const ENABLE_TEST_TICKET_SHOWCASE: bool = false;

    /// Staff-only in-memory Ticket counterpart of the Requests thread fixture.
    /// It never reaches Manage and lets the Ticket adapter exercise the same
    /// visual states even though today's Support API only returns basic
    /// messages, internal notes and attachments.
    fn fake_ticket_showcase(agent_name: &str, agent_email: &str) -> manage_support::SupportTicket {
        use manage_support::{
            SupportContact, SupportMessage, SupportMessageDelivery, SupportMessageQuote,
            SupportSla, SupportThreadDraft, SupportThreadState, SupportTicket, SupportTyping,
        };
        use shelldeck_core::config::issues::IssueAttachment;

        static SHOWCASE_NOW: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
        let now = *SHOWCASE_NOW.get_or_init(|| chrono::Utc::now().timestamp_millis() as f64);
        let at = |mins: f64| now - mins * 60_000.0;
        let agent_name = if agent_name.trim().is_empty() {
            "Karim"
        } else {
            agent_name.trim()
        };
        let message = |from: &str, name: &str, text: &str, mins: f64| SupportMessage {
            from: from.to_string(),
            name: (!name.is_empty()).then(|| name.to_string()),
            text: text.to_string(),
            at: at(mins),
            ..Default::default()
        };

        SupportTicket {
            id: crate::support_view::SUPPORT_TICKET_SHOWCASE_ID.to_string(),
            channel: "livechat".to_string(),
            subject: "[DÉMO TICKET] Fil de démonstration — TOUS les cas d'affichage".to_string(),
            contact: SupportContact {
                name: Some("Bruno".to_string()),
                email: Some("bruno@watchme.video".to_string()),
                phone: None,
            },
            status: "pending".to_string(),
            unread: false,
            assignee: agent_email.to_string(),
            last_at: at(0.05),
            msg_count: 9,
            last_preview: "Je viens de relancer, laissez-moi 5 minutes.".to_string(),
            priority: "high".to_string(),
            tags: vec!["Fil de démonstration".to_string()],
            sla: SupportSla::default(),
            messages: vec![
                message(
                    "contact",
                    "Bruno",
                    "Depuis hier, les vidéos uploadées sur WatchMe ne se lisent plus. L'upload aboutit, la miniature apparaît, mais le lecteur reste noir. Ça touche les trois comptes qu'on a testés — plus de détails dans le lien.",
                    3_100.0,
                ),
                SupportMessage {
                    kind: "status".to_string(),
                    from: "note".to_string(),
                    name: Some(agent_name.to_string()),
                    text: format!("{agent_name} a fait passer le ticket d'Ouvert à En attente."),
                    at: at(3_050.0),
                    ..Default::default()
                },
                SupportMessage {
                    attachments: vec![IssueAttachment {
                        id: "fake-ticket-image".to_string(),
                        filename: "df-h-media-01.png".to_string(),
                        content_type: "image/png".to_string(),
                        bytes: 218_432,
                        width: Some(640),
                        height: Some(180),
                        created_by: agent_name.to_string(),
                        created_at: at(3_000.0),
                        ..Default::default()
                    }],
                    delivery: Some(SupportMessageDelivery {
                        status: "read".to_string(),
                        channel: "livechat".to_string(),
                        at: at(32.0),
                        error: String::new(),
                    }),
                    channel: "livechat".to_string(),
                    ..message(
                        "agent",
                        agent_name,
                        "J'ai reproduit sur `video-12`. Le transcodage part mais s'arrête à 40 % — le disque de `media-01` est plein. Voici l'écran `df -h` :",
                        3_000.0,
                    )
                },
                SupportMessage {
                    kind: "github".to_string(),
                    from: "note".to_string(),
                    name: Some(agent_name.to_string()),
                    text: "Liée à webdesign29/activ#3007 — « Lecteur vidéo bloqué après transcodage »".to_string(),
                    at: at(2_940.0),
                    ..Default::default()
                },
                SupportMessage {
                    quote: Some(SupportMessageQuote {
                        author: agent_name.to_string(),
                        body: "Voici l'écran df -h".to_string(),
                    }),
                    channel: "livechat".to_string(),
                    ..message(
                        "contact",
                        "Bruno",
                        "Merci, c'est aligné avec ce qu'on voit côté prod. On a mis en place la rotation nocturne, le disque est redescendu à 62 %. Est-ce qu'on peut relancer la file ? Détails ici https://docs.activ-com.fr/ops/media/backfill",
                        1_400.0,
                    )
                },
                SupportMessage {
                    delivery: Some(SupportMessageDelivery {
                        status: "sent".to_string(),
                        channel: "livechat".to_string(),
                        at: at(1_295.0),
                        error: String::new(),
                    }),
                    channel: "livechat".to_string(),
                    ..message(
                        "agent",
                        agent_name,
                        "File relancée. J'ai posé trois garde-fous :\n\n## Vérifications\n\n- [x] Alerte disque à 80 %\n- [x] Nettoyage nocturne des `.tmp` de transcodage\n- [ ] Reprise automatique après échec 5xx\n\nLe dernier est en cours de review, PR :\n\n```text\ngit diff --stat HEAD~1\n apps/media/src/queue.rs  | 44 +++++++++\n apps/media/src/retry.rs  | 12 +--\n```",
                        1_300.0,
                    )
                },
                SupportMessage {
                    attachments: vec![
                        IssueAttachment {
                            id: "fake-ticket-pdf".to_string(),
                            filename: "rapport-incident-2026-08.pdf".to_string(),
                            content_type: "application/pdf".to_string(),
                            bytes: 219_136,
                            created_by: "Bruno".to_string(),
                            created_at: at(1_200.0),
                            ..Default::default()
                        },
                        IssueAttachment {
                            id: "fake-ticket-link".to_string(),
                            url: "https://watchme.video/status".to_string(),
                            viewer_url: "https://watchme.video/status".to_string(),
                            filename: "watchme.video/status".to_string(),
                            content_type: "text/uri-list".to_string(),
                            created_by: "Bruno".to_string(),
                            created_at: at(1_200.0),
                            ..Default::default()
                        },
                    ],
                    channel: "email".to_string(),
                    ..message("contact", "Bruno", "", 1_200.0)
                },
                SupportMessage {
                    kind: "dispatch".to_string(),
                    from: "note".to_string(),
                    name: Some(agent_name.to_string()),
                    text: "Dispatché vers fleet · media-01 — script backfill-video-queue".to_string(),
                    at: at(40.0),
                    ..Default::default()
                },
                SupportMessage {
                    delivery: Some(SupportMessageDelivery {
                        status: "failed".to_string(),
                        channel: "livechat".to_string(),
                        at: at(0.1),
                        error: "Échec d'envoi".to_string(),
                    }),
                    channel: "livechat".to_string(),
                    ..message(
                        "agent",
                        agent_name,
                        "Je viens de relancer, laissez-moi 5 minutes.",
                        0.1,
                    )
                },
            ],
            thread_state: SupportThreadState {
                typing: vec![SupportTyping {
                    author: "Ludo".to_string(),
                    at: at(0.5),
                }],
                suggested_reply: Some(SupportThreadDraft {
                    body: "Bonjour Bruno, la file est relancée, disque à 62 % après rotation. Je surveille jusqu'à ce que le rattrapage soit fini — il devrait être bouclé d'ici deux heures. Je reviens vers vous à ce moment.".to_string(),
                    model: "Claude Sonnet".to_string(),
                    at: at(0.4),
                }),
                local_draft: Some(SupportThreadDraft {
                    body: "Je vérifie le pipeline transcodage et je reviens vers toi…".to_string(),
                    model: String::new(),
                    at: at(0.3),
                }),
            },
            ..Default::default()
        }
    }

    fn inject_ticket_showcase(
        tickets: &mut Vec<manage_support::SupportTicket>,
        me: &manage_support::SupportMe,
        enabled: bool,
    ) {
        if !enabled {
            return;
        }
        tickets.retain(|ticket| ticket.id != crate::support_view::SUPPORT_TICKET_SHOWCASE_ID);
        tickets.insert(0, Self::fake_ticket_showcase(&me.name, &me.email));
    }

    pub(super) fn sync_support_poll(&mut self, cx: &mut Context<Self>) {
        let want = self.should_poll(super::polling::PolledSurface::Support);
        if want {
            if self._support_poll_task.is_none() {
                let task = cx.spawn(async move |this, cx: &mut AsyncApp| loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(30))
                        .await;
                    let keep_going = this
                        .update(cx, |ws, cx| {
                            if ws.should_poll(super::polling::PolledSurface::Support) {
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
        if !self.can_access_mode(AppMode::Support) {
            return;
        }
        let base = self.account_base_url();
        let token = self.app_config.cloud_sync.token.clone();
        let need_agents = !self.support.read(cx).has_agents();
        let inject_showcase = Self::ENABLE_TEST_TICKET_SHOWCASE && self.is_inklura_support();
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
                        Ok(mut r) => {
                            Self::inject_ticket_showcase(&mut r.tickets, &r.me, inject_showcase);
                            if inject_showcase {
                                r.counts.all = r.counts.all.saturating_add(1);
                                r.counts.pending = r.counts.pending.saturating_add(1);
                                if r.me.email.trim().is_empty() {
                                    r.counts.unassigned = r.counts.unassigned.saturating_add(1);
                                } else {
                                    r.counts.mine = r.counts.mine.saturating_add(1);
                                }
                            }
                            v.set_list(r.tickets, r.counts, r.me)
                        }
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
        if id == crate::support_view::SUPPORT_TICKET_SHOWCASE_ID {
            let (name, email) = self
                .app_config
                .account
                .as_ref()
                .map(|account| (account.name.clone(), account.email.clone()))
                .unwrap_or_default();
            let ticket = Self::fake_ticket_showcase(&name, &email);
            self.support
                .update(cx, |view, cx| view.set_detail(ticket, cx));
            return;
        }
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
        if !self.can_access_mode(AppMode::Support) {
            return;
        }
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
                self.support
                    .update(cx, |view, cx| view.set_issue_ai_pending(true, cx));
                self.open_ai_workflow(AiWorkflowTarget::IssueReply { issue_id }, cx)
            }
            SupportViewEvent::PublishIssueAiDraft => {
                self.support
                    .update(cx, |view, cx| view.publish_issue_ai_draft(cx));
            }
            SupportViewEvent::DiscardIssueAiDraft => {
                self.support
                    .update(cx, |view, cx| view.discard_issue_ai_draft(cx));
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
                if id == crate::support_view::SUPPORT_TICKET_SHOWCASE_ID {
                    self.support.update(cx, |view, cx| {
                        view.append_ticket_showcase_message(&id, text, note, attachments, cx);
                    });
                    return;
                }
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
                if id == crate::support_view::SUPPORT_TICKET_SHOWCASE_ID {
                    self.support.update(cx, |view, cx| {
                        view.update_ticket_showcase(
                            &id,
                            |ticket| ticket.status = status.clone(),
                            cx,
                        );
                    });
                    return;
                }
                self.support_action(cx, move |b, t| ms::support_status(&b, &t, &id, &status));
            }
            SupportViewEvent::SetPriority { id, priority } => {
                if id == crate::support_view::SUPPORT_TICKET_SHOWCASE_ID {
                    self.support.update(cx, |view, cx| {
                        view.update_ticket_showcase(
                            &id,
                            |ticket| ticket.priority = priority.clone(),
                            cx,
                        );
                    });
                    return;
                }
                self.support_action(cx, move |b, t| ms::support_priority(&b, &t, &id, &priority));
            }
            SupportViewEvent::Assign { id, assignee } => {
                if id == crate::support_view::SUPPORT_TICKET_SHOWCASE_ID {
                    let assignee = if assignee == "me" {
                        self.support.read(cx).my_email().to_string()
                    } else {
                        assignee
                    };
                    self.support.update(cx, |view, cx| {
                        view.update_ticket_showcase(
                            &id,
                            |ticket| ticket.assignee = assignee.clone(),
                            cx,
                        );
                    });
                    return;
                }
                self.support_action(cx, move |b, t| ms::support_assign(&b, &t, &id, &assignee));
            }
            SupportViewEvent::Resolve { id, resolution } => {
                if id == crate::support_view::SUPPORT_TICKET_SHOWCASE_ID {
                    self.support.update(cx, |view, cx| {
                        view.update_ticket_showcase(
                            &id,
                            |ticket| ticket.status = "closed".to_string(),
                            cx,
                        );
                    });
                    return;
                }
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
            SupportViewEvent::RetryIssueComment {
                issue_id,
                comment_id,
            } => {
                tracing::info!(%issue_id, %comment_id, "Issue delivery retry requested before API support");
                self.show_toast(
                    t!("toast.issue.retry_unavailable").to_string(),
                    ToastLevel::Info,
                    cx,
                );
            }
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
        if !self.can_access_mode(AppMode::Support) {
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
