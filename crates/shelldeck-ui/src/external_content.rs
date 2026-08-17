//! Presentation adapters for text received from external systems.
//!
//! Source payloads stay untouched. These helpers only derive reader-facing
//! labels for compact UI surfaces such as request lists and headers.

/// Turn a request title into a single-line display label and resolve the Slack
/// mrkdwn tokens that otherwise leak into Support lists (`<url|label>`,
/// `<url>`, channel references and broadcast mentions).
pub(crate) fn external_title(source: &str) -> String {
    let mut rendered = String::with_capacity(source.len());
    let mut cursor = 0usize;

    while let Some(relative_start) = source[cursor..].find('<') {
        let start = cursor + relative_start;
        rendered.push_str(&source[cursor..start]);

        let token_start = start + 1;
        let Some(relative_end) = source[token_start..].find('>') else {
            rendered.push_str(&source[start..]);
            cursor = source.len();
            break;
        };
        let end = token_start + relative_end;
        let token = &source[token_start..end];

        if let Some(label) = slack_token_label(token) {
            rendered.push_str(&label);
        } else {
            rendered.push_str(&source[start..=end]);
        }
        cursor = end + 1;
    }

    if cursor < source.len() {
        rendered.push_str(&source[cursor..]);
    }

    rendered.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn slack_token_label(token: &str) -> Option<String> {
    let (target, supplied_label) = token
        .split_once('|')
        .map_or((token, None), |(target, label)| (target, Some(label)));
    let supplied_label = supplied_label
        .map(str::trim)
        .filter(|label| !label.is_empty());

    if target.starts_with("https://") || target.starts_with("http://") {
        return Some(supplied_label.unwrap_or(target).to_string());
    }
    if let Some(address) = target.strip_prefix("mailto:") {
        return Some(supplied_label.unwrap_or(address).to_string());
    }
    if target.starts_with('#') {
        return supplied_label.map(|label| format!("#{}", label.trim_start_matches('#')));
    }
    if target.starts_with('@') {
        return supplied_label.map(|label| format!("@{}", label.trim_start_matches('@')));
    }
    if matches!(target, "!channel" | "!everyone" | "!here") {
        return Some(format!("@{}", target.trim_start_matches('!')));
    }
    if target.starts_with("!subteam^") || target.starts_with("!date^") {
        return supplied_label.map(ToOwned::to_owned);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::external_title;

    // SDTEST-1610
    #[test]
    fn slack_links_use_their_label_or_readable_url() {
        assert_eq!(
            external_title("Incident <https://status.example.com|sur la production>"),
            "Incident sur la production"
        );
        assert_eq!(
            external_title("<https://github.com/inklura/example/issues/42>"),
            "https://github.com/inklura/example/issues/42"
        );
        assert_eq!(
            external_title("<mailto:support@example.com|Contacter le support>"),
            "Contacter le support"
        );
    }

    // SDTEST-1611
    #[test]
    fn slack_references_are_readable_but_unknown_angle_text_is_preserved() {
        assert_eq!(
            external_title("Voir <#C123|incidents> avec <!here>"),
            "Voir #incidents avec @here"
        );
        assert_eq!(
            external_title("Alerter <!subteam^S123|@astreinte>"),
            "Alerter @astreinte"
        );
        assert_eq!(external_title("Valeur <non-slack>"), "Valeur <non-slack>");
    }

    // SDTEST-1612
    #[test]
    fn external_titles_are_trimmed_and_kept_on_one_line() {
        assert_eq!(
            external_title("  Incident\n  réseau   critique  "),
            "Incident réseau critique"
        );
    }
}
