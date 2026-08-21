//! UI translations — rust-i18n helpers (init macro lives in `lib.rs`).

use shelldeck_core::config::app_config::UiLanguage;

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
