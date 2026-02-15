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
