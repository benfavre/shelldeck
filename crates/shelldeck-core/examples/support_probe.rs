//! Manual probe for the support console client — exercises the real Rust
//! deserializers against a live endpoint (list → detail → agents).
//!
//! ```bash
//! SHELLDECK_SYNC_URL=https://manage.inklura.fr \
//! SHELLDECK_SYNC_TOKEN=sd_... \
//!   cargo run -p shelldeck-core --example support_probe
//! ```
//! Read-only: it never posts replies/notes/status changes.

use shelldeck_core::config::manage_support as ms;

fn main() {
    let base = std::env::var("SHELLDECK_SYNC_URL")
        .unwrap_or_else(|_| "https://manage.inklura.fr".to_string());
    let token = match std::env::var("SHELLDECK_SYNC_TOKEN") {
        Ok(t) if !t.trim().is_empty() => t.trim().to_string(),
        _ => {
            eprintln!("SHELLDECK_SYNC_TOKEN is required.");
            std::process::exit(2);
        }
    };

    let list = match ms::support_list(&base, &token) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("support_list FAILED: {}", e);
            std::process::exit(1);
        }
    };
    println!(
        "list OK — {} tickets, counts all={} unassigned={} open={} pending={} breaching={}, me={} (staff={})",
        list.tickets.len(),
        list.counts.all,
        list.counts.unassigned,
        list.counts.open,
        list.counts.pending,
        list.counts.breaching,
        list.me.email,
        list.me.staff,
    );
    for t in list.tickets.iter().take(3) {
        println!(
            "  - [{}] {} | {} | {} | prio {} | {} | {}",
            t.channel,
            &t.id,
            if t.subject.is_empty() {
                "(sans objet)"
            } else {
                &t.subject
            },
            t.status,
            t.priority,
            t.contact.display(),
            if t.is_unassigned() {
                "unassigned".into()
            } else {
                t.assignee.clone()
            },
        );
    }

    if let Some(first) = list.tickets.first() {
        match ms::support_ticket(&base, &token, &first.id) {
            Ok(detail) => {
                println!("detail OK — {} messages:", detail.messages.len());
                for m in detail.messages.iter().take(6) {
                    let kind = if m.is_customer() {
                        "client"
                    } else if m.is_note() {
                        "note"
                    } else {
                        "agent"
                    };
                    let text: String = m.text.chars().take(48).collect();
                    println!("    {} | {}", kind, text);
                }
            }
            Err(e) => {
                eprintln!("support_ticket FAILED: {}", e);
                std::process::exit(1);
            }
        }
    }

    match ms::support_agents(&base, &token) {
        Ok(agents) => println!("agents OK — {} agents", agents.len()),
        Err(e) => {
            eprintln!("support_agents FAILED: {}", e);
            std::process::exit(1);
        }
    }
}
