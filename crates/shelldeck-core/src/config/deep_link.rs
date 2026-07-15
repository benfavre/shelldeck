//! `shelldeck://` deep-link parser.
//!
//! Deep links are the seam between ShellDeck and the surfaces around it
//! (Manage, Slack, JeanClaude, e-mails): a click on a `shelldeck://…`
//! URL focuses the app on the right view or fires the right action. The
//! OS hands the URL to ShellDeck as a process argument; if an instance is
//! already running, [`crate::config::single_instance`] forwards it to the
//! live process, which routes it through `Workspace::open_deep_link`.
//!
//! This module is deliberately **pure + std-only**: it turns a `&str`
//! into a typed [`DeepLink`] (or `None`) and nothing else. All the side
//! effects live in `shelldeck-ui`. That keeps the grammar unit-testable
//! without a running app.
//!
//! ## Grammar
//!
//! | URL | Variant |
//! |-----|---------|
//! | `shelldeck://open/connection/<uuid>` | [`DeepLink::OpenConnection`] |
//! | `shelldeck://ssh/connect/<uuid>`     | [`DeepLink::SshConnect`] |
//! | `shelldeck://tunnel/start/<uuid>`    | [`DeepLink::TunnelStart`] |
//! | `shelldeck://open/site/<id>`         | [`DeepLink::OpenSite`] |
//! | `shelldeck://issue/<id>`             | [`DeepLink::OpenIssue`] |
//! | `shelldeck://ticket/<id>`            | [`DeepLink::OpenTicket`] |
//! | `shelldeck://jean/confirm/<job_id>`  | [`DeepLink::JeanConfirm`] |
//!
//! Anything else (unknown verb, bad UUID, wrong scheme) parses to `None`
//! so the caller can no-op safely instead of guessing.

use uuid::Uuid;

/// The URL scheme ShellDeck registers with the OS. Kept as a constant so
/// the packaging scripts, the single-instance guard, and this parser all
/// agree on the exact string.
pub const SCHEME: &str = "shelldeck";

/// A parsed, typed deep link. IDs mirror the model types they resolve
/// against: connections/tunnels are `Uuid` (validated at parse time),
/// while sites/tickets/issues/Jean jobs are opaque `String` IDs (the
/// server owns their shape).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLink {
    /// Focus the connections view and highlight this connection (no SSH).
    OpenConnection(Uuid),
    /// Open the app and start an SSH session for this connection.
    SshConnect(Uuid),
    /// Open the app and start this saved port-forward / tunnel.
    TunnelStart(Uuid),
    /// Switch to this site and show the User home.
    OpenSite(String),
    /// Open the request/issue in Support (or the User detail sheet).
    OpenIssue(String),
    /// Open the support ticket in Support mode.
    OpenTicket(String),
    /// Open the Fleet view where the Jean job awaits confirmation.
    JeanConfirm(String),
}

impl DeepLink {
    /// Parse a `shelldeck://…` URL into a typed link, or `None` when the
    /// scheme is wrong, the verb is unknown, or an embedded UUID is
    /// malformed. Query strings and fragments are ignored.
    pub fn parse(url: &str) -> Option<Self> {
        let url = url.trim();
        // Case-insensitive scheme match, tolerant of `shelldeck://` only
        // (we never mint `shelldeck:` opaque URLs). Strip the scheme then
        // work on the remaining `authority/path` as opaque segments.
        let prefix = format!("{SCHEME}://");
        let rest = strip_prefix_ci(url, &prefix)?;

        // Drop query + fragment before splitting into path segments.
        let rest = rest
            .split(['?', '#'])
            .next()
            .unwrap_or("")
            .trim_matches('/');
        if rest.is_empty() {
            return None;
        }

        let segments: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
        // Verbs are matched case-insensitively; the trailing ID keeps its
        // original casing (server-side ticket/issue/site IDs are
        // case-sensitive). We match on a lowercased view of the structural
        // segments but read IDs from the untouched `segments`.
        let verbs: Vec<String> = segments.iter().map(|s| s.to_ascii_lowercase()).collect();
        let verbs: Vec<&str> = verbs.iter().map(|s| s.as_str()).collect();
        match verbs.as_slice() {
            ["open", "connection", _] => {
                Uuid::parse_str(segments[2]).ok().map(DeepLink::OpenConnection)
            }
            ["ssh", "connect", _] => Uuid::parse_str(segments[2]).ok().map(DeepLink::SshConnect),
            ["tunnel", "start", _] => {
                Uuid::parse_str(segments[2]).ok().map(DeepLink::TunnelStart)
            }
            ["open", "site", _] => non_empty(segments[2]).map(|s| DeepLink::OpenSite(s.to_string())),
            ["issue", _] => non_empty(segments[1]).map(|s| DeepLink::OpenIssue(s.to_string())),
            ["ticket", _] => non_empty(segments[1]).map(|s| DeepLink::OpenTicket(s.to_string())),
            ["jean", "confirm", _] => {
                non_empty(segments[2]).map(|s| DeepLink::JeanConfirm(s.to_string()))
            }
            _ => None,
        }
    }

