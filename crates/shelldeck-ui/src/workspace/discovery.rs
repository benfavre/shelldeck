use gpui::prelude::*;
use gpui::*;
use shelldeck_core::models::connection::Connection;
use shelldeck_core::models::managed_site::ManagedSite;
use shelldeck_ssh::client::SshClient;

use crate::server_sync_view::PanelSide;
use crate::sites_view::SitesEvent;
use crate::toast::ToastLevel;

use super::{ActiveView, Workspace};

impl Workspace {
    pub(super) fn handle_sites_event(&mut self, event: &SitesEvent, cx: &mut Context<Self>) {
        match event {
            SitesEvent::ScanServer(conn_id) => {
                let conn_id = *conn_id;
                if let Some(conn) = self.connections.iter().find(|c| c.id == conn_id).cloned() {
                    self.sites.update(cx, |view, _| {
                        view.scans_pending += 1;
                    });
                    self.discover_for_sites(conn, cx);
                }
            }
            SitesEvent::ScanAllServers => {
                let conns: Vec<Connection> = self.connections.clone();
                let count = conns.len() as u32;
                self.sites.update(cx, |view, _| {
                    view.scans_pending += count;
                });
                for conn in conns {
                    self.discover_for_sites(conn, cx);
                }
            }
            SitesEvent::RemoveSite(id) => {
                let _ = self.store.remove_managed_site(*id);
                self.sites.update(cx, |view, _| {
                    view.set_sites(self.store.managed_sites.clone());
                });
                cx.notify();
            }
            SitesEvent::ToggleFavorite(id) => {
                let id = *id;
                if let Some(site) = self.store.managed_sites.iter_mut().find(|s| s.id == id) {
                    site.favorite = !site.favorite;
                }
                let _ = self.store.save();
                self.sites.update(cx, |view, _| {
                    view.set_sites(self.store.managed_sites.clone());
                });
                cx.notify();
            }
            SitesEvent::UpdateTags(id, tags) => {
                let id = *id;
                let tags = tags.clone();
                if let Some(site) = self.store.managed_sites.iter_mut().find(|s| s.id == id) {
                    site.tags = tags;
                }
                let _ = self.store.save();
                self.sites.update(cx, |view, _| {
                    view.set_sites(self.store.managed_sites.clone());
                });
                cx.notify();
            }
            SitesEvent::OpenInBrowser(url) => {
                let _ = open::that(url);
            }
            SitesEvent::SshToServer(conn_id) => {
                let conn_id = *conn_id;
                if let Some(conn) = self.connections.iter().find(|c| c.id == conn_id).cloned() {
                    self.connect_ssh(conn, cx);
                }
            }
            SitesEvent::AddToSync(site_id) => {
                let _ = site_id;
                self.set_active_view(ActiveView::ServerSync);
                cx.notify();
            }
            SitesEvent::CheckSiteStatus(site_id) => {
                let site_id = *site_id;
                if let Some(site) = self.store.managed_sites.iter().find(|s| s.id == site_id) {
                    let conn_id = site.connection_id;
                    let port = site.port();
                    if let Some(conn) = self.connections.iter().find(|c| c.id == conn_id).cloned() {
                        if let Some(port) = port {
                            let sites_handle = self.sites.downgrade();
                            let check_cmd = format!(
                                "ss -tlnp 2>/dev/null | grep -q ':{} ' && echo ONLINE || echo OFFLINE",
                                port
                            );

                            let (done_tx, done_rx) = std::sync::mpsc::channel::<(bool, String)>();

                            let spawn_result = std::thread::Builder::new()
                                .name("site-status-check".to_string())
                                .spawn(move || {
                                    let rt = match tokio::runtime::Builder::new_current_thread()
                                        .enable_all()
                                        .build()
                                    {
                                        Ok(rt) => rt,
                                        Err(e) => {
                                            let _ = done_tx.send((
                                                false,
                                                format!("async runtime error: {}", e),
                                            ));
                                            return;
                                        }
                                    };
                                    rt.block_on(async move {
                                        let client = SshClient::new();
                                        match client.connect(&conn).await {
                                            Ok(session) => match session.exec(&check_cmd).await {
                                                Ok(result) => {
                                                    let output =
                                                        String::from_utf8_lossy(&result.stdout)
                                                            .trim()
                                                            .to_string();
                                                    let online = output.contains("ONLINE");
                                                    let _ = done_tx.send((online, String::new()));
                                                }
                                                Err(e) => {
                                                    let _ = done_tx.send((false, e.to_string()));
                                                }
                                            },
                                            Err(e) => {
                                                let _ = done_tx.send((false, e.to_string()));
                                            }
                                        }
                                    });
                                });
                            if let Err(e) = spawn_result {
                                tracing::error!("Failed to spawn status-check thread: {}", e);
                                self.show_toast(
                                    format!("Failed to check site status: {}", e),
                                    ToastLevel::Error,
                                    cx,
                                );
                                return;
                            }

                            cx.spawn(async move |_ws, cx: &mut AsyncApp| {
                                loop {
                                    cx.background_executor()
                                        .timer(std::time::Duration::from_millis(50))
                                        .await;
                                    if let Ok((online, err_msg)) = done_rx.try_recv() {
                                        let _ = sites_handle.update(cx, |view, cx| {
                                            if let Some(site) = view.sites.iter_mut().find(|s| s.id == site_id) {
                                                site.last_checked = Some(chrono::Utc::now());
                                                if err_msg.is_empty() {
                                                    site.status = if online {
                                                        shelldeck_core::models::managed_site::SiteStatus::Online
                                                    } else {
                                                        shelldeck_core::models::managed_site::SiteStatus::Offline
                                                    };
                                                } else {
                                                    site.status = shelldeck_core::models::managed_site::SiteStatus::Error(err_msg);
                                                }
                                            }
                                            cx.notify();
                                        });
                                        break;
                                    }
                                }
                            }).detach();
                        }
                    }
                }
            }
            SitesEvent::ClearAllSites => {
                self.store.managed_sites.clear();
                let _ = self.store.save();
                self.sites.update(cx, |view, _| {
                    view.set_sites(Vec::new());
                });
                cx.notify();
            }
            SitesEvent::RefreshSites => {
                self.sites.update(cx, |view, _| {
                    view.set_sites(self.store.managed_sites.clone());
                });
                cx.notify();
            }
        }
    }

