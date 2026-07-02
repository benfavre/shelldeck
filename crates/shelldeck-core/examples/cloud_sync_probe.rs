//! Manual Cloud Sync probe — fetch and print the remote profile payload.
//!
//! Reads the endpoint and token from the environment so no secret is ever
//! baked into the repo:
//!
//! ```bash
//! SHELLDECK_SYNC_URL=https://manage.inklura.fr \
//! SHELLDECK_SYNC_TOKEN=sd_... \
//!   cargo run -p shelldeck-core --example cloud_sync_probe
//! ```
//!
//! `SHELLDECK_SYNC_URL` defaults to `https://manage.inklura.fr` if unset.
//! This does a real device check-in (`POST`) via the same code path the app
//! uses, so it also exercises the 404/405 → GET fallback and 401 handling.

use shelldeck_core::config::cloud_sync::{fetch_sync, CloudSyncConfig};

fn main() {
    let base_url = std::env::var("SHELLDECK_SYNC_URL")
        .unwrap_or_else(|_| "https://manage.inklura.fr".to_string());
    let token = match std::env::var("SHELLDECK_SYNC_TOKEN") {
        Ok(t) if !t.trim().is_empty() => t.trim().to_string(),
        _ => {
            eprintln!("SHELLDECK_SYNC_TOKEN is required (bearer token for the sync endpoint).");
            std::process::exit(2);
        }
    };

    let cfg = CloudSyncConfig {
        enabled: true,
        base_url,
        token,
        sync_on_startup: false,
    };

    println!("Probing {} …", cfg.base_url);
    match fetch_sync(&cfg, shelldeck_core::VERSION) {
        Ok(payload) => {
            println!(
                "OK — payload version {}, generated_at {:?}, {} connection(s):",
                payload.version,
                payload.generated_at,
                payload.connections.len()
            );
            for c in &payload.connections {
                println!(
                    "  - {} [{}]  {}@{}:{}  proxy_jump={:?} group={:?} tags={:?} forward_agent={} identity_file={:?}",
                    c.alias,
                    c.id,
                    c.user,
                    c.hostname,
                    c.port,
                    c.proxy_jump,
                    c.group,
                    c.tags,
                    c.forward_agent,
                    c.identity_file,
                );
            }
        }
        Err(e) => {
            eprintln!("FAILED: {}", e);
            std::process::exit(1);
        }
    }
}
