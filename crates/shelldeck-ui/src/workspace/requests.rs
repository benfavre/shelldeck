use super::*;

fn issue_list_filter_for_mode(
    mode: AppMode,
    support_filter: &issues::IssueListFilter,
) -> issues::IssueListFilter {
    if mode == AppMode::User {
        issues::IssueListFilter {
            mine: true,
            ..Default::default()
        }
    } else {
        support_filter.clone()
    }
}

impl Workspace {
    // --- Hosted issue management (requests) ---

    /// Palette: focus the User-mode "Nouvelle demande" title field.
    pub fn open_new_request(&mut self, cx: &mut Context<Self>) {
        if !self.signed_in() {
            self.show_toast(
                t!("toast.issue.login_required_create").to_string(),
                ToastLevel::Warning,
                cx,
            );
            return;
        }
        if self.can_switch_mode() {
            self.set_mode(AppMode::User, cx);
        }
        self.issue_new_source = "user";
        self.reset_new_request_site_to_active(cx);
        self.user_new_request_sheet_open = true;
        self.sync_issues_poll(cx);
        cx.notify();
    }

    pub(super) fn open_prefilled_request(
        &mut self,
        title: String,
        body: String,
        source: &'static str,
        cx: &mut Context<Self>,
    ) {
        if !self.signed_in() {
            return;
        }
        self.issue_title_state
            .update(cx, |state, cx| state.replace_content(title, cx));
        self.issue_body_state
            .update(cx, |state, cx| state.replace_content(body, cx));
        Self::reset_input(&self.issue_ai_prompt_state.clone(), cx);
        self.issue_new_priority = "normal".to_string();
        self.issue_new_source = source;
        self.reset_new_request_site_to_active(cx);
        self.issue_ai_expanded = false;
        self.issue_ai_loading = false;
        self.issue_ai_error = None;
        self.issue_ai_request_id = self.issue_ai_request_id.wrapping_add(1);
        if self.can_switch_mode() {
            self.set_mode(AppMode::User, cx);
        }
        self.user_new_request_sheet_open = true;
        self.sync_issues_poll(cx);
        cx.notify();
    }

    pub(super) fn open_ai_request_draft(
        &mut self,
        draft: AiGeneratedIssueDraft,
        cx: &mut Context<Self>,
    ) {
        if !self.signed_in() {
            self.show_toast(
                t!("toast.issue.login_required_create").to_string(),
                ToastLevel::Warning,
                cx,
            );
            return;
        }
        self.open_prefilled_request(draft.title, draft.description, "user", cx);
        self.issue_new_priority = draft.priority;
        self.show_toast(
            t!("toast.ai.request_draft_opened").to_string(),
            ToastLevel::Success,
            cx,
        );
        cx.notify();
    }

    /// Reset an `InputState` entity's content back to empty. `set_value` needs
    /// a `Window`, which we don't have in async close callbacks, so we clear
    /// the public `content` field directly (the widget re-reads it on next
    /// paint). Selection state is left at its previous position; since the
    /// content is empty, any range is effectively out of bounds and the
    /// widget clamps it on next input.
    pub(super) fn reset_input(state: &Entity<InputState>, cx: &mut Context<Self>) {
        state.update(cx, |s, cx| {
            s.reset(cx);
        });
    }

    pub(super) fn attachment_drafts(&self, target: IssueAttachmentTarget) -> &Vec<AttachmentDraft> {
        match target {
            IssueAttachmentTarget::NewRequest => &self.issue_new_attachments,
            IssueAttachmentTarget::Comment => &self.issue_comment_attachments,
        }
    }

    pub(super) fn attachment_drafts_mut(
        &mut self,
        target: IssueAttachmentTarget,
    ) -> &mut Vec<AttachmentDraft> {
        match target {
            IssueAttachmentTarget::NewRequest => &mut self.issue_new_attachments,
            IssueAttachmentTarget::Comment => &mut self.issue_comment_attachments,
        }
    }

    pub(super) fn add_attachment_draft(
        &mut self,
        target: IssueAttachmentTarget,
        draft: AttachmentDraft,
        cx: &mut Context<Self>,
    ) {
        let drafts = self.attachment_drafts_mut(target);
        if drafts.len() >= issues::ISSUE_ATTACHMENT_MAX_COUNT {
            self.show_toast(
                t!(
                    "toast.issue.attachment_limit",
                    count = issues::ISSUE_ATTACHMENT_MAX_COUNT
                )
                .to_string(),
                ToastLevel::Warning,
                cx,
            );
            return;
        }
        drafts.push(draft);
        cx.notify();
    }

