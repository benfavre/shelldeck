use crate::error::{Result, ShellDeckError};
use tracing::debug;

const SERVICE_NAME: &str = "shelldeck-ssh";

/// Build a keyring entry key from host and user.
fn entry_key(host: &str, user: &str) -> String {
    format!("{}@{}", user, host)
}

/// Store a password in the OS keychain for a given host and user.
pub fn store_password(host: &str, user: &str, password: &str) -> Result<()> {
    let key = entry_key(host, user);
    let entry = keyring::Entry::new(SERVICE_NAME, &key)
        .map_err(|e| ShellDeckError::Keychain(format!("Failed to create keyring entry: {}", e)))?;
    entry
        .set_password(password)
        .map_err(|e| ShellDeckError::Keychain(format!("Failed to store password: {}", e)))?;
    debug!("Stored password for {}", key);
    Ok(())
}

/// Retrieve a password from the OS keychain. Returns None if not found.
pub fn get_password(host: &str, user: &str) -> Result<Option<String>> {
    let key = entry_key(host, user);
    let entry = keyring::Entry::new(SERVICE_NAME, &key)
        .map_err(|e| ShellDeckError::Keychain(format!("Failed to create keyring entry: {}", e)))?;
    match entry.get_password() {
        Ok(password) => {
            debug!("Retrieved password for {}", key);
            Ok(Some(password))
        }
        Err(keyring::Error::NoEntry) => {
            debug!("No password stored for {}", key);
            Ok(None)
        }
        Err(e) => Err(ShellDeckError::Keychain(format!(
            "Failed to retrieve password: {}",
            e
        ))),
    }
}

/// Delete a password from the OS keychain. Returns Ok(()) even if not found.
pub fn delete_password(host: &str, user: &str) -> Result<()> {
    let key = entry_key(host, user);
    let entry = keyring::Entry::new(SERVICE_NAME, &key)
        .map_err(|e| ShellDeckError::Keychain(format!("Failed to create keyring entry: {}", e)))?;
    match entry.delete_credential() {
        Ok(()) => {
            debug!("Deleted password for {}", key);
            Ok(())
        }
        Err(keyring::Error::NoEntry) => {
            debug!("No password to delete for {}", key);
            Ok(())
        }
        Err(e) => Err(ShellDeckError::Keychain(format!(
            "Failed to delete password: {}",
            e
        ))),
    }
}

/// Build a keyring entry key for a key file passphrase.
fn passphrase_entry_key(key_path: &str) -> String {
    format!("passphrase:{}", key_path)
}

/// Store a private key passphrase in the OS keychain, keyed by the key file path.
pub fn store_key_passphrase(key_path: &str, passphrase: &str) -> Result<()> {
    let key = passphrase_entry_key(key_path);
    let entry = keyring::Entry::new(SERVICE_NAME, &key)
        .map_err(|e| ShellDeckError::Keychain(format!("Failed to create keyring entry: {}", e)))?;
    entry
        .set_password(passphrase)
        .map_err(|e| ShellDeckError::Keychain(format!("Failed to store passphrase: {}", e)))?;
    debug!("Stored passphrase for key {}", key_path);
    Ok(())
}

