use gpui::*;
use shelldeck_core::config::bext_cloud::{self, BextCloudConfig};
use shelldeck_core::config::bext_instance;
use shelldeck_core::config::cloud_account::{self, AppMode};
use uuid::Uuid;

use crate::bext_cloud_view::BextViewEvent;
use crate::t;
use crate::toast::ToastLevel;

use super::{ActiveView, Workspace};

impl Workspace {
    fn bext_visible(&self) -> bool {
        self.should_poll(super::polling::PolledSurface::Bext)
    }

    /// Open the bext Cloud view (palette / sidebar).
    pub fn open_bext_cloud(&mut self, cx: &mut Context<Self>) {
        if !self.enter_dev_mode(cx) {
            return;
        }
        self.active_view = ActiveView::BextCloud;
        self.on_active_view_changed(cx);
        cx.notify();
    }

    /// Palette: open the view and immediately start the cloud connect flow.
    pub fn connect_bext_cloud_action(&mut self, cx: &mut Context<Self>) {
        if !self.enter_dev_mode(cx) {
            return;
        }
        self.open_bext_cloud(cx);
        self.connect_bext(cx);
    }

    /// Per-connection "Gérer bext": open the Instance tab. v1 targets the local
    /// loopback SDK (remote reach via SSH tunnel is a follow-up).
    pub fn manage_bext_for_connection(&mut self, conn_id: Uuid, cx: &mut Context<Self>) {
        if !self.enter_dev_mode(cx) {
            return;
        }
        let app_id = self
            .connections
            .iter()
            .find(|c| c.id == conn_id)
            .map(|c| c.alias.clone())
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| "default".to_string());
        self.active_view = ActiveView::BextCloud;
        let base = "http://127.0.0.1".to_string();
        self.bext_view.update(cx, |v, cx| {
            v.open_instance(base.clone(), app_id.clone(), cx);
            cx.notify();
        });
        self.show_toast(
            t!("toast.bext.local_instance").to_string(),
            ToastLevel::Info,
            cx,
        );
        self.refresh_bext_instance(base, app_id, cx);
        cx.notify();
    }

    pub(super) fn sync_bext_poll(&mut self, cx: &mut Context<Self>) {
        if self.bext_visible() {
            self.refresh_bext_cloud(cx);
            if self._bext_poll.is_none() {
                let task = cx.spawn(async move |this, cx: &mut AsyncApp| loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(15))
                        .await;
                    let keep = this
                        .update(cx, |ws, cx| {
                            if ws.bext_visible() {
                                ws.refresh_bext_cloud(cx);
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
                self._bext_poll = Some(task);
            }
        } else {
            self._bext_poll = None;
        }
    }

    fn refresh_bext_cloud(&mut self, cx: &mut Context<Self>) {
        let cfg = self.app_config.bext_cloud.clone();
        if !cfg.is_connected() {
            self.bext_view.update(cx, |v, cx| {
                v.set_connection(false, None);
                cx.notify();
            });
            return;
        }
        self.bext_view.update(cx, |v, cx| {
            v.set_loading(true);
            cx.notify();
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let bundle = cx
                .background_executor()
                .spawn(async move {
                    // Fan out whoami / sites / dashboard onto three OS
                    // threads — the bext_cloud client is reqwest-blocking,
                    // so the previous serial chain cost ~3× round-trip.
                    // Instances stays serial after whoami since it's only
                    // fetched for super-admin.
                    let cfg_w = cfg.clone();
                    let cfg_s = cfg.clone();
                    let cfg_d = cfg.clone();
                    let who_h = std::thread::spawn(move || bext_cloud::whoami(&cfg_w));
                    let sites_h = std::thread::spawn(move || bext_cloud::list_sites(&cfg_s));
                    let dash_h = std::thread::spawn(move || bext_cloud::dashboard(&cfg_d));
                    let who = who_h.join().expect("bext whoami thread panicked");
                    let sites = sites_h.join().expect("bext list_sites thread panicked");
                    let dash = dash_h.join().expect("bext dashboard thread panicked");
                    let is_super = who.as_ref().map(|u| u.is_super_admin).unwrap_or(false);
                    let instances = if is_super {
                        bext_cloud::list_instances(&cfg).ok()
                    } else {
                        None
                    };
                    (who, sites, dash, instances)
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                let (who, sites, dash, instances) = bundle;
                match who {
                    Ok(u) => {
                        ws.bext_user = Some(u.clone());
                        ws.bext_view.update(cx, |v, cx| {
                            v.set_connection(true, Some(u));
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        ws.bext_view.update(cx, |v, cx| {
                            v.set_error(cloud_account::user_message(&e));
                            cx.notify();
                        });
                    }
                }
                if let Ok(s) = sites {
                    ws.bext_view.update(cx, |v, cx| {
                        v.set_sites(s);
                        cx.notify();
                    });
                }
                if let Ok(d) = dash {
                    ws.bext_view.update(cx, |v, cx| {
                        v.set_stats(d.stats);
                        cx.notify();
                    });
                }
                if let Some(insts) = instances {
                    ws.bext_view.update(cx, |v, cx| {
                        v.set_instances(insts.instances);
                        cx.notify();
                    });
                }
            });
        })
        .detach();
    }

    fn connect_bext(&mut self, cx: &mut Context<Self>) {
        let base = {
            let b = self.app_config.bext_cloud.base_url.trim().to_string();
            if b.is_empty() {
                "https://cloud.bext.dev".to_string()
            } else {
                b
            }
        };
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
        let url = bext_cloud::cli_login_url(&base, port);
        if let Err(e) = cloud_account::open_in_browser(&url) {
            self.show_toast(
                t!(
                    "toast.open_browser_failed",
                    error = cloud_account::user_message(&e)
                )
                .to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }
        self.show_toast(
            t!("toast.bext.connect_waiting").to_string(),
            ToastLevel::Info,
            cx,
        );
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    bext_cloud::browser_connect_listen(
                        listener,
                        std::time::Duration::from_secs(180),
                    )
                })
                .await;
            let _ = this.update(cx, |ws, cx| match outcome {
                Ok(conn) => {
                    ws.app_config.bext_cloud.token = conn.token;
                    ws.app_config.bext_cloud.email = conn.email;
                    ws.app_config.bext_cloud.name = conn.name;
                    if let Err(e) = ws.app_config.save() {
                        tracing::error!("Failed to save bext_cloud config: {}", e);
                    }
                    ws.show_toast(
                        t!(
                            "toast.bext.connected",
                            email = ws.app_config.bext_cloud.email.as_str()
                        )
                        .to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                    ws.refresh_bext_cloud(cx);
                }
                Err(e) => ws.show_toast(
                    t!(
                        "toast.bext.connect_failed",
                        error = cloud_account::user_message(&e)
                    )
                    .to_string(),
                    ToastLevel::Error,
                    cx,
                ),
            });
        })
        .detach();
    }

    fn disconnect_bext(&mut self, cx: &mut Context<Self>) {
        self.app_config.bext_cloud.token = String::new();
        self.app_config.bext_cloud.email = String::new();
        self.app_config.bext_cloud.name = String::new();
        if let Err(e) = self.app_config.save() {
            tracing::error!("Failed to save bext_cloud config: {}", e);
        }
        self.bext_user = None;
        self.bext_view.update(cx, |v, cx| {
            v.set_connection(false, None);
            cx.notify();
        });
        self.show_toast(
            t!("toast.bext.disconnected").to_string(),
            ToastLevel::Info,
            cx,
        );
        cx.notify();
    }

    fn bext_cloud_action<F>(&mut self, cx: &mut Context<Self>, f: F)
    where
        F: FnOnce(BextCloudConfig) -> shelldeck_core::Result<()> + Send + 'static,
    {
        let cfg = self.app_config.bext_cloud.clone();
        if !cfg.is_connected() {
            return;
        }
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let r = cx.background_executor().spawn(async move { f(cfg) }).await;
            let _ = this.update(cx, |ws, cx| {
                match r {
                    Ok(_) => ws.show_toast(
                        t!("toast.bext.action_ok").to_string(),
                        ToastLevel::Success,
                        cx,
                    ),
                    Err(e) => ws.show_toast(
                        t!("toast.bext.error", error = cloud_account::user_message(&e)).to_string(),
                        ToastLevel::Error,
                        cx,
                    ),
                }
                ws.refresh_bext_cloud(cx);
            });
        })
        .detach();
    }

    fn refresh_bext_instance(&mut self, base: String, app_id: String, cx: &mut Context<Self>) {
        let (b2, a2) = (base.clone(), app_id.clone());
        self.bext_view.update(cx, |v, cx| {
            v.set_loading(true);
            cx.notify();
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let r = cx
                .background_executor()
                .spawn(async move {
                    let inst = bext_instance::BextInstance::new(base, app_id);
                    bext_instance::list_sites(&inst)
                })
                .await;
            let _ = this.update(cx, |ws, cx| match r {
                Ok(sites) => ws.bext_view.update(cx, |v, cx| {
                    v.set_instance_sites(sites.sites, b2.clone(), a2.clone(), cx);
                    cx.notify();
                }),
                Err(e) => ws.bext_view.update(cx, |v, cx| {
                    v.set_error(cloud_account::user_message(&e));
                    cx.notify();
                }),
            });
        })
        .detach();
    }

    fn bext_instance_action<F>(
        &mut self,
        base: String,
        app_id: String,
        cx: &mut Context<Self>,
        f: F,
    ) where
        F: FnOnce(&bext_instance::BextInstance) -> shelldeck_core::Result<()> + Send + 'static,
    {
        let (b2, a2) = (base.clone(), app_id.clone());
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let r = cx
                .background_executor()
                .spawn(async move {
                    let inst = bext_instance::BextInstance::new(base, app_id);
                    f(&inst)
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                match r {
                    Ok(_) => ws.show_toast(
                        t!("toast.bext.instance_action_ok").to_string(),
                        ToastLevel::Success,
                        cx,
                    ),
                    Err(e) => ws.show_toast(
                        t!(
                            "toast.bext.instance_error",
                            error = cloud_account::user_message(&e)
                        )
                        .to_string(),
                        ToastLevel::Error,
                        cx,
                    ),
                }
                ws.refresh_bext_instance(b2.clone(), a2.clone(), cx);
            });
        })
        .detach();
    }

    pub(super) fn handle_bext_event(&mut self, event: BextViewEvent, cx: &mut Context<Self>) {
        if !self.can_access_mode(AppMode::Dev) {
            return;
        }
        match event {
            BextViewEvent::Connect => self.connect_bext(cx),
            BextViewEvent::Disconnect => self.disconnect_bext(cx),
            BextViewEvent::RefreshCloud => self.refresh_bext_cloud(cx),
            BextViewEvent::CreateSite { name, title } => {
                let t = if title.trim().is_empty() {
                    None
                } else {
                    Some(title)
                };
                self.bext_cloud_action(cx, move |cfg| {
                    bext_cloud::create_site(&cfg, &name, t.as_deref()).map(|_| ())
                });
            }
            BextViewEvent::SiteAction { slug, action } => {
                self.bext_cloud_action(cx, move |cfg| {
                    bext_cloud::site_action(&cfg, &slug, &action, None).map(|_| ())
                });
            }
            BextViewEvent::OpenSite(domain) => {
                let url = if domain.starts_with("http") {
                    domain
                } else {
                    format!("https://{}", domain)
                };
                if let Err(e) = cloud_account::open_in_browser(&url) {
                    self.show_toast(
                        t!(
                            "toast.open_failed_generic",
                            error = cloud_account::user_message(&e)
                        )
                        .to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                }
            }
            BextViewEvent::RefreshInstance { base, app_id } => {
                self.refresh_bext_instance(base, app_id, cx)
            }
            BextViewEvent::InstanceCreate {
                base,
                app_id,
                slug,
                title,
            } => {
                let t = if title.trim().is_empty() {
                    None
                } else {
                    Some(title)
                };
                self.bext_instance_action(base, app_id, cx, move |inst| {
                    bext_instance::create_site(inst, &slug, t.as_deref(), None, None).map(|_| ())
                });
            }
            BextViewEvent::InstanceGoLive {
                base,
                app_id,
                slug,
                domain,
            } => {
                self.bext_instance_action(base, app_id, cx, move |inst| {
                    bext_instance::go_live(inst, &slug, &domain).map(|_| ())
                });
            }
            BextViewEvent::InstanceDestroy { base, app_id, slug } => {
                self.bext_instance_action(base, app_id, cx, move |inst| {
                    bext_instance::destroy_site(inst, &slug).map(|_| ())
                });
            }
        }
    }
}
