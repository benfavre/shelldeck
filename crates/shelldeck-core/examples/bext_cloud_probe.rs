//! Live probe for the cloud.bext.dev + instance SDK clients. Reads the cloud token
//! from env (no secret in the repo):
//!
//! ```bash
//! BEXT_CLOUD_URL=https://cloud.bext.dev BEXT_CLOUD_TOKEN=bext_… \
//!   cargo run -p shelldeck-core --example bext_cloud_probe
//! ```
use shelldeck_core::config::bext_cloud::{self, BextCloudConfig};
use shelldeck_core::config::bext_instance::{self, BextInstance};

fn main() {
    let base_url =
        std::env::var("BEXT_CLOUD_URL").unwrap_or_else(|_| "https://cloud.bext.dev".to_string());
    let token = std::env::var("BEXT_CLOUD_TOKEN").unwrap_or_default();
    if !token.is_empty() {
        let cfg = BextCloudConfig {
            base_url,
            token,
            email: String::new(),
            name: String::new(),
        };
        match bext_cloud::whoami(&cfg) {
            Ok(u) => println!(
                "whoami: {} <{}> super_admin={}",
                u.name, u.email, u.is_super_admin
            ),
            Err(e) => {
                eprintln!("whoami failed: {}", e);
                std::process::exit(1);
            }
        }
        match bext_cloud::list_sites(&cfg) {
            Ok(s) => println!("sites: {}/{} ({} listed)", s.count, s.max, s.sites.len()),
            Err(e) => eprintln!("sites failed: {}", e),
        }
        match bext_cloud::dashboard(&cfg) {
            Ok(d) => println!(
                "dashboard stats: projects={} deploys={} domains={} targets={}",
                d.stats.projects, d.stats.deploys, d.stats.domains, d.stats.targets
            ),
            Err(e) => eprintln!("dashboard failed: {}", e),
        }
        match bext_cloud::list_instances(&cfg) {
            Ok(i) => {
                println!("instances: {}", i.total);
                for x in &i.instances {
                    println!("  - {} [{}] {} / {}", x.name, x.status, x.health, x.url);
                }
            }
            Err(e) => eprintln!("instances failed: {}", e),
        }
    } else {
        println!("(BEXT_CLOUD_TOKEN unset — skipping cloud API)");
    }

    // Instance SDK (this box's loopback, safe read).
    if let Ok(url) = std::env::var("BEXT_INSTANCE_URL") {
        let inst = BextInstance::new(
            url,
            std::env::var("BEXT_INSTANCE_APP_ID").unwrap_or_else(|_| "cloud-bext".into()),
        );
        match bext_instance::list_sites(&inst) {
            Ok(s) => {
                println!("instance sites: {}", s.sites.len());
                for x in s.sites.iter().take(5) {
                    println!("  - {} [{}] {}", x.slug, x.kind, x.primary_domain);
                }
            }
            Err(e) => eprintln!("instance list failed: {}", e),
        }
    }
}