    pub(super) fn discover_local_services(&mut self, panel: PanelSide, cx: &mut Context<Self>) {
        use shelldeck_core::models::discovery;

        let (tx, rx) = std::sync::mpsc::channel::<(
            Vec<shelldeck_core::models::DiscoveredSite>,
            Vec<shelldeck_core::models::DiscoveredDatabase>,
        )>();

        let spawn_result = std::thread::Builder::new()
            .name("sync-discover-local".to_string())
            .spawn(move || {
                let nginx_output = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(discovery::nginx_discover_command())
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                    .unwrap_or_default();
                let sites = discovery::parse_nginx_configs(&nginx_output);

                let mysql_output = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(discovery::mysql_discover_command(""))
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                    .unwrap_or_default();
                let mysql_dbs = discovery::parse_mysql_discovery(&mysql_output);

                let pg_output = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(discovery::pg_discover_command("-U postgres"))
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                    .unwrap_or_default();
                let pg_dbs = discovery::parse_pg_discovery(&pg_output);

                let mut all_dbs = mysql_dbs;
                all_dbs.extend(pg_dbs);

                let _ = tx.send((sites, all_dbs));
            });
        if let Err(e) = spawn_result {
            tracing::error!("Failed to spawn local discover thread: {}", e);
            self.server_sync.update(cx, |view, cx| {
                view.panel_state_mut(panel).discovery_loading = false;
                cx.notify();
            });
            self.show_toast(
                format!("Failed to discover local services: {}", e),
                ToastLevel::Error,
                cx,
            );
            return;
        }

        let sync_handle = self.server_sync.downgrade();
        cx.spawn(async move |_this, cx: &mut AsyncApp| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(50))
                .await;

            if let Ok((sites, dbs)) = rx.try_recv() {
                let _ = sync_handle.update(cx, |view, cx| {
                    view.set_discovered_sites(panel, sites);
                    view.set_discovered_databases(panel, dbs);
                    view.panel_state_mut(panel).discovery_loading = false;
                    cx.notify();
                });
                break;
            }
        })
        .detach();
    }

    pub(super) fn discover_remote_services(
        &mut self,
        connection: Connection,
        panel: PanelSide,
        cx: &mut Context<Self>,
    ) {
        use shelldeck_core::models::discovery;

        let disc_conn_id = connection.id;
        let disc_conn_name = connection.display_name().to_string();

        let nginx_cmd = discovery::nginx_discover_command().to_string();
        let mysql_cmd = discovery::mysql_discover_command("");
        let pg_cmd = discovery::pg_discover_command("-U postgres");

        let (stream_tx, stream_rx) = std::sync::mpsc::channel::<(String, String)>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<bool>();

        let thread_disc_conn_name = disc_conn_name.clone();
        let spawn_result = std::thread::Builder::new()
            .name("sync-discover".to_string())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = stream_tx
                            .send(("error".to_string(), format!("Error: async runtime: {}", e)));
                        let _ = done_tx.send(false);
                        return;
                    }
                };

                rt.block_on(async move {
                    let client = SshClient::new();
                    let session = match tokio::time::timeout(
                        std::time::Duration::from_secs(15),
                        client.connect(&connection),
                    )
                    .await
                    {
                        Ok(Ok(s)) => s,
                        Ok(Err(e)) => {
                            let _ = stream_tx.send(("error".to_string(), format!("Error: {}", e)));
                            let _ = done_tx.send(false);
                            return;
                        }
                        Err(_) => {
                            let _ = stream_tx.send((
                                "error".to_string(),
                                format!("Connection timed out for {}", thread_disc_conn_name),
                            ));
                            let _ = done_tx.send(false);
                            return;
                        }
                    };

                    let exec_timeout = std::time::Duration::from_secs(30);

                    // Discover nginx
                    match tokio::time::timeout(exec_timeout, session.exec(&nginx_cmd)).await {
                        Ok(Ok(result)) => {
                            let output = String::from_utf8_lossy(&result.stdout).to_string();
                            let _ = stream_tx.send(("nginx".to_string(), output));
                        }
                        Ok(Err(e)) => tracing::debug!(
                            "nginx discover exec error on {}: {}",
                            thread_disc_conn_name,
                            e
                        ),
                        Err(_) => {
                            tracing::warn!("nginx discover timed out on {}", thread_disc_conn_name)
                        }
                    }

                    // Discover MySQL
                    match tokio::time::timeout(exec_timeout, session.exec(&mysql_cmd)).await {
                        Ok(Ok(result)) => {
                            let output = String::from_utf8_lossy(&result.stdout).to_string();
                            let _ = stream_tx.send(("mysql".to_string(), output));
                        }
                        Ok(Err(e)) => tracing::debug!(
                            "mysql discover exec error on {}: {}",
                            thread_disc_conn_name,
                            e
                        ),
                        Err(_) => {
                            tracing::warn!("mysql discover timed out on {}", thread_disc_conn_name)
                        }
                    }

                    // Discover PostgreSQL
                    match tokio::time::timeout(exec_timeout, session.exec(&pg_cmd)).await {
                        Ok(Ok(result)) => {
                            let output = String::from_utf8_lossy(&result.stdout).to_string();
                            let _ = stream_tx.send(("pg".to_string(), output));
                        }
                        Ok(Err(e)) => tracing::debug!(
                            "pg discover exec error on {}: {}",
                            thread_disc_conn_name,
                            e
                        ),
                        Err(_) => {
                            tracing::warn!("pg discover timed out on {}", thread_disc_conn_name)
                        }
                    }

                    let _ = done_tx.send(true);
                });
            });
        if let Err(e) = spawn_result {
            tracing::error!("Failed to spawn discover thread: {}", e);
            self.server_sync.update(cx, |view, cx| {
                view.panel_state_mut(panel).discovery_loading = false;
                cx.notify();
            });
            self.show_toast(
                format!("Failed to discover remote services: {}", e),
                ToastLevel::Error,
                cx,
            );
            return;
        }

        let sync_handle = self.server_sync.downgrade();
        cx.spawn(async move |ws_handle, cx: &mut AsyncApp| {
            let mut auto_sites: Vec<ManagedSite> = Vec::new();
            let wall_clock_start = std::time::Instant::now();
            let wall_clock_limit = std::time::Duration::from_secs(90);

            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;

                while let Ok((kind, data)) = stream_rx.try_recv() {
                    match kind.as_str() {
                        "nginx" => {
                            let sites = discovery::parse_nginx_configs(&data);
                            for s in &sites {
                                auto_sites.push(ManagedSite::from_nginx(
                                    disc_conn_id,
                                    &disc_conn_name,
                                    s.clone(),
                                ));
                            }
                            let _ = sync_handle.update(cx, |view, cx| {
                                view.set_discovered_sites(panel, sites);
                                cx.notify();
                            });
                        }
                        "mysql" => {
                            let dbs = discovery::parse_mysql_discovery(&data);
                            for d in &dbs {
                                auto_sites.push(ManagedSite::from_database(
                                    disc_conn_id,
                                    &disc_conn_name,
                                    d.clone(),
                                ));
                            }
                            if !dbs.is_empty() {
                                let _ = sync_handle.update(cx, |view, cx| {
                                    let mut all =
                                        view.panel_state_mut(panel).discovered_databases.clone();
                                    all.extend(dbs);
                                    view.set_discovered_databases(panel, all);
                                    cx.notify();
                                });
                            }
                        }
                        "pg" => {
                            let dbs = discovery::parse_pg_discovery(&data);
                            for d in &dbs {
                                auto_sites.push(ManagedSite::from_database(
                                    disc_conn_id,
                                    &disc_conn_name,
                                    d.clone(),
                                ));
                            }
                            if !dbs.is_empty() {
                                let _ = sync_handle.update(cx, |view, cx| {
                                    let mut all =
                                        view.panel_state_mut(panel).discovered_databases.clone();
                                    all.extend(dbs);
                                    view.set_discovered_databases(panel, all);
                                    cx.notify();
                                });
                            }
                        }
                        _ => {}
                    }
                }

                if done_rx.try_recv().is_ok() {
                    let _ = sync_handle.update(cx, |view, cx| {
                        view.panel_state_mut(panel).discovery_loading = false;
                        cx.notify();
                    });
                    if !auto_sites.is_empty() {
                        let _ = ws_handle.update(cx, |ws, cx| {
                            let _ = ws.store.add_managed_sites_bulk(auto_sites);
                            ws.sites.update(cx, |view, _| {
                                view.set_sites(ws.store.managed_sites.clone());
                            });
                            cx.notify();
                        });
                    }
                    break;
                }

                // Wall-clock safety: abort if background thread is stuck
                if wall_clock_start.elapsed() > wall_clock_limit {
                    tracing::warn!(
                        "Sync discover poller timed out after 90s for {}",
                        disc_conn_name
                    );
                    let _ = sync_handle.update(cx, |view, cx| {
                        view.panel_state_mut(panel).discovery_loading = false;
                        cx.notify();
                    });
                    if !auto_sites.is_empty() {
                        let _ = ws_handle.update(cx, |ws, cx| {
                            let _ = ws.store.add_managed_sites_bulk(auto_sites);
                            ws.sites.update(cx, |view, _| {
                                view.set_sites(ws.store.managed_sites.clone());
                            });
                            cx.notify();
                        });
                    }
                    break;
                }
            }
        })
        .detach();
    }

    pub(super) fn discover_for_sites(&mut self, connection: Connection, cx: &mut Context<Self>) {
        use shelldeck_core::models::discovery;

        let conn_id = connection.id;
        let conn_name = connection.display_name().to_string();

        let nginx_cmd = discovery::nginx_discover_command().to_string();
        let mysql_cmd = discovery::mysql_discover_command("");
        let pg_cmd = discovery::pg_discover_command("-U postgres");

        let (stream_tx, stream_rx) = std::sync::mpsc::channel::<(String, String)>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<bool>();

        let thread_conn_name = conn_name.clone();
        let spawn_result = std::thread::Builder::new()
            .name("sites-discover".to_string())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        tracing::error!("Failed to create async runtime for sites discover: {}", e);
                        let _ = done_tx.send(false);
                        return;
                    }
                };

                rt.block_on(async move {
                    let client = SshClient::new();
                    let session = match tokio::time::timeout(
                        std::time::Duration::from_secs(15),
                        client.connect(&connection),
                    )
                    .await
                    {
                        Ok(Ok(s)) => s,
                        Ok(Err(e)) => {
                            tracing::warn!("Sites discover failed for {}: {}", thread_conn_name, e);
                            let _ = done_tx.send(false);
                            return;
                        }
                        Err(_) => {
                            tracing::warn!(
                                "Sites discover timed out connecting to {}",
                                thread_conn_name
                            );
                            let _ = done_tx.send(false);
                            return;
                        }
                    };

                    let exec_timeout = std::time::Duration::from_secs(30);

                    match tokio::time::timeout(exec_timeout, session.exec(&nginx_cmd)).await {
                        Ok(Ok(result)) => {
                            let output = String::from_utf8_lossy(&result.stdout).to_string();
                            let _ = stream_tx.send(("nginx".to_string(), output));
                        }
                        Ok(Err(e)) => tracing::debug!(
                            "nginx discover exec error on {}: {}",
                            thread_conn_name,
                            e
                        ),
                        Err(_) => {
                            tracing::warn!("nginx discover timed out on {}", thread_conn_name)
                        }
                    }

                    match tokio::time::timeout(exec_timeout, session.exec(&mysql_cmd)).await {
                        Ok(Ok(result)) => {
                            let output = String::from_utf8_lossy(&result.stdout).to_string();
                            let _ = stream_tx.send(("mysql".to_string(), output));
                        }
                        Ok(Err(e)) => tracing::debug!(
                            "mysql discover exec error on {}: {}",
                            thread_conn_name,
                            e
                        ),
                        Err(_) => {
                            tracing::warn!("mysql discover timed out on {}", thread_conn_name)
                        }
                    }

                    match tokio::time::timeout(exec_timeout, session.exec(&pg_cmd)).await {
                        Ok(Ok(result)) => {
                            let output = String::from_utf8_lossy(&result.stdout).to_string();
                            let _ = stream_tx.send(("pg".to_string(), output));
                        }
                        Ok(Err(e)) => {
                            tracing::debug!("pg discover exec error on {}: {}", thread_conn_name, e)
                        }
                        Err(_) => tracing::warn!("pg discover timed out on {}", thread_conn_name),
                    }

                    let _ = done_tx.send(true);
                });
            });
        if let Err(e) = spawn_result {
            tracing::error!("Failed to spawn sites-discover thread: {}", e);
            self.show_toast(
                format!("Failed to discover sites: {}", e),
                ToastLevel::Error,
                cx,
            );
            return;
        }

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let mut new_sites: Vec<ManagedSite> = Vec::new();
            let wall_clock_start = std::time::Instant::now();
            let wall_clock_limit = std::time::Duration::from_secs(90);

            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;

                while let Ok((kind, data)) = stream_rx.try_recv() {
                    match kind.as_str() {
                        "nginx" => {
                            let sites = discovery::parse_nginx_configs(&data);
                            for site in sites {
                                new_sites.push(ManagedSite::from_nginx(conn_id, &conn_name, site));
                            }
                        }
                        "mysql" => {
                            let dbs = discovery::parse_mysql_discovery(&data);
                            for db in dbs {
                                new_sites.push(ManagedSite::from_database(conn_id, &conn_name, db));
                            }
                        }
                        "pg" => {
                            let dbs = discovery::parse_pg_discovery(&data);
                            for db in dbs {
                                new_sites.push(ManagedSite::from_database(conn_id, &conn_name, db));
                            }
                        }
                        _ => {}
                    }
                }

                if done_rx.try_recv().is_ok() {
                    let _ = this.update(cx, |ws, cx| {
                        let _ = ws.store.add_managed_sites_bulk(new_sites);
                        ws.sites.update(cx, |view, _| {
                            view.scans_pending = view.scans_pending.saturating_sub(1);
                            view.set_sites(ws.store.managed_sites.clone());
                        });
                        cx.notify();
                    });
                    break;
                }

                // Wall-clock safety: abort if background thread is stuck
                if wall_clock_start.elapsed() > wall_clock_limit {
                    tracing::warn!(
                        "Sites discover poller timed out after 90s for conn {}",
                        conn_name
                    );
                    let _ = this.update(cx, |ws, cx| {
                        let _ = ws.store.add_managed_sites_bulk(new_sites);
                        ws.sites.update(cx, |view, _| {
                            view.scans_pending = view.scans_pending.saturating_sub(1);
                            view.set_sites(ws.store.managed_sites.clone());
                        });
                        cx.notify();
                    });
                    break;
                }
            }
        })
        .detach();
    }
}
