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
}
