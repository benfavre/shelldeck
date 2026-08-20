use super::*;

impl Workspace {
    /// The effective Jean config: a local `[jeanclaude]` override wins, else the
    /// server-delivered config (super-admin only). `None` = feature unavailable.
    ///
    /// Delegates to `JeanConfig::resolve_effective` (the pure fn under test in
    /// SDTEST-1054).
    pub(super) fn effective_jean_config(&self) -> Option<JeanConfig> {
        if !self.signed_in() {
            return None;
        }
        let server = self
            .site_directory
            .as_ref()
            .and_then(|s| s.jeanclaude.as_ref());
        JeanConfig::resolve_effective(self.app_config.jeanclaude.as_ref(), server)
    }

    pub fn has_jean(&self) -> bool {
        self.effective_jean_config().is_some()
    }

    /// Whether a Jean surface is currently on screen (so polling is worthwhile).
    fn jean_surface_visible(&self) -> bool {
        self.should_poll(super::polling::PolledSurface::Jean)
    }

    /// Reflect Jean availability into the sidebar nav (Dev mode only).
    pub(super) fn update_jean_availability(&mut self, cx: &mut Context<Self>) {
        let show = self.has_jean() && self.effective_mode() == AppMode::Dev;
        self.sidebar.update(cx, |s, cx| {
            s.set_jean_available(show);
            cx.notify();
        });
    }

    pub(super) fn sync_jean_poll(&mut self, cx: &mut Context<Self>) {
        if self.jean_surface_visible() {
            // Refresh immediately when a surface becomes visible.
            self.refresh_jean_state(cx);
            if self._jean_poll_task.is_none() {
                let task = cx.spawn(async move |this, cx: &mut AsyncApp| loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(10))
                        .await;
                    let keep = this
                        .update(cx, |ws, cx| {
                            if ws.jean_surface_visible() {
                                ws.refresh_jean_state(cx);
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
                self._jean_poll_task = Some(task);
            }
        } else {
            self._jean_poll_task = None;
        }
    }

    pub(super) fn refresh_jean_state(&mut self, cx: &mut Context<Self>) {
        let Some(cfg) = self.effective_jean_config() else {
            return;
        };
        self.jean_view.update(cx, |v, cx| {
            v.set_loading(true);
            cx.notify();
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { jeanclaude::get_state(&cfg) })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(state) => {
                    ws.jean_state = Some(state.clone());
                    ws.jean_view.update(cx, |v, cx| {
                        v.set_state(state);
                        cx.notify();
                    });
                    ws.push_jean_brief_to_support(cx);
                }
                Err(e) => {
                    ws.jean_view.update(cx, |v, cx| {
                        v.set_error(crate::i18n::api_error_message(&e));
                        cx.notify();
                    });
                }
            });
        })
        .detach();
    }

    /// Feed the Support-mode Jean strip from the cached state.
    pub(super) fn push_jean_brief_to_support(&mut self, cx: &mut Context<Self>) {
        let available = self.has_jean();
        let (pending, active) = self
            .jean_state
            .as_ref()
            .map(|s| {
                let pending: Vec<(String, String)> = s
                    .pending
                    .iter()
                    .map(|p| (p.thread_ts.clone(), p.prompt.clone()))
                    .collect();
                let active = s
                    .tickets
                    .iter()
                    .filter(|t| t.is_running() || t.is_queued())
                    .count();
                (pending, active)
            })
            .unwrap_or_default();
        self.support.update(cx, |v, cx| {
            v.set_jean_brief(available, pending, active);
            cx.notify();
        });
    }

    pub(super) fn handle_jean_event(&mut self, event: JeanViewEvent, cx: &mut Context<Self>) {
        if !self.can_access_mode(AppMode::Dev) {
            return;
        }
        use jeanclaude as j;
        match event {
            JeanViewEvent::Refresh => self.refresh_jean_state(cx),
            JeanViewEvent::SetPaused(p) => self.jean_action(cx, move |c| j::set_paused(&c, p)),
            JeanViewEvent::SetConcurrency(n) => {
                self.jean_action(cx, move |c| j::set_concurrency(&c, n))
            }
            JeanViewEvent::Say(text) => self.jean_say(text, cx),
            JeanViewEvent::Confirm(t) => self.jean_action(cx, move |c| j::confirm(&c, &t)),
            JeanViewEvent::Reject(t) => self.jean_action(cx, move |c| j::reject(&c, &t)),
            JeanViewEvent::Cancel(id) => self.jean_action(cx, move |c| j::cancel(&c, &id)),
            JeanViewEvent::Force(id) => self.jean_action(cx, move |c| j::force_ticket(&c, &id)),
            JeanViewEvent::SelectTicket(id) => self.jean_select_ticket(id, cx),
            JeanViewEvent::LoadHistory { q, status } => self.jean_load_history(q, status, cx),
            JeanViewEvent::LoadTargets => self.jean_load_targets(cx),
            JeanViewEvent::LoadMemory => self.jean_load_memory(cx),
            JeanViewEvent::AddTarget {
                domain,
                ssh_host,
                note,
            } => self.jean_action(cx, move |c| j::add_target(&c, &domain, &ssh_host, &note)),
            JeanViewEvent::RemoveTarget(d) => {
                self.jean_action(cx, move |c| j::remove_target(&c, &d))
            }
            JeanViewEvent::AddMemory { kind, match_, text } => {
                self.jean_action(cx, move |c| j::add_memory(&c, &kind, &match_, &[], &text))
            }
            JeanViewEvent::RemoveMemory(id) => {
                self.jean_action(cx, move |c| j::remove_memory(&c, &id))
            }
        }
    }

