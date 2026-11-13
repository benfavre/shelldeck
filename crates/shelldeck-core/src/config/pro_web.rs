//! Inklura Pro is an independent web session, never an agent browser grant.
use uuid::Uuid;

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