    pub(super) fn import_attachment_paths(
        &mut self,
        target: IssueAttachmentTarget,
        paths: Vec<std::path::PathBuf>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if generation != self.issue_attachment_generation {
            return;
        }
        let remaining =
            issues::ISSUE_ATTACHMENT_MAX_COUNT.saturating_sub(self.attachment_drafts(target).len());
        if remaining == 0 {
            self.show_toast(
                t!(
                    "toast.issue.attachment_limit",
                    count = issues::ISSUE_ATTACHMENT_MAX_COUNT
                )
                .to_string(),
                ToastLevel::Warning,
                cx,
            );
            return;
        }
        self.issue_attachment_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let loaded = cx
                .background_executor()
                .spawn(async move {
                    paths
                        .into_iter()
                        .take(remaining)
                        .map(|path| AttachmentDraft::from_path(&path))
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                ws.issue_attachment_busy = false;
                if generation != ws.issue_attachment_generation {
                    cx.notify();
                    return;
                }
                for result in loaded {
                    match result {
                        Ok(draft) => ws.add_attachment_draft(target, draft, cx),
                        Err(error) => ws.show_toast(
                            t!("toast.issue.attachment_failed", error = error).to_string(),
                            ToastLevel::Error,
                            cx,
                        ),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn pick_issue_attachments(
        &mut self,
        target: IssueAttachmentTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let receiver = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(t!("user.requests.attachments.choose").to_string().into()),
            starting_directory: None,
        });
        let generation = self.issue_attachment_generation;
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let _ = this.update(cx, |ws, cx| {
                ws.import_attachment_paths(target, paths, generation, cx)
            });
        })
        .detach();
    }

    pub(super) fn paste_issue_attachment(
        &mut self,
        target: IssueAttachmentTarget,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        let image = item.entries().iter().find_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(image),
            _ => None,
        });
        let Some(image) = image else { return false };
        match draft_from_clipboard_image(image) {
            Ok(draft) => self.add_attachment_draft(target, draft, cx),
            Err(error) => self.show_toast(
                t!("toast.issue.attachment_failed", error = error).to_string(),
                ToastLevel::Error,
                cx,
            ),
        }
        true
    }

    pub(super) fn import_issue_attachment_url(
        &mut self,
        target: IssueAttachmentTarget,
        cx: &mut Context<Self>,
    ) {
        let url = self
            .issue_attachment_url_state
            .read(cx)
            .content()
            .trim()
            .to_string();
        if url.is_empty() || self.issue_attachment_busy {
            return;
        }
        self.issue_attachment_busy = true;
        let generation = self.issue_attachment_generation;
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { issues::download_issue_image_url(&url) })
                .await;
            let _ = this.update(cx, |ws, cx| {
                ws.issue_attachment_busy = false;
                if generation != ws.issue_attachment_generation {
                    cx.notify();
                    return;
                }
                match result.and_then(|upload| {
                    AttachmentDraft::from_bytes(upload.filename, upload.bytes)
                        .map_err(shelldeck_core::ShellDeckError::Connection)
                }) {
                    Ok(draft) => {
                        ws.add_attachment_draft(target, draft, cx);
                        Self::reset_input(&ws.issue_attachment_url_state.clone(), cx);
                        ws.issue_attachment_url_open = false;
                    }
                    Err(error) => ws.show_toast(
                        t!(
                            "toast.issue.attachment_failed",
                            error = crate::i18n::api_error_message(&error)
                        )
                        .to_string(),
                        ToastLevel::Error,
                        cx,
                    ),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn capture_issue_attachment(
        &mut self,
        target: IssueAttachmentTarget,
        cx: &mut Context<Self>,
    ) {
        if self.issue_attachment_busy {
            return;
        }
        self.issue_attachment_busy = true;
        let generation = self.issue_attachment_generation;
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { capture_region() })
                .await;
            let _ = this.update(cx, |ws, cx| {
                ws.issue_attachment_busy = false;
                if generation != ws.issue_attachment_generation {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(draft) => ws.open_issue_capture_annotator(target, draft, cx),
                    Err(error) => ws.show_toast(
                        t!("toast.issue.attachment_failed", error = error).to_string(),
                        ToastLevel::Warning,
                        cx,
                    ),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn open_issue_capture_annotator(
        &mut self,
        target: IssueAttachmentTarget,
        draft: AttachmentDraft,
        cx: &mut Context<Self>,
    ) {
        let parent = cx.entity().downgrade();
        let cancel_parent = parent.clone();
        let apply_parent = parent;
        let annotator = cx.new(|cx| {
            AttachmentAnnotator::new(
                draft,
                move |cx| {
                    if let Some(parent) = cancel_parent.upgrade() {
                        parent.update(cx, |this, cx| {
                            this.issue_capture_annotator = None;
                            cx.notify();
                        });
                    }
                },
                move |draft, cx| {
                    if let Some(parent) = apply_parent.upgrade() {
                        parent.update(cx, |this, cx| {
                            this.issue_capture_annotator = None;
                            this.add_attachment_draft(target, draft, cx);
                        });
                    }
                },
                cx,
            )
        });
        self.issue_capture_annotator = Some(annotator);
        cx.notify();
    }

    /// Close the "Nouvelle demande" sheet. Plays the exit animation first
    /// (sheet is kept mounted with `dismissing = true`), then clears the state
    /// once the animation duration has elapsed.
    pub(super) fn close_new_request_sheet(&mut self, cx: &mut Context<Self>) {
        if self.user_new_request_sheet_dismissing || !self.user_new_request_sheet_open {
            return;
        }
        self.user_new_request_sheet_dismissing = true;
        self.issue_attachment_generation = self.issue_attachment_generation.wrapping_add(1);
        self.issue_ai_request_id = self.issue_ai_request_id.wrapping_add(1);
        self.issue_ai_expanded = false;
        self.issue_ai_loading = false;
        self.issue_ai_error = None;
        self.issue_capture_annotator = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(SHEET_ANIM_MS))
                .await;
            let _ = this.update(cx, |ws, cx| {
                ws.user_new_request_sheet_open = false;
                ws.user_new_request_sheet_dismissing = false;
                Self::reset_input(&ws.issue_title_state.clone(), cx);
                Self::reset_input(&ws.issue_body_state.clone(), cx);
                Self::reset_input(&ws.issue_ai_prompt_state.clone(), cx);
                Self::reset_input(&ws.issue_attachment_url_state.clone(), cx);
                ws.issue_attachment_url_open = false;
                ws.issue_new_attachments.clear();
                ws.issue_new_source = "user";
                ws.issue_new_site_id = None;
                ws.rebuild_issue_site_select(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Close the User-mode issue detail sheet. Same delayed-unmount pattern
    /// as `close_new_request_sheet`.
    pub(super) fn close_user_issue_detail(&mut self, cx: &mut Context<Self>) {
        if self.user_issue_detail_dismissing || self.issue_selected.is_none() {
            return;
        }
        self.user_issue_detail_dismissing = true;
        self.issue_attachment_generation = self.issue_attachment_generation.wrapping_add(1);
        self.issue_capture_annotator = None;
        self.issue_thread_link_action = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(SHEET_ANIM_MS))
                .await;
            let _ = this.update(cx, |ws, cx| {
                ws.issue_selected = None;
                ws.issue_detail = None;
                ws.user_issue_detail_dismissing = false;
                Self::reset_input(&ws.issue_comment_state.clone(), cx);
                Self::reset_input(&ws.issue_attachment_url_state.clone(), cx);
                ws.issue_attachment_url_open = false;
                ws.issue_comment_attachments.clear();
                ws.issue_comment_attachments_open = false;
                ws.issue_thread_link_action = None;
                cx.notify();
            });
        })
        .detach();
    }

    /// Palette: open the Support console's Demandes tab.
    pub fn open_support_requests(&mut self, cx: &mut Context<Self>) {
        if !self.can_access_mode(AppMode::Support) {
            return;
        }
        if !self.app_config.cloud_sync.is_configured() {
            self.show_toast(
                t!("toast.issue.login_required_list").to_string(),
                ToastLevel::Warning,
                cx,
            );
            return;
        }
        self.set_mode(AppMode::Support, cx);
        self.support.update(cx, |v, cx| {
            v.set_section(crate::support_view::SupportSection::Requests);
            cx.notify();
        });
        self.refresh_issues(cx);
        cx.notify();
    }

    /// A Jean/issues surface is on screen (User home, or Support mode).
    pub(super) fn issues_relevant(&self) -> bool {
        self.should_poll(super::polling::PolledSurface::Issues)
    }

    /// Sentinel id of the staff-only thread showcase — used both to insert the
    /// fixture and to short-circuit `select_issue` so we don't ask Manage for a
    /// row it does not have (which would return a 404 toast).
    const FAKE_SHOWCASE_ID: &'static str = "fake-thread-showcase";

    /// Fixture réservée aux phases de test visuel du fil Demande.
    ///
    /// Garder ce code disponible pour les futures validations UI, mais laisser
    /// cet interrupteur désactivé en utilisation normale afin que la liste ne
    /// contienne que les demandes réellement renvoyées par Manage.
    const ENABLE_TEST_REQUEST_SHOWCASE: bool = false;

    /// Injects a fictional request into the list only when its test-phase
    /// switch is enabled and the account is staff (super-admin or Inklura
    /// Support). Every real user has zero access to it — Manage never returns
    /// it. It exists so we can look at the thread design against a controlled
    /// body: 13 cases at once, exactly what the mockup shows in
    /// `docs/design/assistant-refonte.html`.
    ///
    /// Called from `refresh_issues` after the real list comes back, so the
    /// fixture is refreshed each poll and never persists to disk.
    fn inject_thread_showcase(list: &mut Vec<Issue>, staff: bool) {
        if !Self::ENABLE_TEST_REQUEST_SHOWCASE || !staff {
            return;
        }
        use shelldeck_core::config::issues::{
            IssueAttachment, IssueComment, IssueCommentDelivery, IssueCommentQuote, IssueGithub,
            IssueThreadDraft, IssueThreadState, IssueTyping,
        };
        static SHOWCASE_NOW: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
        let now = *SHOWCASE_NOW.get_or_init(|| chrono::Utc::now().timestamp_millis() as f64);
        let m = |mins: f64| now - mins * 60_000.0;
        let comment = |kind: &str, author: &str, body: &str, mins: f64| IssueComment {
            id: format!("fake-{}-{}", kind, mins as i64),
            author: author.to_string(),
            body: body.to_string(),
            kind: kind.to_string(),
            at: m(mins),
            channel: String::new(),
            quote: None,
            delivery: None,
            attachments: Vec::new(),
        };
        let showcase = Issue {
            id: Self::FAKE_SHOWCASE_ID.to_string(),
            tenant_id: "shelldeck".to_string(),
            tenant_name: "ShellDeck (démo)".to_string(),
            site_id: None,
            site_label: Some("Fil de démonstration".to_string()),
            title: "[DÉMO] Fil de démonstration — TOUS les cas d'affichage".to_string(),
            status: "in_progress".to_string(),
            priority: "high".to_string(),
            source: "slack".to_string(),
            requested_by: "Bruno".to_string(),
            assignee: "Karim".to_string(),
            comment_count: 8,
            attachment_count: 3,
            github: Some(IssueGithub {
                url: "https://github.com/shelldeck/demo/issues/1".to_string(),
                number: 1,
                state: "open".to_string(),
            }),
            job_count: 1,
            created_at: m(3_100.0),
            updated_at: m(0.05),
            body: "Depuis hier, les vidéos uploadées sur WatchMe ne se lisent plus. L'upload aboutit, la miniature apparaît, mais le lecteur reste noir. Ça touche les trois comptes qu'on a testés — plus de détails dans le lien.".to_string(),
            comments: vec![
                // 1 · note statut
                comment("status", "Karim", "Karim a fait passer la demande d'À traiter à En cours.", 3_050.0),
                // 2 · réponse staff (moi) — image + statut lu
                IssueComment {
                    id: "fake-c-2".to_string(),
                    author: "Karim".to_string(),
                    body: "J'ai reproduit sur `video-12`. Le transcodage part mais s'arrête à 40 % — le disque de `media-01` est plein. Voici l'écran `df -h` :".to_string(),
                    kind: "comment".to_string(),
                    at: m(3_000.0),
                    channel: "slack".to_string(),
                    quote: None,
                    delivery: Some(IssueCommentDelivery {
                        status: "read".to_string(),
                        channel: "slack".to_string(),
                        at: m(32.0),
                        error: String::new(),
                    }),
                    attachments: vec![IssueAttachment {
                        id: "fake-a-1".to_string(),
                        share_id: "".to_string(),
                        url: "".to_string(),
                        viewer_url: "".to_string(),
                        filename: "df-h-media-01.png".to_string(),
                        content_type: "image/png".to_string(),
                        bytes: 218_432,
                        width: Some(640),
                        height: Some(180),
                        created_by: "Karim".to_string(),
                        created_at: m(3_000.0),
                    }],
                },
                // 3 · note GitHub
                comment("github", "Karim", "Liée à webdesign29/activ#3007 — « Lecteur vidéo bloqué après transcodage »", 2_940.0),
                // 4 · réponse client — citation + lien, le changement de jour
                // est dérivé de ce timestamp par le renderer.
                IssueComment {
                    id: "fake-c-quote".to_string(),
                    author: "Bruno".to_string(),
                    body: "Merci, c'est aligné avec ce qu'on voit côté prod. On a mis en place la rotation nocturne, le disque est redescendu à 62 %. Est-ce qu'on peut relancer la file ? Détails ici https://docs.activ-com.fr/ops/media/backfill".to_string(),
                    kind: "comment".to_string(),
                    at: m(1_400.0),
                    channel: "slack".to_string(),
                    quote: Some(IssueCommentQuote {
                        author: "Karim".to_string(),
                        body: "Voici l'écran df -h".to_string(),
                    }),
                    delivery: None,
                    attachments: Vec::new(),
                },
                // 5 · réponse staff — Markdown riche + statut envoyé.
                IssueComment {
                    id: "fake-c-rich".to_string(),
                    author: "Karim".to_string(),
                    body: "File relancée. J'ai posé trois garde-fous :\n\n## Vérifications\n\n- [x] Alerte disque à 80 %\n- [x] Nettoyage nocturne des `.tmp` de transcodage\n- [ ] Reprise automatique après échec 5xx\n\nLe dernier est en cours de review, PR :\n\n```text\ngit diff --stat HEAD~1\n apps/media/src/queue.rs  | 44 +++++++++\n apps/media/src/retry.rs  | 12 +--\n```".to_string(),
                    kind: "comment".to_string(),
                    at: m(1_300.0),
                    channel: "slack".to_string(),
                    quote: None,
                    delivery: Some(IssueCommentDelivery {
                        status: "sent".to_string(),
                        channel: "slack".to_string(),
                        at: m(1_295.0),
                        error: String::new(),
                    }),
                    attachments: Vec::new(),
                },
                // 6 · pièces jointes multiples sans corps : fichier + URL.
                IssueComment {
                    id: "fake-c-attachments".to_string(),
                    author: "Bruno".to_string(),
                    body: String::new(),
                    kind: "comment".to_string(),
                    at: m(1_200.0),
                    channel: "email".to_string(),
                    quote: None,
                    delivery: None,
                    attachments: vec![
                        IssueAttachment {
                            id: "fake-a-pdf".to_string(),
                            share_id: String::new(),
                            url: String::new(),
                            viewer_url: String::new(),
                            filename: "rapport-incident-2026-08.pdf".to_string(),
                            content_type: "application/pdf".to_string(),
                            bytes: 219_136,
                            width: None,
                            height: None,
                            created_by: "Bruno".to_string(),
                            created_at: m(1_200.0),
                        },
                        IssueAttachment {
                            id: "fake-a-link".to_string(),
                            share_id: String::new(),
                            url: "https://watchme.video/status".to_string(),
                            viewer_url: "https://watchme.video/status".to_string(),
                            filename: "watchme.video/status".to_string(),
                            content_type: "text/uri-list".to_string(),
                            bytes: 0,
                            width: None,
                            height: None,
                            created_by: "Bruno".to_string(),
                            created_at: m(1_200.0),
                        },
                    ],
                },
                // 7 · note système : dispatché.
                comment("system", "Karim", "Dispatché vers fleet · media-01 — script backfill-video-queue", 40.0),
                // 8 · dernier envoi : échec + action de retry réservée à la
                // future route idempotente.
                IssueComment {
                    id: "fake-c-failed".to_string(),
                    author: "Karim".to_string(),
                    body: "Je viens de relancer, laissez-moi 5 minutes.".to_string(),
                    kind: "comment".to_string(),
                    at: m(0.1),
                    channel: "slack".to_string(),
                    quote: None,
                    delivery: Some(IssueCommentDelivery {
                        status: "failed".to_string(),
                        channel: "slack".to_string(),
                        at: m(0.1),
                        error: "Échec d'envoi".to_string(),
                    }),
                    attachments: Vec::new(),
                },
            ],
            attachments: Vec::new(),
            job_ids: vec!["fake-job-1".to_string()],
            thread_state: IssueThreadState {
                typing: vec![IssueTyping {
                    author: "Ludo".to_string(),
                    at: m(0.5),
                }],
                suggested_reply: Some(IssueThreadDraft {
                    body: "Bonjour Bruno, la file est relancée, disque à 62 % après rotation. Je surveille jusqu'à ce que le rattrapage soit fini — il devrait être bouclé d'ici deux heures. Je reviens vers vous à ce moment.".to_string(),
                    model: "Claude Sonnet".to_string(),
                    at: m(0.4),
                }),
                local_draft: Some(IssueThreadDraft {
                    body: "Je vérifie le pipeline transcodage et je reviens vers toi…".to_string(),
                    model: String::new(),
                    at: m(0.3),
                }),
            },
        };
        // En tête de liste : la démo saute aux yeux, et une fixture SANS body
        // ne s'affiche jamais que dans la liste — ici on met tout, corps ET
        // commentaires, pour que l'onglet aille droit au fil.
        list.insert(0, showcase);
    }

    pub(super) fn refresh_issues(&mut self, cx: &mut Context<Self>) {
        let Some((base, token)) = self.fleet_base_token() else {
            return;
        };
        // User mode is a personal surface, including when an internal staff
        // account switches down to it. Ask Manage for owned requests only;
        // Support mode retains its triage filters and broader staff scope.
        let filter = issue_list_filter_for_mode(self.effective_mode(), &self.issues_filter);
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { issues::list_issues(&base, &token, &filter) })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(list) => {
                    ws.issues_list = list.issues.clone();
                    ws.issues_staff = list.staff;
                    ws.issues_instances = list.instances.clone();
                    // Fixture de phase de test uniquement : son interrupteur
                    // reste coupé en utilisation normale. Même activée, elle
                    // n'affecte que la mémoire et n'est jamais envoyée à Manage.
                    Self::inject_thread_showcase(&mut ws.issues_list, ws.issues_staff);
                    ws.push_issues_to_support(cx);
                    cx.notify();
                }
                Err(e) => ws.show_toast(
                    t!(
                        "toast.issue.list_failed",
                        error = crate::i18n::api_error_message(&e)
                    )
                    .to_string(),
                    ToastLevel::Error,
                    cx,
                ),
            });
        })
        .detach();
    }

    pub(super) fn push_issues_to_support(&mut self, cx: &mut Context<Self>) {
        let issues = self.issues_list.clone();
        let staff = self.issues_staff;
        let instances = self.issues_instances.clone();
        let detail = self.issue_detail.clone();
        let (acc_name, acc_email) = self
            .app_config
            .account
            .as_ref()
            .map(|a| (a.name.clone(), a.email.clone()))
            .unwrap_or_default();
        self.support.update(cx, |v, cx| {
            v.set_account(&acc_name, &acc_email);
            v.set_issues(issues, staff, instances);
            v.set_issue_detail(detail, cx);
            cx.notify();
        });
    }

    /// Push the current `AppConfig.account` identity to `SupportView` — used
    /// on login/logout transitions so the child's identity cache doesn't
    /// outlive the workspace-owned account state (violation of
    /// `.agents/session-state.md` if it does).
    pub(super) fn push_account_to_support(&mut self, cx: &mut Context<Self>) {
        let (acc_name, acc_email) = self
            .app_config
            .account
            .as_ref()
            .map(|a| (a.name.clone(), a.email.clone()))
            .unwrap_or_default();
        self.support.update(cx, |v, cx| {
            v.set_account(&acc_name, &acc_email);
            cx.notify();
        });
    }

    pub(super) fn sync_issues_poll(&mut self, cx: &mut Context<Self>) {
        if self.issues_relevant() {
            self.refresh_issues(cx);
            if self._issues_poll.is_none() {
                let task = cx.spawn(async move |this, cx: &mut AsyncApp| loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(15))
                        .await;
                    let keep = this
                        .update(cx, |ws, cx| {
                            if ws.issues_relevant() {
                                ws.refresh_issues(cx);
                                true
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);
                    if !keep {
                        break;
                    }
                });
                self._issues_poll = Some(task);
            }
        } else {
            self._issues_poll = None;
        }
    }

    pub fn select_issue(&mut self, id: String, cx: &mut Context<Self>) {
        // La fixture n'existe pas côté Manage : demander son détail renvoie 404
        // et un toast erroné. On prend la version qu'on a déjà en mémoire.
        if id == Self::FAKE_SHOWCASE_ID {
            self.issue_selected = Some(id.clone());
            if let Some(iss) = self.issues_list.iter().find(|i| i.id == id).cloned() {
                self.issue_detail = Some(iss);
            }
            self.push_issues_to_support(cx);
            cx.notify();
            return;
        }
        let Some((base, token)) = self.fleet_base_token() else {
            return;
        };
        if self.issue_selected.as_deref() != Some(id.as_str()) {
            self.issue_attachment_generation = self.issue_attachment_generation.wrapping_add(1);
            self.issue_comment_attachments.clear();
            Self::reset_input(&self.issue_attachment_url_state.clone(), cx);
            self.issue_attachment_url_open = false;
            self.issue_comment_attachments_open = false;
        }
        self.issue_selected = Some(id.clone());
        self.add_activity_entry(
            ActivityEntry::new(
                ActivityKind::Issue,
                t!("activity.issue.open", id = id.as_str()).to_string(),
            )
            .with_target(id.clone(), id.clone())
            .with_action(ActivityAction::OpenIssue),
            cx,
        );
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { issues::get_issue(&base, &token, &id) })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(iss) => {
                    ws.issue_detail = Some(iss);
                    ws.push_issues_to_support(cx);
                    cx.notify();
                }
                Err(e) => ws.show_toast(
                    t!(
                        "toast.issue.detail_failed",
                        error = crate::i18n::api_error_message(&e)
                    )
                    .to_string(),
                    ToastLevel::Error,
                    cx,
                ),
            });
        })
        .detach();
    }

    /// Create a request. `source` = "user" (User mode) or "support".
    /// Replace an existing issue in `issues_list` by id, or prepend it if
    /// absent (matches the server's default `updated_at DESC` order for a
    /// freshly-created row). Called after a server-side mutation returns
    /// the updated record so we don't need an eager list refetch.
    pub(super) fn upsert_issue_in_list(&mut self, iss: Issue) {
        if let Some(pos) = self.issues_list.iter().position(|i| i.id == iss.id) {
            self.issues_list[pos] = iss;
        } else {
            self.issues_list.insert(0, iss);
        }
    }

    /// Drop an issue from `issues_list` by id (soft-delete).
    pub(super) fn remove_issue_from_list(&mut self, id: &str) {
        self.issues_list.retain(|i| i.id != id);
    }

    pub(super) fn create_issue_now(&mut self, draft: NewIssueDraft, cx: &mut Context<Self>) {
        if self.issue_attachment_busy {
            return;
        }
        let title = draft.title.trim().to_string();
        if title.is_empty() {
            return;
        }
        let Some((base, token)) = self.fleet_base_token() else {
            return;
        };
        self.issue_attachment_busy = true;
        self.issue_attachment_generation = self.issue_attachment_generation.wrapping_add(1);
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let created = issues::create_issue(
                        &base,
                        &token,
                        issues::CreateIssue {
                            title: &title,
                            body: &draft.body,
                            priority: &draft.priority,
                            source: draft.source,
                            site_id: &draft.site_id,
                            site_label: &draft.site_label,
                        },
                    )?;
                    if draft.attachments.is_empty() {
                        return Ok::<_, shelldeck_core::ShellDeckError>((created, None));
                    }
                    let uploads = draft
                        .attachments
                        .iter()
                        .map(AttachmentDraft::upload)
                        .collect::<Vec<_>>();
                    match issues::upload_issue_attachments(&base, &token, &created.id, &uploads)
                        .and_then(|receipts| {
                            issues::attach_issue_images(&base, &token, &created.id, &receipts)
                        }) {
                        Ok(updated) => Ok((updated, None)),
                        Err(error) => Ok((created, Some(error))),
                    }
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                ws.issue_attachment_busy = false;
                match result {
                    Ok((iss, attachment_error)) => {
                        let preserve_attachments = attachment_error.is_some();
                        ws.show_toast(
                            t!("toast.issue.created").to_string(),
                            ToastLevel::Success,
                            cx,
                        );
                        ws.add_activity_entry(
                            ActivityEntry::new(
                                ActivityKind::Issue,
                                t!("activity.issue.created", title = iss.title.as_str())
                                    .to_string(),
                            )
                            .with_target(iss.id.clone(), iss.title.clone())
                            .with_action(ActivityAction::OpenIssue),
                            cx,
                        );
                        // Success: close the composer sheet, clear its buffers,
                        // and pop the detail sheet on the newly-created request.
                        ws.user_new_request_sheet_open = false;
                        Self::reset_input(&ws.issue_title_state.clone(), cx);
                        Self::reset_input(&ws.issue_body_state.clone(), cx);
                        Self::reset_input(&ws.issue_ai_prompt_state.clone(), cx);
                        Self::reset_input(&ws.issue_attachment_url_state.clone(), cx);
                        if preserve_attachments {
                            ws.issue_comment_attachments =
                                std::mem::take(&mut ws.issue_new_attachments);
                        } else {
                            ws.issue_new_attachments.clear();
                        }
                        ws.issue_ai_request_id = ws.issue_ai_request_id.wrapping_add(1);
                        ws.issue_ai_expanded = false;
                        ws.issue_ai_loading = false;
                        ws.issue_ai_error = None;
                        ws.issue_new_source = "user";
                        ws.issue_new_site_id = None;
                        ws.rebuild_issue_site_select(cx);
                        ws.upsert_issue_in_list(iss.clone());
                        ws.issue_detail = Some(iss.clone());
                        ws.issue_selected = Some(iss.id.clone());
                        ws.push_issues_to_support(cx);
                        if let Some(error) = attachment_error {
                            ws.show_toast(
                                t!(
                                    "toast.issue.attachment_failed_after_create",
                                    error = crate::i18n::api_error_message(&error)
                                )
                                .to_string(),
                                ToastLevel::Warning,
                                cx,
                            );
                        }
                        cx.notify();
                    }
                    Err(e) => ws.show_toast(
                        t!(
                            "toast.issue.create_failed",
                            error = crate::i18n::api_error_message(&e)
                        )
                        .to_string(),
                        ToastLevel::Error,
                        cx,
                    ),
                }
            });
        })
        .detach();
    }

    /// Comment on the selected issue (users can comment on their own requests).
    pub fn comment_issue_now(&mut self, id: String, body: String, cx: &mut Context<Self>) {
        self.comment_issue_with_images(id, body, Vec::new(), cx);
    }

    pub(super) fn comment_issue_with_images(
        &mut self,
        id: String,
        body: String,
        attachments: Vec<AttachmentDraft>,
        cx: &mut Context<Self>,
    ) {
        if self.issue_attachment_busy {
            return;
        }
        let body = body.trim().to_string();
        if body.is_empty() && attachments.is_empty() {
            return;
        }
        let Some((base, token)) = self.fleet_base_token() else {
            return;
        };
        self.issue_attachment_busy = true;
        self.issue_attachment_generation = self.issue_attachment_generation.wrapping_add(1);
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if attachments.is_empty() {
                        issues::comment_issue(&base, &token, &id, &body)
                    } else {
                        let uploads = attachments
                            .iter()
                            .map(AttachmentDraft::upload)
                            .collect::<Vec<_>>();
                        let receipts =
                            issues::upload_issue_attachments(&base, &token, &id, &uploads)?;
                        issues::comment_issue_with_attachments(&base, &token, &id, &body, &receipts)
                    }
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                ws.issue_attachment_busy = false;
                match result {
                    Ok(iss) => {
                        ws.upsert_issue_in_list(iss.clone());
                        ws.issue_detail = Some(iss);
                        if let Some((id, title)) = ws
                            .issue_detail
                            .as_ref()
                            .map(|detail| (detail.id.clone(), detail.title.clone()))
                        {
                            ws.add_activity_entry(
                                ActivityEntry::new(
                                    ActivityKind::Issue,
                                    t!("activity.issue.commented", title = title.as_str())
                                        .to_string(),
                                )
                                .with_target(id, title)
                                .with_action(ActivityAction::OpenIssue),
                                cx,
                            );
                        }
                        ws.push_issues_to_support(cx);
                        ws.support.update(cx, |view, cx| {
                            view.clear_composer_after_send(cx);
                        });
                        Self::reset_input(&ws.issue_comment_state.clone(), cx);
                        Self::reset_input(&ws.issue_attachment_url_state.clone(), cx);
                        ws.issue_attachment_url_open = false;
                        ws.issue_comment_attachments.clear();
                        ws.issue_comment_attachments_open = false;
                        cx.notify();
                    }
                    Err(e) => {
                        let message = t!(
                            "toast.issue.comment_failed",
                            error = crate::i18n::api_error_message(&e)
                        )
                        .to_string();
                        ws.support.update(cx, |view, cx| {
                            view.set_error(message.clone());
                            cx.notify();
                        });
                        ws.show_toast(message, ToastLevel::Error, cx);
                    }
                }
            });
        })
        .detach();
    }

    /// Generic staff issue action (status/assign/priority/dispatch/github);
    /// installs the updated issue in the list + refreshes the detail. The
    /// 15 s issues poll catches any drift on other rows.
    pub fn issue_staff_action<F>(&mut self, cx: &mut Context<Self>, f: F)
    where
        F: FnOnce(String, String) -> shelldeck_core::Result<Issue> + Send + 'static,
    {
        if !self.can_access_mode(AppMode::Support) || !self.issues_staff {
            return;
        }
        let Some((base, token)) = self.fleet_base_token() else {
            return;
        };
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { f(base, token) })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(iss) => {
                    ws.upsert_issue_in_list(iss.clone());
                    ws.issue_detail = Some(iss);
                    if let Some((id, title)) = ws
                        .issue_detail
                        .as_ref()
                        .map(|detail| (detail.id.clone(), detail.title.clone()))
                    {
                        ws.add_activity_entry(
                            ActivityEntry::new(
                                ActivityKind::Issue,
                                t!("activity.issue.updated", title = title.as_str()).to_string(),
                            )
                            .with_target(id, title)
                            .with_action(ActivityAction::OpenIssue),
                            cx,
                        );
                    }
                    ws.push_issues_to_support(cx);
                    cx.notify();
                }
                Err(e) => ws.show_toast(
                    t!(
                        "toast.issue.staff_failed",
                        error = crate::i18n::api_error_message(&e)
                    )
                    .to_string(),
                    ToastLevel::Error,
                    cx,
                ),
            });
        })
        .detach();
    }

    pub(super) fn apply_issue_triage(
        &mut self,
        issue_id: String,
        proposal: AiIssueTriageProposal,
        cx: &mut Context<Self>,
    ) {
        if !self.issues_staff {
            self.show_toast(
                t!("toast.ai.triage_staff_only").to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }
        let Some(current) = self
            .issue_detail
            .as_ref()
            .filter(|issue| issue.id == issue_id)
            .or_else(|| self.issues_list.iter().find(|issue| issue.id == issue_id))
        else {
            self.show_toast(
                t!("toast.ai.triage_obsolete").to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        };
        if let Some(assignee) = proposal.assignee.as_deref() {
            if !self.support.read(cx).is_known_issue_assignee(assignee) {
                self.show_toast(
                    t!("toast.ai.triage_unknown_assignee", assignee = assignee).to_string(),
                    ToastLevel::Error,
                    cx,
                );
                return;
            }
        }
        let priority = proposal
            .priority
            .filter(|priority| priority != &current.priority);
        let assignee = proposal
            .assignee
            .filter(|assignee| !assignee.eq_ignore_ascii_case(current.assignee.trim()));
        let change_count = usize::from(priority.is_some()) + usize::from(assignee.is_some());
        if change_count == 0 {
            self.show_toast(
                t!("toast.ai.triage_no_changes").to_string(),
                ToastLevel::Info,
                cx,
            );
            return;
        }
        let Some((base, token)) = self.fleet_base_token() else {
            return;
        };
        self.show_toast(
            t!("toast.ai.triage_applying").to_string(),
            ToastLevel::Info,
            cx,
        );
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut updated = None;
                    if let Some(priority) = priority {
                        updated = Some(issues::set_priority(&base, &token, &issue_id, &priority)?);
                    }
                    if let Some(assignee) = assignee {
                        updated = Some(issues::assign(&base, &token, &issue_id, &assignee)?);
                    }
                    updated.ok_or_else(|| {
                        shelldeck_core::ShellDeckError::Config(
                            "issue triage has no applicable changes".to_string(),
                        )
                    })
                })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(issue) => {
                    ws.upsert_issue_in_list(issue.clone());
                    ws.issue_detail = Some(issue.clone());
                    ws.push_issues_to_support(cx);
                    ws.add_activity_entry(
                        ActivityEntry::new(
                            ActivityKind::Issue,
                            t!("activity.issue.updated", title = issue.title.as_str()).to_string(),
                        )
                        .with_target(issue.id, issue.title)
                        .with_action(ActivityAction::OpenIssue),
                        cx,
                    );
                    ws.show_toast(
                        t!("toast.ai.triage_applied", count = change_count).to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                    cx.notify();
                }
                Err(error) => {
                    ws.show_toast(
                        t!(
                            "toast.ai.triage_failed",
                            error = crate::i18n::api_error_message(&error)
                        )
                        .to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                    ws.refresh_issues(cx);
                }
            });
        })
        .detach();
    }

    pub(super) fn apply_support_triage(
        &mut self,
        ticket_id: String,
        proposal: AiIssueTriageProposal,
        cx: &mut Context<Self>,
    ) {
        let selected = self.support.read(cx).selected_ticket_identity();
        if selected.as_ref().map(|(id, _)| id.as_str()) != Some(ticket_id.as_str()) {
            self.show_toast(
                t!("toast.ai.triage_obsolete").to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }
        if let Some(assignee) = proposal.assignee.as_deref() {
            if !self.support.read(cx).is_known_support_assignee(assignee) {
                self.show_toast(
                    t!("toast.ai.triage_unknown_assignee", assignee = assignee).to_string(),
                    ToastLevel::Error,
                    cx,
                );
                return;
            }
        }
        let Some((current_priority, current_assignee)) =
            self.support.read(cx).selected_ticket_triage_state()
        else {
            return;
        };
        let priority = proposal
            .priority
            .filter(|priority| !priority.eq_ignore_ascii_case(current_priority.trim()));
        let assignee = proposal
            .assignee
            .filter(|assignee| !assignee.eq_ignore_ascii_case(current_assignee.trim()));
        let change_count = usize::from(priority.is_some()) + usize::from(assignee.is_some());
        if change_count == 0 {
            self.show_toast(
                t!("toast.ai.triage_no_changes").to_string(),
                ToastLevel::Info,
                cx,
            );
            return;
        }
        let base = self.account_base_url();
        let token = self.app_config.cloud_sync.token.clone();
        self.show_toast(
            t!("toast.ai.triage_applying").to_string(),
            ToastLevel::Info,
            cx,
        );
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut updated = None;
                    if let Some(priority) = priority {
                        updated = Some(manage_support::support_priority(
                            &base, &token, &ticket_id, &priority,
                        )?);
                    }
                    if let Some(assignee) = assignee {
                        updated = Some(manage_support::support_assign(
                            &base, &token, &ticket_id, &assignee,
                        )?);
                    }
                    updated.ok_or_else(|| {
                        shelldeck_core::ShellDeckError::Config(
                            "support triage has no applicable changes".to_string(),
                        )
                    })
                })
                .await;
            let _ = this.update(cx, |workspace, cx| match result {
                Ok(ticket) => {
                    let label = ticket.subject.clone();
                    let id = ticket.id.clone();
                    workspace.support.update(cx, |view, cx| {
                        view.set_detail(ticket, cx);
                    });
                    workspace.add_activity_entry(
                        ActivityEntry::new(
                            ActivityKind::Support,
                            t!("activity.support.updated", subject = label.as_str()).to_string(),
                        )
                        .with_target(id, label)
                        .with_action(ActivityAction::OpenTicket),
                        cx,
                    );
                    workspace.show_toast(
                        t!("toast.ai.triage_applied", count = change_count).to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                    workspace.refresh_support(cx);
                }
                Err(error) => workspace.show_toast(
                    t!(
                        "toast.ai.triage_failed",
                        error = crate::i18n::api_error_message(&error)
                    )
                    .to_string(),
                    ToastLevel::Error,
                    cx,
                ),
            });
        })
        .detach();
    }

    /// Soft-delete a request (owner-or-staff). On success the row is
    /// removed from the local list, the detail pane closed, and any drift
    /// is caught by the 15 s issues poll.
    pub(super) fn delete_issue_now(&mut self, id: String, cx: &mut Context<Self>) {
        let Some((base, token)) = self.fleet_base_token() else {
            return;
        };
        let deleted_id = id.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { issues::delete_issue(&base, &token, &id) })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(_) => {
                    ws.add_activity(
                        t!("activity.issue.deleted", id = deleted_id.as_str()).to_string(),
                        ActivityKind::Issue,
                        cx,
                    );
                    ws.remove_issue_from_list(&deleted_id);
                    if ws.issue_selected.as_deref() == Some(deleted_id.as_str()) {
                        ws.issue_selected = None;
                        ws.issue_detail = None;
                    }
                    ws.push_issues_to_support(cx);
                    ws.show_toast(
                        t!("toast.issue.deleted").to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                    cx.notify();
                }
                Err(e) => ws.show_toast(
                    t!(
                        "toast.issue.delete_failed",
                        error = crate::i18n::api_error_message(&e)
                    )
                    .to_string(),
                    ToastLevel::Error,
                    cx,
                ),
            });
        })
        .detach();
    }

    pub(super) fn delete_issue_attachment_now(
        &mut self,
        id: String,
        attachment_id: String,
        cx: &mut Context<Self>,
    ) {
        let Some((base, token)) = self.fleet_base_token() else {
            return;
        };
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    issues::delete_issue_attachment(&base, &token, &id, &attachment_id)
                })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(issue) => {
                    ws.upsert_issue_in_list(issue.clone());
                    if ws.issue_selected.as_deref() == Some(issue.id.as_str()) {
                        ws.issue_detail = Some(issue);
                    }
                    ws.push_issues_to_support(cx);
                    ws.show_toast(
                        t!("toast.issue.attachment_deleted").to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                    cx.notify();
                }
                Err(error) => ws.show_toast(
                    t!(
                        "toast.issue.attachment_delete_failed",
                        error = crate::i18n::api_error_message(&error)
                    )
                    .to_string(),
                    ToastLevel::Error,
                    cx,
                ),
            });
        })
        .detach();
    }

    pub(super) fn delete_support_attachment_now(
        &mut self,
        id: String,
        attachment_id: String,
        cx: &mut Context<Self>,
    ) {
        self.support_action(cx, move |base, token| {
            manage_support::support_delete_attachment(&base, &token, &id, &attachment_id)
        });
    }

    /// Whether the given issue was filed by the currently signed-in user
    /// (matching `requested_by` against the account name or email — the
    /// server stores `actor = user_name || user_email` so we accept either).
    /// Comparison is trimmed + case-insensitive to tolerate cosmetic drift
    /// between the token payload and whoami.
    pub(super) fn is_my_issue(&self, iss: &Issue) -> bool {
        let Some(a) = self.app_config.account.as_ref() else {
            return false;
        };
        let rb = iss.requested_by.trim().to_ascii_lowercase();
        if rb.is_empty() {
            return false;
        }
        let name = a.name.trim().to_ascii_lowercase();
        let email = a.email.trim().to_ascii_lowercase();
        (!name.is_empty() && rb == name) || (!email.is_empty() && rb == email)
    }

    /// Destructive confirm modal for soft-deleting a request from User mode.
    pub(super) fn render_delete_issue_modal(
        &self,
        id: String,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        let title: SharedString = crate::external_content::external_title(
            &self
                .issue_detail
                .as_ref()
                .filter(|i| i.id == id)
                .map(|i| i.title.clone())
                .or_else(|| {
                    self.issues_list
                        .iter()
                        .find(|i| i.id == id)
                        .map(|i| i.title.clone())
                })
                .unwrap_or_default(),
        )
        .into();

        let close_entity = entity.clone();
        let confirm_entity = entity;
        let confirm_id = id;

        render_issue_delete_dialog(
            title,
            "ws-iss-del",
            move |cx| {
                close_entity.update(cx, |this, cx| {
                    this.confirm_issue_delete = None;
                    cx.notify();
                });
            },
            move |cx| {
                let id = confirm_id.clone();
                confirm_entity.update(cx, |this, cx| {
                    this.confirm_issue_delete = None;
                    this.delete_issue_now(id, cx);
                    cx.notify();
                });
            },
        )
    }

    pub(super) fn render_delete_attachment_modal(
        &self,
        issue_id: String,
        attachment_id: String,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        let close_entity = entity.clone();
        let confirm_entity = entity;
        render_attachment_delete_dialog(
            "ws-attachment-delete",
            move |cx| {
                close_entity.update(cx, |this, cx| {
                    this.confirm_attachment_delete = None;
                    cx.notify();
                });
            },
            move |cx| {
                let issue_id = issue_id.clone();
                let attachment_id = attachment_id.clone();
                confirm_entity.update(cx, |this, cx| {
                    this.confirm_attachment_delete = None;
                    this.delete_issue_attachment_now(issue_id, attachment_id, cx);
                    cx.notify();
                });
            },
        )
    }

    /// Submit the "Nouvelle demande" composer sheet: read the Input states,
    /// hand them to `create_issue_now`. Called from the "Créer" button and
    /// from the Title `Input::on_enter`.
    pub(super) fn generate_new_request_with_ai(&mut self, cx: &mut Context<Self>) {
        if self.issue_ai_loading
            || !self.ai_backend_available()
            || !self.app_config.ai.allows(AiSurface::Issue)
        {
            return;
        }
        let instructions = self
            .issue_ai_prompt_state
            .read(cx)
            .content()
            .trim()
            .to_string();
        if instructions.is_empty() {
            self.issue_ai_error = Some(t!("user.requests.ai.required").to_string());
            cx.notify();
            return;
        }

        self.issue_ai_request_id = self.issue_ai_request_id.wrapping_add(1);
        let request_id = self.issue_ai_request_id;
        self.issue_ai_loading = true;
        self.issue_ai_error = None;
        let context = AiContext::new(
            AiSurface::Issue,
            t!("ai.context.issue_form").to_string(),
            serde_json::json!({
                "draft": {
                    "title": self.issue_title_state.read(cx).content().to_string(),
                    "description": self.issue_body_state.read(cx).content().to_string(),
                    "priority": self.issue_new_priority.clone(),
                },
                "hosts": self.ai_hosts_context_data(),
            }),
        );
        let prompt = format!(
            "{}\n\n{}:\n{}",
            t!("ai.prompt.issue_generate_form"),
            t!("ai.workflow.additional_instructions"),
            instructions
        );
        let config = self.app_config.ai.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let client = create_client(&config)?;
                    let response = client.complete(&prompt, context.clone())?;
                    match parse_generated_issue_draft(&response.text) {
                        Ok(draft) => Ok(draft),
                        Err(first_error) => {
                            let repair_prompt = format!(
                                "{}\n\n{}",
                                prompt,
                                t!(
                                    "ai.prompt.issue_generate_repair",
                                    error = first_error.to_string()
                                )
                            );
                            let repaired = client.complete(&repair_prompt, context)?;
                            parse_generated_issue_draft(&repaired.text)
                        }
                    }
                })
                .await
                .map_err(|error| error.to_string());
            let _ = this.update(cx, |ws, cx| {
                if request_id != ws.issue_ai_request_id || !ws.user_new_request_sheet_open {
                    return;
                }
                ws.issue_ai_loading = false;
                match result {
                    Ok(draft) => {
                        ws.issue_title_state
                            .update(cx, |state, cx| state.replace_content(draft.title, cx));
                        ws.issue_body_state
                            .update(cx, |state, cx| state.replace_content(draft.description, cx));
                        ws.issue_new_priority = draft.priority;
                        ws.issue_ai_error = None;
                    }
                    Err(error) => ws.issue_ai_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn submit_new_request(&mut self, cx: &mut Context<Self>) {
        let title = self.issue_title_state.read(cx).content().to_string();
        let body = self.issue_body_state.read(cx).content().to_string();
        let prio = self.issue_new_priority.clone();
        let source = self.issue_new_source;
        let selected_site = self.issue_new_site_id.as_deref().and_then(|site_id| {
            self.site_directory
                .as_ref()?
                .sites
                .iter()
                .find(|site| site.site_id == site_id)
        });
        let site_id = selected_site
            .map(|site| site.site_id.clone())
            .unwrap_or_default();
        let site_label = selected_site
            .map(ManagedSiteInfo::display_label)
            .unwrap_or_default();
        let attachments = self.issue_new_attachments.clone();
        self.create_issue_now(
            NewIssueDraft {
                title,
                body,
                priority: prio,
                source,
                site_id,
                site_label,
                attachments,
            },
            cx,
        );
    }

    /// Submit the comment composer on the currently-open detail sheet.
    pub(super) fn submit_issue_comment(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.issue_selected.clone() else {
            return;
        };
        let body = self.issue_comment_state.read(cx).content().to_string();
        let attachments = self.issue_comment_attachments.clone();
        if body.trim().is_empty() && attachments.is_empty() {
            return;
        }
        self.comment_issue_with_images(id, body, attachments, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{issue_list_filter_for_mode, issues, AppMode};

    // SDTEST-1433
    #[test]
    fn user_issue_refresh_forces_owner_scope_while_support_keeps_triage_filters() {
        let support = issues::IssueListFilter {
            status: "open".to_string(),
            q: "database".to_string(),
            priority: "urgent".to_string(),
            source: "support".to_string(),
            assignee: "me".to_string(),
            mine: false,
            tenant_id: "tenant-2".to_string(),
            has_github: Some(true),
            since: "2026-07-01T00:00:00Z".to_string(),
        };

        let user = issue_list_filter_for_mode(AppMode::User, &support);
        assert!(user.mine);
        assert!(user.status.is_empty());
        assert!(user.q.is_empty());
        assert!(user.tenant_id.is_empty());
        assert_eq!(user.has_github, None);

        let retained = issue_list_filter_for_mode(AppMode::Support, &support);
        assert!(!retained.mine);
        assert_eq!(retained.status, "open");
        assert_eq!(retained.q, "database");
        assert_eq!(retained.tenant_id, "tenant-2");
        assert_eq!(retained.has_github, Some(true));
    }
}
