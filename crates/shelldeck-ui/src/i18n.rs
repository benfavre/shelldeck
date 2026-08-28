//! UI translations — rust-i18n helpers (init macro lives in `lib.rs`).

use shelldeck_core::config::app_config::UiLanguage;

use chrono::{DateTime, Local, TimeZone};

/// Apply the persisted UI language to the global rust-i18n locale.
pub fn apply_ui_language(preference: &UiLanguage) {
    rust_i18n::set_locale(resolve_locale(preference));
}

/// Resolve the effective rust-i18n locale tag from the user's preference.
pub fn resolve_locale(preference: &UiLanguage) -> &'static str {
    match preference {
        UiLanguage::Fr => "fr",
        UiLanguage::En => "en",
        UiLanguage::System => detect_system_locale(),
    }
}

/// Best-effort OS locale → `fr` or `en`. Unknown → **`fr`** (product default).
fn detect_system_locale() -> &'static str {
    sys_locale::get_locale()
        .map(|locale| if locale.starts_with("fr") { "fr" } else { "en" })
        .unwrap_or("fr")
}

/// Human-readable relative time for support/fleet timestamps (epoch ms).
pub fn rel_time(at_ms: f64) -> String {
    if at_ms <= 0.0 {
        return String::new();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(at_ms);
    let secs = ((now - at_ms) / 1000.0).max(0.0);
    if secs < 60.0 {
        crate::t!("time.just_now").to_string()
    } else if secs < 3600.0 {
        crate::t!("time.ago_minutes", count = (secs / 60.0) as i64).to_string()
    } else if secs < 86400.0 {
        crate::t!("time.ago_hours", count = (secs / 3600.0) as i64).to_string()
    } else {
        crate::t!("time.ago_days", count = (secs / 86400.0) as i64).to_string()
    }
}

/// Render an RFC 3339 timestamp as a short, local, customer-facing date.
///
/// Manage returns wire timestamps such as `2026-08-27T08:34:49.305Z`. They
/// remain useful in logs, but should never be shown verbatim in the UI.
pub fn local_timestamp(value: &str) -> String {
    let value = value.trim();
    let Ok(parsed) = DateTime::parse_from_rfc3339(value) else {
        return value.to_string();
    };
    friendly_datetime(parsed.with_timezone(&Local), Local::now())
}

fn friendly_datetime<Tz>(value: DateTime<Tz>, now: DateTime<Tz>) -> String
where
    Tz: TimeZone,
    Tz::Offset: std::fmt::Display,
{
    let time = value.format("%H:%M").to_string();
    if value.date_naive() == now.date_naive() {
        crate::t!("time.today_at", time = time).to_string()
    } else if Some(value.date_naive()) == now.date_naive().pred_opt() {
        crate::t!("time.yesterday_at", time = time).to_string()
    } else {
        let date = if rust_i18n::locale().starts_with("fr") {
            value.format("%d/%m/%Y").to_string()
        } else {
            value.format("%m/%d/%Y").to_string()
        };
        crate::t!("time.date_at", date = date, time = time).to_string()
    }
}

/// Phrase à montrer à l'utilisateur quand une requête vers Manage échoue.
///
/// Les clients construisent des messages techniques qui embarquent l'URL
/// interne — `"support list failed: error sending request for url
/// (http://…/api/manage/shelldeck/support?action=list)"`. Ce texte n'apprend
/// rien à qui l'utilise et expose l'adresse du portail. Il part donc dans les
/// journaux, et l'interface reçoit une phrase qui dit quoi faire.
pub fn api_error_message(err: &shelldeck_core::error::ShellDeckError) -> String {
    use shelldeck_core::config::cloud_account::{classify_api_error, ApiFailure};

    // Le détail reste accessible : il est indispensable pour diagnostiquer, il
    // n'a simplement rien à faire dans une bulle de notification.
    tracing::warn!("requête Manage échouée : {err}");

    let key = match classify_api_error(err) {
        ApiFailure::Unreachable => "error.api.unreachable",
        ApiFailure::Timeout => "error.api.timeout",
        ApiFailure::AuthRejected => "error.api.auth_rejected",
        ApiFailure::Forbidden => "error.api.forbidden",
        ApiFailure::NotFound => "error.api.not_found",
        ApiFailure::ServerError => "error.api.server",
        ApiFailure::BadResponse => "error.api.bad_response",
        ApiFailure::Other => "error.api.other",
    };
    crate::t!(key).to_string()
}

/// Human-readable error for the SDK exposed by one bext instance.
///
/// Its HTTP statuses do not describe Manage resources. In particular, a 404
/// can mean that the targeted instance does not expose the requested SDK route;
/// it must never be presented as an item deleted from the portal.
pub fn bext_instance_error_message(err: &shelldeck_core::error::ShellDeckError) -> String {
    use shelldeck_core::config::cloud_account::{classify_api_error, ApiFailure};

    tracing::warn!("requête Instance bext échouée : {err}");

    let key = match classify_api_error(err) {
        ApiFailure::Unreachable => "bext.error.instance_unreachable",
        ApiFailure::Timeout => "bext.error.instance_timeout",
        ApiFailure::AuthRejected | ApiFailure::Forbidden => "bext.error.instance_rejected",
        ApiFailure::NotFound => "bext.error.instance_not_found",
        ApiFailure::ServerError => "bext.error.instance_server",
        ApiFailure::BadResponse => "bext.error.instance_bad_response",
        ApiFailure::Other => "bext.error.instance_other",
    };
    crate::t!(key).to_string()
}

/// Human-readable error for the shared Platform projection.
///
/// Platform polling can retry the same unavailable endpoint several times.
/// `log_failure` lets its owner emit one diagnostic per outage instead of one
/// warning per poll while still rebuilding the localized UI message.
pub fn platform_error_message(
    err: &shelldeck_core::error::ShellDeckError,
    log_failure: bool,
) -> String {
    use shelldeck_core::config::cloud_account::{classify_api_error, ApiFailure};

    if log_failure {
        tracing::warn!("requête Plateforme échouée : {err}");
    }

    let key = match classify_api_error(err) {
        ApiFailure::Unreachable => "fleet.error.unreachable",
        ApiFailure::Timeout => "fleet.error.timeout",
        ApiFailure::AuthRejected => "fleet.error.auth_rejected",
        ApiFailure::Forbidden => "fleet.error.forbidden",
        ApiFailure::NotFound => "fleet.error.not_found",
        ApiFailure::ServerError => "fleet.error.server",
        ApiFailure::BadResponse => "fleet.error.bad_response",
        ApiFailure::Other => "fleet.error.other",
    };
    crate::t!(key).to_string()
}

/// Comme [`api_error_message`], mais pour un échec du **formulaire de
/// connexion**.
///
/// Un 401 n'y veut pas dire la même chose qu'ailleurs : pendant une session
/// c'est un jeton périmé, sur ce formulaire ce sont des identifiants refusés.
/// Répondre « Votre session a expiré » à quelqu'un qui vient de taper son mot
/// de passe l'envoie chercher un problème qui n'existe pas.
pub fn login_error_message(err: &shelldeck_core::error::ShellDeckError) -> String {
    use shelldeck_core::config::cloud_account::{classify_api_error, ApiFailure};

    if classify_api_error(err) == ApiFailure::AuthRejected {
        tracing::warn!("connexion refusée par le portail : {err}");
        return crate::t!("error.login.rejected").to_string();
    }
    api_error_message(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SDTEST-1656 — appelé dans chaque section de langue du test unique
    /// ci-dessous, jamais seul : `apply_ui_language` est global au processus.
    ///
    /// Pinne la régression qui a motivé `api_error_message` : l'utilisateur
    /// lisait « error sending request for url
    /// (http://127.0.0.1:8899/api/manage/shelldeck/sync) » dans une notification.
    fn assert_portal_failures_stay_readable(language: &str) {
        use shelldeck_core::error::ShellDeckError;

        let shown = api_error_message(&ShellDeckError::Connection(
            "cloud sync request failed: error sending request for url \
             (http://127.0.0.1:8899/api/manage/shelldeck/sync)"
                .to_string(),
        ));
        assert!(
            !shown.contains("http") && !shown.contains("://"),
            "URL interne exposée en {language} : {shown}",
        );
        assert!(
            !shown.contains("error sending request"),
            "jargon reqwest exposé en {language} : {shown}",
        );

        // Une session morte se dit dans la langue de l'utilisateur, pas en 401.
        let expired = api_error_message(&ShellDeckError::Connection(
            "session token rejected (401)".to_string(),
        ));
        assert!(
            !expired.contains("401"),
            "code HTTP exposé en {language} : {expired}",
        );
        assert_ne!(
            shown, expired,
            "portail injoignable et session expirée disent la même chose en {language}",
        );
    }

    /// SDTEST-1807 — reste dans le scénario bilingue unique car la locale
    /// rust-i18n est globale au processus.
    fn assert_bext_instance_failures_keep_sdk_context(language: &str, expected_404: &str) {
        use shelldeck_core::error::ShellDeckError;

        let not_found = bext_instance_error_message(&ShellDeckError::Connection(
            "instance SDK request failed: HTTP 404".to_string(),
        ));
        assert_eq!(not_found, expected_404);
        assert!(
            !not_found.to_ascii_lowercase().contains("portal")
                && !not_found.to_ascii_lowercase().contains("portail")
                && !not_found.contains("404"),
            "un statut Instance est attribué au portail en {language}: {not_found}",
        );

        let unreachable = bext_instance_error_message(&ShellDeckError::Connection(
            "instance SDK request failed: error sending request for url \
             (http://127.0.0.1/__bext/sdk/site/list)"
                .to_string(),
        ));
        assert!(
            !unreachable.contains("127.0.0.1") && !unreachable.contains("/__bext/"),
            "URL SDK exposée en {language}: {unreachable}",
        );
    }

    /// SDTEST-1704 — reste dans le scénario bilingue unique : la locale est
    /// globale au processus et ne doit jamais être modifiée par deux tests en
    /// parallèle.
    fn assert_operational_vocabulary_is_localized(
        unknown_status: &str,
        unknown_priority: &str,
        one_connection: &str,
        many_connections: &str,
        one_forward: &str,
        many_forwards: &str,
        one_script: &str,
        many_scripts: &str,
    ) {
        use crate::status_bar::{status_count_label, StatusMetric};

        assert_eq!(
            crate::support_view::status_label("awaiting_agent"),
            unknown_status
        );
        assert_eq!(
            crate::support_view::issue_status_label("awaiting_review"),
            unknown_status
        );
        assert_eq!(
            crate::support_view::priority_label("critical"),
            unknown_priority
        );

        assert_eq!(
            status_count_label(StatusMetric::ActiveConnections, 1),
            one_connection
        );
        assert_eq!(
            status_count_label(StatusMetric::ActiveConnections, 2),
            many_connections
        );
        assert_eq!(
            status_count_label(StatusMetric::ActiveForwards, 1),
            one_forward
        );
        assert_eq!(
            status_count_label(StatusMetric::ActiveForwards, 2),
            many_forwards
        );
        assert_eq!(
            status_count_label(StatusMetric::RunningScripts, 1),
            one_script
        );
        assert_eq!(
            status_count_label(StatusMetric::RunningScripts, 2),
            many_scripts
        );

        assert_eq!(
            crate::t!("sidebar.nav.server_sync"),
            crate::t!("menu.go.server_sync")
        );
        assert_eq!(
            crate::t!("sidebar.nav.server_sync"),
            crate::t!("sync.title")
        );
        assert_eq!(crate::t!("sidebar.nav.recent"), crate::t!("menu.go.recent"));
        assert_eq!(crate::t!("sidebar.nav.recent"), crate::t!("recent.title"));
    }

    /// SDTEST-1793 — appelé dans le scénario bilingue unique, car la locale
    /// rust-i18n est globale au processus.
    fn assert_account_timestamps_are_customer_facing(today: &str, yesterday: &str, older: &str) {
        let offset = chrono::FixedOffset::east_opt(2 * 60 * 60).unwrap();
        let now = offset.with_ymd_and_hms(2026, 8, 27, 12, 0, 0).unwrap();
        let same_day = offset.with_ymd_and_hms(2026, 8, 27, 10, 34, 49).unwrap();
        let previous_day = offset.with_ymd_and_hms(2026, 8, 26, 18, 5, 0).unwrap();
        let previous_month = offset.with_ymd_and_hms(2026, 7, 9, 8, 7, 0).unwrap();

        assert_eq!(friendly_datetime(same_day, now), today);
        assert_eq!(friendly_datetime(previous_day, now), yesterday);
        assert_eq!(friendly_datetime(previous_month, now), older);
        assert_eq!(local_timestamp("valeur historique"), "valeur historique");
    }

    /// Single test — `rust_i18n::set_locale` is process-global; parallel tests race.
    #[test]
    fn locale_fr_and_en() {
        apply_ui_language(&UiLanguage::Fr);
        assert_eq!(resolve_locale(&UiLanguage::Fr), "fr");
        assert_eq!(crate::t!("login.submit").as_ref(), "Se connecter");
        let tray_fr = crate::ai_dock::TrayLabels::localized();
        assert_eq!(tray_fr.show, "Ouvrir ShellDeck");
        assert_eq!(crate::ai_dock::tray_counter_tickets(3), "3 tickets non lus");
        assert_eq!(
            crate::ai_dock::tray_counter_ai_tasks(2),
            "2 tâches IA en cours"
        );
        assert_eq!(
            crate::workspace::TrayNotification::SshDisconnected {
                name: "production".to_string(),
            }
            .localized_text()
            .1,
            // L'espace avant les deux-points est insécable : c'est la
            // typographie française, pas une coquille. Elle est écrite en
            // échappement pour rester visible à la relecture.
            "Connexion interrompue\u{a0}: production"
        );
        assert_portal_failures_stay_readable("fr");
        assert_bext_instance_failures_keep_sdk_context(
            "fr",
            "Ressource ou route SDK introuvable sur l’Instance Bext. Vérifiez la cible et sa version.",
        );
        assert_operational_vocabulary_is_localized(
            "statut inconnu",
            "Priorité inconnue",
            "1 connexion active",
            "2 connexions actives",
            "1 redirection active",
            "2 redirections actives",
            "1 script en cours",
            "2 scripts en cours",
        );
        assert_account_timestamps_are_customer_facing(
            "Aujourd’hui à 10:34",
            "Hier à 18:05",
            "09/07/2026 à 08:07",
        );

        apply_ui_language(&UiLanguage::En);
        assert_eq!(resolve_locale(&UiLanguage::En), "en");
        assert_eq!(crate::t!("login.submit").as_ref(), "Sign in");
        let tray_en = crate::ai_dock::TrayLabels::localized();
        assert_eq!(tray_en.show, "Open ShellDeck");
        assert_eq!(crate::ai_dock::tray_counter_tickets(3), "3 unread tickets");
        assert_eq!(
            crate::ai_dock::tray_counter_ai_tasks(2),
            "2 AI tasks running"
        );
        assert_eq!(
            crate::workspace::TrayNotification::SshDisconnected {
                name: "production".to_string(),
            }
            .localized_text()
            .1,
            "Connection interrupted: production"
        );
        assert_portal_failures_stay_readable("en");
        assert_bext_instance_failures_keep_sdk_context(
            "en",
            "Resource or SDK route not found on the Bext instance. Check the target and its version.",
        );
        assert_operational_vocabulary_is_localized(
            "unknown status",
            "Unknown priority",
            "1 active connection",
            "2 active connections",
            "1 active port forward",
            "2 active port forwards",
            "1 running script",
            "2 running scripts",
        );
        assert_account_timestamps_are_customer_facing(
            "Today at 10:34",
            "Yesterday at 18:05",
            "07/09/2026 at 08:07",
        );
    }

    #[test]
    fn resolve_locale_system_is_fr_or_en() {
        let loc = resolve_locale(&UiLanguage::System);
        assert!(loc == "fr" || loc == "en");
    }

    /// SDTEST-1302 — key parity between `fr.toml` and `en.toml`.
    ///
    /// Every key present in one locale MUST exist in the other. `AGENTS.md`
    /// § i18n commits to French fallback ­­(`rust_i18n::i18n!(fallback = "fr")`),
    /// but that mechanism silently masks a missing translation as
    /// "same as French" — a divergence would ship without any visible
    /// error until an English-speaking user notices a random FR string
    /// in the UI. This test is the regression sensor.
    ///
    /// Locale files are shape-flat (dotted keys, no nested tables), so
    /// we parse them as `HashMap<String, toml::Value>` and diff the key
    /// sets.
    #[test]
    fn fr_en_locale_key_parity() {
        use std::collections::BTreeSet;

        let fr_src = include_str!("../../shelldeck-core/locales/fr.toml");
        let en_src = include_str!("../../shelldeck-core/locales/en.toml");

        let fr: toml::Table = toml::from_str(fr_src).expect("fr.toml parses");
        let en: toml::Table = toml::from_str(en_src).expect("en.toml parses");

        let fr_keys: BTreeSet<&str> = fr.keys().map(String::as_str).collect();
        let en_keys: BTreeSet<&str> = en.keys().map(String::as_str).collect();

        let only_in_fr: Vec<&&str> = fr_keys.difference(&en_keys).collect();
        let only_in_en: Vec<&&str> = en_keys.difference(&fr_keys).collect();

        assert!(
            only_in_fr.is_empty() && only_in_en.is_empty(),
            "locale key drift — only in fr.toml: {only_in_fr:?}, only in en.toml: {only_in_en:?}",
        );
    }
}
