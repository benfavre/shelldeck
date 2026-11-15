//! Inklura Pro is an independent web session, never an agent browser grant.
use uuid::Uuid;

/// Hosted origin explicitly selected for the Pro workspace.
pub const DEFAULT_URL: &str = "https://pro.inklura.fr";

pub(super) fn default_url() -> String {
    DEFAULT_URL.to_owned()
}

pub(super) fn deserialize_url<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let value = <String as serde::Deserialize>::deserialize(deserializer)?;
    Ok(if value.trim().is_empty() {
        default_url()
    } else {
        value
    })
}

/// Only encrypted hosted apps or loopback development servers. No credentials,
/// query tokens, fragments, remote cleartext HTTP, or non-web schemes.
pub fn normalize_url(value: &str) -> Result<String, &'static str> {
    let url = url::Url::parse(value.trim()).map_err(|_| "invalid_url")?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err("invalid_url");
    }
    Ok(url.to_string())
}

pub fn profile_scope(manage_origin: &str, email: &str, pro_url: &str) -> String {
    let namespace = Uuid::new_v5(&Uuid::NAMESPACE_URL, b"inklura-pro-workspace-v1");
    let identity = serde_json::to_vec(&(
        manage_origin.trim_end_matches('/'),
        email.trim().to_lowercase(),
        pro_url,
    ))
    .expect("strings serialize");
    Uuid::new_v5(&namespace, &identity).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-3176 / SDUC-586: old and new config files select the chosen origin
    // without overwriting explicit administrator or loopback addresses.
    #[test]
    fn sdtest_3176_pro_config_defaults_preserve_explicit_origins() {
        use crate::config::AppConfig;
        assert_eq!(AppConfig::default().general.pro_web_url, DEFAULT_URL);
        for saved in [
            None,
            Some(""),
            Some("  "),
            Some("https://custom.example.test"),
            Some("http://127.0.0.1:8096"),
            Some("invalid-explicit-origin"),
        ] {
            let mut document = toml::Value::try_from(AppConfig::default()).unwrap();
            let general = document["general"].as_table_mut().unwrap();
            general.remove("pro_web_url");
            if let Some(value) = saved {
                general.insert("pro_web_url".into(), toml::Value::String(value.into()));
            }
            let config: AppConfig = document.try_into().unwrap();
            let expected = saved
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(DEFAULT_URL);
            assert_eq!(config.general.pro_web_url, expected, "saved={saved:?}");
        }
    }
    // SDTEST-3171 / SDUC-586: refuse credential/transport confusion and isolate accounts.
    #[test]
    fn sdtest_3171_pro_origin_and_account_isolation() {
        for bad in [
            "javascript:alert(1)",
            "file:///tmp/app",
            "http://example.com",
            "https://user:pass@example.com",
            "https://example.com/?token=x",
            "https://example.com/#token",
            "https://example.com/subpath",
        ] {
            assert!(normalize_url(bad).is_err(), "{bad}");
        }
        assert_eq!(
            normalize_url(" http://127.0.0.1:8094 ").unwrap(),
            "http://127.0.0.1:8094/"
        );
        let a = profile_scope(
            "https://manage.example",
            "a@example.test",
            "https://pro.example/",
        );
        assert_ne!(
            a,
            profile_scope(
                "https://manage.example",
                "b@example.test",
                "https://pro.example/"
            )
        );
        assert_ne!(
            a,
            profile_scope(
                "https://other.example",
                "a@example.test",
                "https://pro.example/"
            )
        );
        assert_ne!(
            a,
            profile_scope(
                "https://manage.example",
                "a@example.test",
                "https://other-pro.example/"
            )
        );
    }
}
