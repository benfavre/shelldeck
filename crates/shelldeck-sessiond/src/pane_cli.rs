//! The parts of the pane commands of `shelldeck-sessiond` (`ls`, `run`,
//! `send`, `read`, `wait`) that need no daemon: choosing a pane from what
//! the user typed and printing the pane list. See `docs/pane-cli.md`.

use crate::protocol::PaneInfo;
use uuid::Uuid;

/// The shortest id prefix accepted as a pane selector. Shorter ones are too
/// easy to hit by accident with a name.
const MIN_ID_PREFIX: usize = 4;

/// Find the pane a selector means: a pane id, a name given at `run --name`,
/// or the start of an id. An ambiguous selector is an error, never a guess:
/// the commands type into panes and signal them.
pub fn resolve_pane(selector: &str, panes: &[PaneInfo]) -> Result<Uuid, String> {
    let selector = selector.trim();
    if selector.is_empty() {
        return Err("expected a pane (an id, the start of one, or a name)".into());
    }
    if let Ok(id) = selector.parse::<Uuid>() {
        return panes
            .iter()
            .find(|info| info.pane == id)
            .map(|info| info.pane)
            .ok_or_else(|| format!("no pane {id}"));
    }
    let named: Vec<&PaneInfo> = panes.iter().filter(|info| info.label == selector).collect();
    match named.as_slice() {
        [one] => return Ok(one.pane),
        [] => {}
        several => {
            return Err(format!(
                "{} panes are named {selector:?}: {}",
                several.len(),
                ids(several)
            ))
        }
    }
    let prefix = selector.to_ascii_lowercase();
    let is_prefix =
        prefix.len() >= MIN_ID_PREFIX && prefix.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-');
    if is_prefix {
        let matching: Vec<&PaneInfo> = panes
            .iter()
            .filter(|info| info.pane.to_string().starts_with(&prefix))
            .collect();
        match matching.as_slice() {
            [one] => return Ok(one.pane),
            [] => {}
            several => {
                return Err(format!(
                    "{} pane ids start with {selector:?}: {}",
                    several.len(),
                    ids(several)
                ))
            }
        }
    }
    Err(format!("no pane matches {selector:?}"))
}

fn ids(panes: &[&PaneInfo]) -> String {
    panes
        .iter()
        .map(|info| info.pane.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A JSON string literal.
pub fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The pane list as a JSON array, one pane per line. `exit_code` is null
/// while the pane's program runs.
pub fn panes_json(panes: &[PaneInfo]) -> String {
    let rows: Vec<String> = panes
        .iter()
        .map(|info| {
            format!(
                "{{\"id\":{},\"name\":{},\"title\":{},\"cwd\":{},\"rows\":{},\"cols\":{},\
                 \"alive\":{},\"exit_code\":{},\"clients\":{},\"created_at\":{}}}",
                json_string(&info.pane.to_string()),
                json_string(&info.label),
                json_string(&info.title),
                json_string(&info.cwd),
                info.rows,
                info.cols,
                info.alive,
                if info.alive {
                    "null".to_string()
                } else {
                    info.exit_code.to_string()
                },
                info.clients,
                info.created_at,
            )
        })
        .collect();
    if rows.is_empty() {
        "[]".to_string()
    } else {
        format!("[\n  {}\n]", rows.join(",\n  "))
    }
}

/// The pane list for people: id, name, state, size, attached clients, title.
pub fn panes_table(panes: &[PaneInfo]) -> String {
    let mut out = String::new();
    for info in panes {
        let state = if info.alive {
            "running".to_string()
        } else {
            format!("exited({})", info.exit_code)
        };
        let name = if info.label.is_empty() {
            "-"
        } else {
            info.label.as_str()
        };
        out.push_str(&format!(
            "{}  {:<16}  {:<11}  {}x{}  clients={}  {}\n",
            info.pane, name, state, info.rows, info.cols, info.clients, info.title
        ));
    }
    out
}

/// The process exit status for a pane that ended: its exit code, or
/// 128 + signal like a shell reports a killed child.
pub fn exit_status(exit_code: i32, signal: u8) -> u8 {
    if signal > 0 {
        return 128u8.saturating_add(signal);
    }
    exit_code.clamp(0, 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: u128, label: &str, alive: bool) -> PaneInfo {
        PaneInfo {
            pane: Uuid::from_u128(id),
            group: Uuid::nil(),
            label: label.into(),
            title: "user@host: ~".into(),
            cwd: "/srv/app".into(),
            rows: 40,
            cols: 120,
            created_at: 1_700_000_000,
            alive,
            exit_code: if alive { 0 } else { 3 },
            clients: 1,
            offset: 0,
        }
    }

    // SDTEST-3104 / SDUC-578. A selector means exactly one pane or nothing:
    // these commands type into panes and signal them, so a name shared by
    // two panes or a short prefix is refused, never guessed.
    #[test]
    fn sdtest_3104_a_pane_selector_names_exactly_one_pane() {
        let a = 0xaaaa_1111_0000_0000_0000_0000_0000_0001;
        let b = 0xaaaa_2222_0000_0000_0000_0000_0000_0002;
        let c = 0xbbbb_0000_0000_0000_0000_0000_0000_0003;
        let panes = [
            pane(a, "reviewer", true),
            pane(b, "web", true),
            pane(c, "web", false),
        ];
        let id = |n: u128| Uuid::from_u128(n);

        assert_eq!(resolve_pane(&id(b).to_string(), &panes), Ok(id(b)));
        assert_eq!(resolve_pane(" reviewer ", &panes), Ok(id(a)));
        assert_eq!(resolve_pane("aaaa1", &panes), Ok(id(a)));
        assert_eq!(resolve_pane("BBBB", &panes), Ok(id(c)));

        // Two panes named "web", two ids starting with "aaaa".
        assert!(resolve_pane("web", &panes).unwrap_err().contains("2 panes"));
        assert!(resolve_pane("aaaa", &panes)
            .unwrap_err()
            .contains("2 pane ids"));
        // Too short to be an id prefix, and no pane has that name.
        assert!(resolve_pane("aaa", &panes).is_err());
        assert!(resolve_pane("", &panes).is_err());
        assert!(resolve_pane("nobody", &panes).is_err());
        // A full id that is not in the list is not matched by prefix.
        assert!(resolve_pane(&id(0xdead).to_string(), &panes).is_err());
    }

    // SDTEST-3105 / SDUC-578. The JSON list is what agents parse: strings
    // are escaped, a running pane has no exit code, an empty list is `[]`.
    #[test]
    fn sdtest_3105_the_json_pane_list_escapes_text_and_marks_running_panes() {
        assert_eq!(panes_json(&[]), "[]");
        let mut running = pane(1, "we\"b", true);
        running.title = "a\\b\nc\u{1}".into();
        let json = panes_json(&[running, pane(2, "", false)]);
        assert!(json.contains(r#""name":"we\"b""#), "{json}");
        assert!(json.contains(r#""title":"a\\b\nc\u0001""#), "{json}");
        assert!(json.contains(r#""alive":true,"exit_code":null"#), "{json}");
        assert!(json.contains(r#""alive":false,"exit_code":3"#), "{json}");
        assert_eq!(json.lines().count(), 4, "one pane per line:\n{json}");

        assert_eq!(exit_status(0, 0), 0);
        assert_eq!(exit_status(7, 0), 7);
        assert_eq!(exit_status(-1, 0), 0);
        assert_eq!(exit_status(0, 9), 137);
    }
}