    /// True when `arg` looks like a ShellDeck deep link (cheap prefix
    /// check used to sift the process arguments before parsing).
    pub fn looks_like(arg: &str) -> bool {
        let prefix = format!("{SCHEME}://");
        strip_prefix_ci(arg.trim(), &prefix).is_some()
    }
}

/// Case-insensitive `strip_prefix`. Only the ASCII prefix is lowercased —
/// the returned remainder keeps its original casing (UUIDs/IDs are
/// case-sensitive on the ID side).
fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

fn non_empty(s: &str) -> Option<&str> {
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "550e8400-e29b-41d4-a716-446655440000";

    fn uuid() -> Uuid {
        Uuid::parse_str(UUID).unwrap()
    }

    // SDTEST-1320 — the full deep-link grammar round-trips to the right
    // variant. This is the single choke point every OS-delivered URL
    // flows through; a silent grammar drift would send clicks to the
    // wrong view (or nowhere), so every documented verb is pinned.
    #[test]
    fn parses_every_documented_verb() {
        assert_eq!(
            DeepLink::parse(&format!("shelldeck://open/connection/{UUID}")),
            Some(DeepLink::OpenConnection(uuid()))
        );
        assert_eq!(
            DeepLink::parse(&format!("shelldeck://ssh/connect/{UUID}")),
            Some(DeepLink::SshConnect(uuid()))
        );
        assert_eq!(
            DeepLink::parse(&format!("shelldeck://tunnel/start/{UUID}")),
            Some(DeepLink::TunnelStart(uuid()))
        );
        assert_eq!(
            DeepLink::parse("shelldeck://open/site/site_abc123"),
            Some(DeepLink::OpenSite("site_abc123".to_string()))
        );
        assert_eq!(
            DeepLink::parse("shelldeck://issue/iss_42"),
            Some(DeepLink::OpenIssue("iss_42".to_string()))
        );
        assert_eq!(
            DeepLink::parse("shelldeck://ticket/tkt_7"),
            Some(DeepLink::OpenTicket("tkt_7".to_string()))
        );
        assert_eq!(
            DeepLink::parse("shelldeck://jean/confirm/job_9"),
            Some(DeepLink::JeanConfirm("job_9".to_string()))
        );
    }

    #[test]
    fn scheme_is_case_insensitive_but_id_is_not() {
        assert_eq!(
            DeepLink::parse(&format!("ShellDeck://ISSUE/Iss_MixedCase")),
            Some(DeepLink::OpenIssue("Iss_MixedCase".to_string()))
        );
    }

    #[test]
    fn ignores_query_and_fragment_and_trailing_slash() {
        assert_eq!(
            DeepLink::parse("shelldeck://ticket/tkt_1/?foo=bar#frag"),
            Some(DeepLink::OpenTicket("tkt_1".to_string()))
        );
    }

    #[test]
    fn rejects_bad_scheme_and_unknown_verbs() {
        assert_eq!(DeepLink::parse("https://example.com/x"), None);
        assert_eq!(DeepLink::parse("shelldeck://unknown/thing"), None);
        assert_eq!(DeepLink::parse("shelldeck://"), None);
        assert_eq!(DeepLink::parse("shelldeck://open"), None);
        assert_eq!(DeepLink::parse("not a url at all"), None);
    }

    #[test]
    fn rejects_malformed_uuid() {
        assert_eq!(DeepLink::parse("shelldeck://ssh/connect/not-a-uuid"), None);
        assert_eq!(DeepLink::parse("shelldeck://open/connection/123"), None);
    }

    #[test]
    fn looks_like_prefix_check() {
        assert!(DeepLink::looks_like("shelldeck://issue/x"));
        assert!(DeepLink::looks_like("  ShellDeck://ticket/y  "));
        assert!(!DeepLink::looks_like("/usr/bin/shelldeck"));
        assert!(!DeepLink::looks_like("--minimized"));
    }
}
