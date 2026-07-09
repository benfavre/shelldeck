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

#[cfg(test)]
mod tests {
    use super::*;

    /// Single test — `rust_i18n::set_locale` is process-global; parallel tests race.
    #[test]
    fn locale_fr_and_en() {
        apply_ui_language(&UiLanguage::Fr);
        assert_eq!(resolve_locale(&UiLanguage::Fr), "fr");
        assert_eq!(crate::t!("login.submit").as_ref(), "Se connecter");

        apply_ui_language(&UiLanguage::En);
        assert_eq!(resolve_locale(&UiLanguage::En), "en");
        assert_eq!(crate::t!("login.submit").as_ref(), "Sign in");
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