    /// Run a Jean write action on the background executor, then refresh state.
    pub(super) fn jean_action<F>(&mut self, cx: &mut Context<Self>, f: F)
    where
        F: FnOnce(JeanConfig) -> shelldeck_core::Result<()> + Send + 'static,
    {
        let Some(cfg) = self.effective_jean_config() else {
            return;
        };
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx.background_executor().spawn(async move { f(cfg) }).await;
            let _ = this.update(cx, |ws, cx| {
                if let Err(e) = result {
                    ws.show_toast(
                        t!(
                            "toast.jean.error",
                            error = crate::i18n::api_error_message(&e)
                        )
                        .to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                }
                ws.refresh_jean_state(cx);
            });
        })
        .detach();
    }

    pub(super) fn jean_say(&mut self, text: String, cx: &mut Context<Self>) {
        let Some(cfg) = self.effective_jean_config() else {
            return;
        };
        let activity_text = text.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { jeanclaude::say(&cfg, &text) })
                .await;
            let _ = this.update(cx, |ws, cx| {
                match result {
                    Ok(_) => {
                        ws.add_activity_entry(
                            ActivityEntry::new(
                                ActivityKind::Jean,
                                t!("activity.jean.sent").to_string(),
                            )
                            .with_detail(activity_text.clone())
                            .with_action(ActivityAction::OpenJean),
                            cx,
                        );
                        ws.show_toast(t!("toast.jean.sent").to_string(), ToastLevel::Success, cx)
                    }
                    Err(e) => ws.show_toast(
                        t!(
                            "toast.jean.error",
                            error = crate::i18n::api_error_message(&e)
                        )
                        .to_string(),
                        ToastLevel::Error,
                        cx,
                    ),
                }
                ws.refresh_jean_state(cx);
            });
        })
        .detach();
    }

    fn jean_select_ticket(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(cfg) = self.effective_jean_config() else {
            return;
        };
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { jeanclaude::get_ticket(&cfg, &id) })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(t) => ws.jean_view.update(cx, |v, cx| {
                    v.set_detail(t);
                    cx.notify();
                }),
                Err(e) => ws.jean_view.update(cx, |v, cx| {
                    v.set_error(crate::i18n::api_error_message(&e));
                    cx.notify();
                }),
            });
        })
        .detach();
    }

    fn jean_load_history(&mut self, q: String, status: String, cx: &mut Context<Self>) {
        let Some(cfg) = self.effective_jean_config() else {
            return;
        };
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { jeanclaude::get_history(&cfg, &q, &status, 60) })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(h) => ws.jean_view.update(cx, |v, cx| {
                    v.set_history(h);
                    cx.notify();
                }),
                Err(e) => ws.jean_view.update(cx, |v, cx| {
                    v.set_error(crate::i18n::api_error_message(&e));
                    cx.notify();
                }),
            });
        })
        .detach();
    }

    fn jean_load_targets(&mut self, cx: &mut Context<Self>) {
        let Some(cfg) = self.effective_jean_config() else {
            return;
        };
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { jeanclaude::get_targets(&cfg) })
                .await;
            let _ = this.update(cx, |ws, cx| {
                if let Ok(t) = result {
                    ws.jean_view.update(cx, |v, cx| {
                        v.set_targets(t);
                        cx.notify();
                    });
                }
            });
        })
        .detach();
    }

    fn jean_load_memory(&mut self, cx: &mut Context<Self>) {
        let Some(cfg) = self.effective_jean_config() else {
            return;
        };
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { jeanclaude::get_memory(&cfg) })
                .await;
            let _ = this.update(cx, |ws, cx| {
                if let Ok(m) = result {
                    ws.jean_view.update(cx, |v, cx| {
                        v.set_memory(m);
                        cx.notify();
                    });
                }
            });
        })
        .detach();
    }
}
