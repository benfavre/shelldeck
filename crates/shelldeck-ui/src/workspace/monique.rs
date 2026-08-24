use super::*;

impl Workspace {
    pub(super) fn effective_monique_config(&self) -> Option<MoniqueConfig> {
        if !self.signed_in() {
            return None;
        }
        let server = self
            .site_directory
            .as_ref()
            .and_then(|sites| sites.monique.as_ref());
        MoniqueConfig::resolve_effective(self.app_config.monique.as_ref(), server)
    }

    pub fn has_monique(&self) -> bool {
        self.effective_monique_config().is_some()
    }

    fn monique_surface_visible(&self) -> bool {
        self.should_poll(super::polling::PolledSurface::Monique)
    }

    pub(super) fn update_monique_availability(&mut self, cx: &mut Context<Self>) {
        let configured = self.has_monique();
        let show = configured && self.effective_mode() == AppMode::Dev;
        let support_show = configured && self.can_access_mode(AppMode::Support);
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_monique_available(show);
            cx.notify();
        });
        self.support.update(cx, |support, cx| {
            support.set_monique_available(support_show);
            cx.notify();
        });
    }

    pub(super) fn sync_monique_poll(&mut self, cx: &mut Context<Self>) {
        if !self.monique_surface_visible() {
            self._monique_poll_task = None;
            self._monique_auth_poll_task = None;
            return;
        }
        if !self.monique_view.read(cx).chat_busy() {
            self.refresh_monique_snapshot(cx, self.monique_status.is_none());
        }
        if self._monique_poll_task.is_some() {
            return;
        }
        self._monique_poll_task = Some(cx.spawn(async move |this, cx: &mut AsyncApp| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(10))
                .await;
            let keep = this
                .update(cx, |workspace, cx| {
                    if workspace.monique_surface_visible() {
                        if !workspace.monique_view.read(cx).chat_busy() {
                            workspace.refresh_monique_snapshot(cx, false);
                        }
                        true
                    } else {
                        false
                    }
                })
                .unwrap_or(false);
            if !keep {
                break;
            }
        }));
    }

    fn sync_monique_auth_poll(&mut self, cx: &mut Context<Self>) {
        let should_poll =
            self.monique_surface_visible() && self.monique_view.read(cx).has_active_login();
        if !should_poll {
            self._monique_auth_poll_task = None;
            return;
        }
        if self._monique_auth_poll_task.is_some() {
            return;
        }
        self._monique_auth_poll_task = Some(cx.spawn(async move |this, cx: &mut AsyncApp| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(2))
                .await;
            let keep = this
                .update(cx, |workspace, cx| {
                    if workspace.monique_surface_visible()
                        && workspace.monique_view.read(cx).has_active_login()
                    {
                        workspace.refresh_monique_accounts(cx);
                        true
                    } else {
                        workspace._monique_auth_poll_task = None;
                        false
                    }
                })
                .unwrap_or(false);
            if !keep {
                break;
            }
        }));
    }

    pub(super) fn refresh_monique(&mut self, cx: &mut Context<Self>) {
        if self.monique_view.read(cx).chat_busy() {
            return;
        }
        self.refresh_monique_snapshot(cx, true);
    }

    fn refresh_monique_snapshot(&mut self, cx: &mut Context<Self>, show_loading: bool) {
        let Some(config) = self.effective_monique_config() else {
            return;
        };
        if show_loading {
            self.monique_view.update(cx, |view, cx| {
                view.set_loading(true);
                cx.notify();
            });
        }
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let status = monique_client::status(&config)?;
                    let processes = monique_client::processes(&config)?;
                    let history = monique_client::chat_history(&config)?;
                    let accounts = monique_client::agent_accounts(&config)?;
                    Ok::<_, shelldeck_core::ShellDeckError>((status, processes, history, accounts))
                })
                .await;
            let _ = this.update(cx, |workspace, cx| match result {
                Ok((status, processes, history, accounts)) => {
                    workspace.monique_status = Some(status.clone());
                    workspace.monique_processes = Some(processes.clone());
                    workspace.monique_view.update(cx, |view, cx| {
                        view.set_snapshot(status, processes, history);
                        view.set_agent_accounts(accounts, cx);
                        cx.notify();
                    });
                    workspace.sync_monique_auth_poll(cx);
                }
                Err(error) => workspace.monique_view.update(cx, |view, cx| {
                    view.set_error(crate::i18n::api_error_message(&error));
                    cx.notify();
                }),
            });
        })
        .detach();
    }

    fn refresh_monique_accounts(&mut self, cx: &mut Context<Self>) {
        let Some(config) = self.effective_monique_config() else {
            return;
        };
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { monique_client::agent_accounts(&config) })
                .await;
            let _ = this.update(cx, |workspace, cx| match result {
                Ok(accounts) => {
                    workspace.monique_view.update(cx, |view, cx| {
                        view.set_agent_accounts(accounts, cx);
                        cx.notify();
                    });
                    workspace.sync_monique_auth_poll(cx);
                }
                Err(error) => workspace.monique_view.update(cx, |view, cx| {
                    view.set_account_error(crate::i18n::api_error_message(&error));
                    cx.notify();
                }),
            });
        })
        .detach();
    }

    pub(super) fn handle_monique_event(&mut self, event: MoniqueViewEvent, cx: &mut Context<Self>) {
        if !self.can_access_mode(AppMode::Dev) {
            return;
        }
        match event {
            MoniqueViewEvent::Refresh => self.refresh_monique(cx),
            MoniqueViewEvent::Send(message) => self.monique_chat(message, cx),
            MoniqueViewEvent::ResolveAction {
                action_id,
                approved,
            } => self.resolve_monique_action(action_id, approved, cx),
            MoniqueViewEvent::NewChat => self.new_monique_chat(cx),
            MoniqueViewEvent::AgentAction(action) => self.mutate_monique_accounts(action, cx),
            MoniqueViewEvent::OpenAuthorization(url) => {
                if let Err(error) = shelldeck_core::config::cloud_account::open_in_browser(&url) {
                    self.monique_view.update(cx, |view, cx| {
                        view.set_error(crate::i18n::api_error_message(&error));
                        cx.notify();
                    });
                }
            }
        }
    }

    fn mutate_monique_accounts(
        &mut self,
        action: shelldeck_core::config::monique::MoniqueAgentAuthAction,
        cx: &mut Context<Self>,
    ) {
        let Some(config) = self.effective_monique_config() else {
            return;
        };
        self.monique_view.update(cx, |view, cx| {
            view.set_account_loading(true);
            cx.notify();
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { monique_client::mutate_agent_accounts(&config, &action) })
                .await;
            let _ = this.update(cx, |workspace, cx| match result {
                Ok(accounts) => {
                    workspace.monique_view.update(cx, |view, cx| {
                        view.set_agent_accounts(accounts, cx);
                        cx.notify();
                    });
                    workspace.sync_monique_auth_poll(cx);
                }
                Err(error) => workspace.monique_view.update(cx, |view, cx| {
                    view.set_account_error(crate::i18n::api_error_message(&error));
                    cx.notify();
                }),
            });
        })
        .detach();
    }

    fn monique_chat(&mut self, message: String, cx: &mut Context<Self>) {
        let Some(config) = self.effective_monique_config() else {
            return;
        };
        self.monique_view.update(cx, |view, cx| {
            view.set_loading(true);
            cx.notify();
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { monique_client::chat(&config, &message) })
                .await;
            let _ = this.update(cx, |workspace, cx| match result {
                Ok(response) => workspace.monique_view.update(cx, |view, cx| {
                    view.apply_response(response);
                    cx.notify();
                }),
                Err(error) => workspace.monique_view.update(cx, |view, cx| {
                    view.set_error(crate::i18n::api_error_message(&error));
                    cx.notify();
                }),
            });
        })
        .detach();
    }

    fn resolve_monique_action(
        &mut self,
        action_id: String,
        approved: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(config) = self.effective_monique_config() else {
            return;
        };
        self.monique_view.update(cx, |view, cx| {
            view.set_loading(true);
            cx.notify();
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { monique_client::resolve_action(&config, &action_id, approved) })
                .await;
            let _ = this.update(cx, |workspace, cx| match result {
                Ok(response) => {
                    workspace.monique_view.update(cx, |view, cx| {
                        view.apply_response(response);
                        cx.notify();
                    });
                    workspace.refresh_monique(cx);
                }
                Err(error) => workspace.monique_view.update(cx, |view, cx| {
                    view.set_error(crate::i18n::api_error_message(&error));
                    cx.notify();
                }),
            });
        })
        .detach();
    }

    fn new_monique_chat(&mut self, cx: &mut Context<Self>) {
        let Some(config) = self.effective_monique_config() else {
            return;
        };
        self.monique_view.update(cx, |view, cx| {
            view.set_loading(true);
            cx.notify();
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { monique_client::new_chat(&config) })
                .await;
            let _ = this.update(cx, |workspace, cx| match result {
                Ok(_) => workspace.monique_view.update(cx, |view, cx| {
                    view.clear_chat();
                    cx.notify();
                }),
                Err(error) => workspace.monique_view.update(cx, |view, cx| {
                    view.set_error(crate::i18n::api_error_message(&error));
                    cx.notify();
                }),
            });
        })
        .detach();
    }
}
