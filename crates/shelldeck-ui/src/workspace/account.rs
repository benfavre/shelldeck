use super::*;

impl Workspace {
    // --- Cloud account (Inklura Manage) ---

    /// The account/sync base URL, defaulting to the portal if unset.
    pub(super) fn account_base_url(&self) -> String {
        let b = self.app_config.cloud_sync.base_url.trim().to_string();
        if b.is_empty() {
            "https://manage.inklura.fr".to_string()
        } else {
            b
        }
    }

    /// Background whoami at startup: refresh the status dot + account name, and
    /// warn once if the token was revoked remotely. No-op when logged out.
    pub fn check_account_on_startup(&mut self, cx: &mut Context<Self>) {
        if !self.app_config.cloud_sync.is_configured() {
            return;
        }
        let base = self.account_base_url();
        let token = self.app_config.cloud_sync.token.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { cloud_account::whoami(&base, &token) })
                .await;
            let _ = this.update(cx, |ws, cx| {
                match result {
                    Ok(info) => {
                        ws.account_status = AccountStatus::Ok;
                        let refreshed = info.account_info();
                        if !refreshed.name.trim().is_empty() || !refreshed.email.trim().is_empty() {
                            ws.app_config.account = Some(refreshed);
                            let _ = ws.app_config.save();
                        }
                        // Stash the full whoami — the User "Mes informations"
                        // tab renders every field (device label, created_at,
                        // last_seen_at, …) that `AccountInfo` doesn't persist.
                        ws.last_whoami = Some(info);
                        // Token is valid → load the sites directory + activate
                        // the persisted mode (starts the support poll if needed).
                        ws.refresh_sites(cx);
                        ws.refresh_mention_people(cx);
                        ws.activate_current_mode(cx);
                        ws.maybe_show_onboarding(cx);
                    }
                    Err(e) if cloud_account::is_auth_rejected(&e) => {
                        ws.invalidate_cloud_session(cx);
                        ws.account_status = AccountStatus::Rejected;
                        ws.show_toast(
                            t!("toast.session.expired").to_string(),
                            ToastLevel::Warning,
                            cx,
                        );
                    }
                    Err(_) => {
                        ws.account_status = AccountStatus::Offline;
                    }
                }
                // La palette est construite une seule fois, avant que le
                // compte ne soit connu, donc avec `signed_in = false`. Sans
                // cette reconstruction, un portail injoignable au démarrage
                // prive un utilisateur pourtant connecté de toutes ses
                // commandes (Réglages, Synchroniser, Nouvelle demande…) pour
                // le reste de la session — alors que le menu Fichier, lui,
                // les affiche puisqu'il se reconstruit à chaque rendu.
                // Les trois branches en ont besoin : la branche `Ok` elle-même
                // ne rebâtit la palette que si l'annuaire des sites répond.
                ws.refresh_command_palette(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Open the Manage sign-up page in the system browser. Wired to the
    /// welcome landing's secondary CTA — ShellDeck requires an account to
    /// launch (no guest mode), so the "Créer un compte" button funnels
    /// prospects to Manage rather than dropping them into an unusable
    /// classic Dev workspace.
    pub fn open_signup(&mut self, cx: &mut Context<Self>) {
        let url = "https://inklura.fr/signup";
        match cloud_account::open_in_browser(url) {
            Ok(_) => self.show_toast(
                t!("toast.opening_browser").to_string(),
                ToastLevel::Info,
                cx,
            ),
            Err(e) => self.show_toast(
                t!(
                    "toast.open_browser_failed",
                    error = crate::i18n::api_error_message(&e)
                )
                .to_string(),
                ToastLevel::Error,
                cx,
            ),
        }
    }

    /// Whether the pre-login welcome landing should intercept the render.
    /// True whenever the user is not signed in — there is no guest path;
    /// every launch of a fresh install lands here, and every logout brings
    /// the user back. Sign-in (or account creation via `open_signup`) is
    /// the only way past.
    pub(super) fn show_welcome(&self) -> bool {
        !self.signed_in()
    }

    /// Open the password + OIDC login modal.
    pub fn show_login_form(&mut self, cx: &mut Context<Self>) {
        let server = self.account_base_url();
        let device = cloud_account::device_name();
        let form = cx.new(|form_cx| LoginForm::new(server, device, form_cx));

        let sub = cx.subscribe(
            &form,
            |this, _form, event: &LoginFormEvent, cx| match event {
                LoginFormEvent::SubmitPassword { email, password } => {
                    this.start_password_login(email.clone(), password.clone(), cx);
                }
                LoginFormEvent::StartOidc(provider) => {
                    this.start_oidc_login(provider.clone(), cx);
                }
                LoginFormEvent::Cancel => {
                    this.login_form = None;
                    this._login_form_sub = None;
                    cx.notify();
                }
            },
        );

        self.account_menu_open = false;
        self.login_form = Some(form);
        self._login_form_sub = Some(sub);
        cx.notify();
    }

    /// Open the post-login onboarding tour. Callable from Settings replay
    /// or from `maybe_show_onboarding` on first sign-in.
    pub fn show_onboarding(&mut self, cx: &mut Context<Self>) {
        if !self.signed_in() {
            return;
        }
        let allowed_modes = self.allowed_modes();
        let form = cx.new(|form_cx| OnboardingView::new(allowed_modes, form_cx));
        let sub = cx.subscribe(
            &form,
            |this, _form, event: &OnboardingEvent, cx| match event {
                OnboardingEvent::Finished | OnboardingEvent::Skipped => {
                    this.complete_onboarding(cx);
                }
            },
        );
        self.onboarding = Some(form);
        self._onboarding_sub = Some(sub);
        cx.notify();
    }

    /// Show onboarding once per account install when not yet completed.
    pub(super) fn maybe_show_onboarding(&mut self, cx: &mut Context<Self>) {
        if !self.signed_in() || self.app_config.general.onboarding_completed {
            return;
        }
        if self.onboarding.is_some() {
            return;
        }
        self.show_onboarding(cx);
    }

    /// Close the tour and persist completion (skip counts as done).
    pub(super) fn complete_onboarding(&mut self, cx: &mut Context<Self>) {
        self.onboarding = None;
        self._onboarding_sub = None;
        if !self.app_config.general.onboarding_completed {
            self.app_config.general.onboarding_completed = true;
            if let Err(e) = self.app_config.save() {
                tracing::error!("Failed to save onboarding_completed: {}", e);
            }
            self.sync_settings_config(cx);
        }
        cx.notify();
    }

    /// Run password login on a background thread, then apply on success.
    pub(super) fn start_password_login(
        &mut self,
        email: String,
        password: String,
        cx: &mut Context<Self>,
    ) {
        let base = self.account_base_url();
        let device = cloud_account::device_name();
        if let Some(form) = &self.login_form {
            form.update(cx, |f, cx| {
                f.set_busy(true);
                cx.notify();
            });
        }
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(
                    async move { cloud_account::login_password(&base, &email, &password, &device) },
                )
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok((token, account)) => ws.apply_login(token, account, cx),
                Err(e) => {
                    let msg = crate::i18n::api_error_message(&e);
                    if let Some(form) = &ws.login_form {
                        form.update(cx, |f, cx| {
                            f.set_busy(false);
                            f.set_error(msg.clone());
                            cx.notify();
                        });
                    }
                    ws.show_toast(msg, ToastLevel::Error, cx);
                }
            });
        })
        .detach();
    }

    /// Start the browser device-authorize flow: bind a loopback listener, open
    /// the system browser, and wait (background) for the token redirect.
    pub(super) fn start_oidc_login(&mut self, provider: Option<String>, cx: &mut Context<Self>) {
        let base = self.account_base_url();
        let device = cloud_account::device_name();

        let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(l) => l,
            Err(e) => {
                self.show_toast(
                    t!("toast.local_port_open_failed", error = e.to_string()).to_string(),
                    ToastLevel::Error,
                    cx,
                );
                return;
            }
        };
        let port = match listener.local_addr() {
            Ok(a) => a.port(),
            Err(e) => {
                self.show_toast(
                    t!("toast.local_port_read_failed", error = e.to_string()).to_string(),
                    ToastLevel::Error,
                    cx,
                );
                return;
            }
        };
        // Random state: two v4 UUIDs → 64 hex chars, matches [A-Za-z0-9_-]{32,64}.
        let state = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let url =
            cloud_account::browser_connect_url(&base, port, &state, &device, provider.as_deref());

        if let Err(e) = cloud_account::open_in_browser(&url) {
            self.show_toast(
                t!(
                    "toast.open_browser_failed",
                    error = crate::i18n::api_error_message(&e)
                )
                .to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }

        // Dismiss the login surfaces and show progress.
        self.account_menu_open = false;
        self.login_form = None;
        self._login_form_sub = None;
        self.show_toast(
            t!("toast.browser_auth_waiting").to_string(),
            ToastLevel::Info,
            cx,
        );
        cx.notify();

        let state_for_task = state.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    let token = cloud_account::browser_connect_listen(
                        listener,
                        &state_for_task,
                        std::time::Duration::from_secs(180),
                    )?;
                    let who = cloud_account::whoami(&base, &token)?;
                    Ok::<(String, AccountInfo), shelldeck_core::ShellDeckError>((
                        token,
                        who.account_info(),
                    ))
                })
                .await;
            let _ = this.update(cx, |ws, cx| match outcome {
                Ok((token, account)) => ws.apply_login(token, account, cx),
                Err(e) => ws.show_toast(
                    t!(
                        "toast.browser_login_failed",
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

    /// Persist a successful login (enable cloud sync, store token + account),
    /// then sync profiles and report the count.
    pub(super) fn apply_login(
        &mut self,
        token: String,
        account: AccountInfo,
        cx: &mut Context<Self>,
    ) {
        self.app_config.cloud_sync.enabled = true;
        self.app_config.cloud_sync.token = token;
        self.app_config.account = Some(account.clone());
        if let Err(e) = self.app_config.save() {
            tracing::error!("Failed to save config after login: {}", e);
        }
        self.sync_settings_config(cx);
        self.push_account_to_support(cx);
        self.account_status = AccountStatus::Ok;
        self.login_form = None;
        self._login_form_sub = None;
        self.account_menu_open = false;
        self.post_login_splash = Some(PostLoginSplash {
            display_name: account.display_name().to_string(),
            dismissing: false,
        });
        cx.notify();

        // Load the sites directory for the switcher (background, non-blocking).
        self.refresh_sites(cx);
        // Who this account may address in the assistant's `@` picker. Same
        // shape: background, best-effort, silent when the endpoint is absent.
        self.refresh_mention_people(cx);
        // Non-super-admins are forced to User mode; activate whatever mode applies.
        self.activate_current_mode(cx);
        // Kick a whoami to populate `last_whoami` (device label, created_at,
        // last_seen_at) — the login response only carries the AccountInfo
        // subset, but "Mes informations" needs the richer payload.
        self.check_account_on_startup(cx);
        let cfg = self.app_config.cloud_sync.clone();
        let name = account.display_name();
        let splash_started_at = std::time::Instant::now();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    shelldeck_core::config::cloud_sync::sync_now(&cfg, shelldeck_core::VERSION)
                })
                .await;
            if let Some(remaining) = post_login_splash_remaining(splash_started_at.elapsed()) {
                cx.background_executor().timer(remaining).await;
            }
            let _ = this.update(cx, |ws, cx| match result {
                Ok(_stats) => {
                    ws.reload_connections_after_sync(cx);
                    let n = ws
                        .connections
                        .iter()
                        .filter(|c| c.source == ConnectionSource::CloudSync)
                        .count();
                    ws.show_toast(
                        t!("toast.login_synced", name = name.as_str(), count = n).to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                }
                Err(e) => {
                    ws.show_toast(
                        t!(
                            "toast.login_sync_failed",
                            name = name.as_str(),
                            error = crate::i18n::api_error_message(&e)
                        )
                        .to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                }
            });
            let _ = this.update(cx, |ws, cx| {
                if let Some(splash) = &mut ws.post_login_splash {
                    splash.dismissing = true;
                }
                cx.notify();
            });
            cx.background_executor()
                .timer(std::time::Duration::from_millis(POST_LOGIN_SPLASH_FADE_MS))
                .await;
            let _ = this.update(cx, |ws, cx| {
                ws.post_login_splash = None;
                ws.maybe_show_onboarding(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Keep `SettingsView`'s config snapshot aligned with `app_config`.
    /// Settings persists to disk on many small edits (sidebar nav collapse,
    /// font size, …) and emits `ConfigChanged` — if its copy is stale it
    /// would resurrect a logged-out session. Call after login/logout/session
    /// invalidation and whenever the workspace mutates account/cloud_sync.
    pub(super) fn sync_settings_config(&mut self, cx: &mut Context<Self>) {
        let snapshot = self.app_config.clone();
        self.settings.update(cx, |settings, cx| {
            // Only rebuild the Select entities whose backing slice actually
            // changed — otherwise a login/logout/mode switch would nuke
            // every open dropdown popover.
            let old = std::mem::replace(&mut settings.config, snapshot);
            settings.sync_selects_if_changed(&old, cx);
            cx.notify();
        });
    }

    /// Clear Inklura Manage credentials and stop cloud-backed polls/views.
    pub(super) fn invalidate_cloud_session(&mut self, cx: &mut Context<Self>) {
        self.stop_authenticated_runtime(cx);
        // Overwrite the persisted workspace after closing sessions. Otherwise
        // a later launch/account could restore terminals from the old session.
        self.save_workspace_state(cx);
        self.app_config.account = None;
        self.app_config.cloud_sync.token = String::new();
        self.app_config.cloud_sync.enabled = false;
        self.app_config.cloud_sync.active_site_id = None;
        self.app_config.cloud_sync.active_site_label = None;
        if let Err(e) = self.app_config.save() {
            tracing::error!("Failed to save config after session invalidation: {}", e);
        }
        self.sync_settings_config(cx);
        self.push_account_to_support(cx);
        self.account_status = AccountStatus::Unknown;
        self.post_login_splash = None;
        self.mode_transition = None;
        self.account_menu_open = false;
        self.mode_menu_open = false;
        self.settings_open = false;
        self.last_whoami = None;
        self.user_home_tab = UserHomeTab::Home;
        self.site_directory = None;
        self.jean_state = None;
        self.fleet_snapshot = None;
        self.runtime_instance = None;
        self.runtime_awaiting.clear();
        self.runtime_busy = false;
        self.issues_list.clear();
        self.issues_instances.clear();
        self.issues_staff = false;
        self.reset_issue_selection(cx);
        self.issue_new_site_id = None;
        self.rebuild_issue_site_select(cx);
        self.site_menu_open = false;
        self.ai_sheet = None;
        self.ai_workflow_sheet = None;
        self.ai_workflow = None;
        self._ai_workflow_sub = None;
        self.connection_form = None;
        self._form_sub = None;
        self.port_forward_form = None;
        self._pf_form_sub = None;
        self.script_form = None;
        self._script_form_sub = None;
        self.template_browser = None;
        self._template_browser_sub = None;
        self.variable_prompt = None;
        self._variable_prompt_sub = None;
        // The Dock is owned by the application runtime, but its window can be
        // closed from the foreground App context. Its stale handle is cleared
        // lazily by the runtime on the next authenticated open attempt.
        for handle in cx.windows() {
            if let Some(dock) = handle.downcast::<crate::ai_dock::AiDockView>() {
                let _ = dock.update(cx, |_dock, window, _cx| window.remove_window());
            }
        }
        self._support_poll_task = None;
        self._issues_poll = None;
        self._jean_poll_task = None;
        self._fleet_view_poll = None;
        self._runtime_loop = None;
        self._bext_poll = None;
        self.support.update(cx, |support, cx| {
            support.set_list(Vec::new(), Default::default(), Default::default());
            support.set_agents(Vec::new());
            support.set_issues(Vec::new(), false, Vec::new());
            support.set_jean_brief(false, Vec::new(), 0);
            support.clear_selection();
            cx.notify();
        });
        self.sidebar.update(cx, |s, cx| {
            s.set_site_filter(None);
            cx.notify();
        });
        self.activate_current_mode(cx);
        self.refresh_command_palette(cx);
        self.publish_tray_state(cx);
    }

    /// Sign out: revoke the token server-side (best-effort), then clear local
    /// account state and disable cloud sync.
    pub(super) fn logout_account(&mut self, cx: &mut Context<Self>) {
        let base = self.account_base_url();
        let token = self.app_config.cloud_sync.token.clone();
        if !token.is_empty() {
            cx.background_executor()
                .spawn(async move {
                    let _ = cloud_account::logout(&base, &token);
                })
                .detach();
        }

        self.invalidate_cloud_session(cx);
        self.show_toast(t!("toast.logged_out").to_string(), ToastLevel::Info, cx);
        cx.notify();
    }
}