/// Retrieve a private key passphrase from the OS keychain. Returns None if not found.
pub fn get_key_passphrase(key_path: &str) -> Result<Option<String>> {
    let key = passphrase_entry_key(key_path);
    let entry = keyring::Entry::new(SERVICE_NAME, &key)
        .map_err(|e| ShellDeckError::Keychain(format!("Failed to create keyring entry: {}", e)))?;
    match entry.get_password() {
        Ok(passphrase) => {
            debug!("Retrieved passphrase for key {}", key_path);
            Ok(Some(passphrase))
        }
        Err(keyring::Error::NoEntry) => {
            debug!("No passphrase stored for key {}", key_path);
            Ok(None)
        }
        Err(e) => Err(ShellDeckError::Keychain(format!(
            "Failed to retrieve passphrase: {}",
            e
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-124 — key namespace: password entries and passphrase
    // entries MUST NOT collide even when the inputs look similar. The
    // password key is `user@host` and the passphrase key is
    // `passphrase:<path>` — the `passphrase:` prefix is the load-
    // bearing separator. A user with `alice@server.com` and an SSH key
    // at `/home/passphrase:server.com` would collide if the prefix
    // ever gets dropped.
    #[test]
    fn password_and_passphrase_key_namespaces_do_not_collide() {
        // Contrived worst-case: an SSH key path that literally spells
        // out the shape of the password key.
        let pw_key = entry_key("host.example", "user");
        let pph_key = passphrase_entry_key("user@host.example");
        assert_ne!(
            pw_key, pph_key,
            "password / passphrase namespaces must never produce identical entry keys",
        );
        assert!(
            pph_key.starts_with("passphrase:"),
            "passphrase key must be prefixed for the namespace to hold: {pph_key:?}",
        );
    }

    // Documented shape of the password entry key (`user@host`) — used
    // as the readable service label in the OS keychain UI (Secret
    // Service on Linux, Keychain on macOS, Credential Manager on
    // Windows). A rename breaks upgrades (existing entries stay
    // orphaned in the keychain under the old label).
    #[test]
    fn entry_key_is_user_at_host() {
        assert_eq!(entry_key("server.example", "alice"), "alice@server.example");
        // Non-standard characters survive verbatim (the keyring layer
        // handles escaping; our layer is transparent).
        assert_eq!(entry_key("h", "a b"), "a b@h");
    }

    #[test]
    fn passphrase_key_carries_prefix_and_path() {
        assert_eq!(
            passphrase_entry_key("/home/alice/.ssh/id_ed25519"),
            "passphrase:/home/alice/.ssh/id_ed25519",
        );
    }

    // ── SDTEST-120/123 — live keychain smoke (opt-in) ──────────────
    //
    // Gated by `SHELLDECK_LIVE_KEYCHAIN=1` so `cargo test` in CI (or
    // on a headless dev box without a running Secret Service) doesn't
    // touch the real OS keychain. Locally with a Secret Service +
    // unlocked session, run:
    //   SHELLDECK_LIVE_KEYCHAIN=1 cargo test -p shelldeck-core \
    //     -- --ignored keychain::tests::live_
    //
    // Uses a randomised host string so tests never step on a
    // pre-existing entry, and cleans up in a scope guard.

    fn live_gate() -> bool {
        std::env::var("SHELLDECK_LIVE_KEYCHAIN").ok().as_deref() == Some("1")
    }

    fn unique_host() -> String {
        format!(
            "shelldeck-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        )
    }

    // SDTEST-120 — store / get / delete round-trip.
    #[test]
    #[ignore = "requires SHELLDECK_LIVE_KEYCHAIN=1 + a running Secret Service (Linux)"]
    fn live_password_round_trip() {
        if !live_gate() {
            eprintln!("skipped: SHELLDECK_LIVE_KEYCHAIN not set");
            return;
        }
        let host = unique_host();
        let user = "shelldeck-test-user";
        let password = "hunter2-live-round-trip";

        store_password(&host, user, password).expect("store");
        let got = get_password(&host, user).expect("get returns Result");
        assert_eq!(got.as_deref(), Some(password));

        delete_password(&host, user).expect("delete");
        let after = get_password(&host, user).expect("get post-delete Result");
        assert!(after.is_none(), "entry must be gone after delete");
    }

    // SDTEST-123 — get_password returns Ok(None) for a missing entry,
    // NOT Err (consumers rely on the distinction to choose between
    // "prompt user" and "surface error toast").
    #[test]
    #[ignore = "requires SHELLDECK_LIVE_KEYCHAIN=1 + a running Secret Service (Linux)"]
    fn live_get_password_none_for_missing_entry() {
        if !live_gate() {
            return;
        }
        let host = unique_host();
        let got = get_password(&host, "definitely-not-there").expect("Ok even when missing");
        assert!(got.is_none());
    }

    // delete_password on a missing entry is a no-op (Ok(())) — matches
    // the impl comment. Consumers `delete_password` on logout without
    // caring whether the entry existed.
    #[test]
    #[ignore = "requires SHELLDECK_LIVE_KEYCHAIN=1"]
    fn live_delete_password_missing_entry_is_ok() {
        if !live_gate() {
            return;
        }
        let host = unique_host();
        delete_password(&host, "never-stored")
            .expect("delete on missing entry must be Ok, not Err");
    }
}
