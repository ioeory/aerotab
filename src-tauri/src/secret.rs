//! OS keyring–backed master password store.
//!
//! The master password is the KEK seed for the sync envelope crypto.
//! Persisting it in plaintext (config file, sled, etc.) would defeat the
//! purpose of the E2E layer, so we delegate to the platform credential
//! store via the `keyring` crate:
//!
//! - Linux:   Secret Service (gnome-keyring, kwallet, …)
//! - macOS:   Keychain
//! - Windows: Credential Manager
//!
//! Tests use a per-test entry name and clean up after themselves.

use keyring::Entry;

const SERVICE: &str = "org.tabby.v2";
const DEFAULT_ACCOUNT: &str = "sync.master";

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("keyring: {0}")]
    Keyring(String),
    #[error("not found")]
    NotFound,
}

impl From<keyring::Error> for SecretError {
    fn from(e: keyring::Error) -> Self {
        match e {
            keyring::Error::NoEntry => SecretError::NotFound,
            other => SecretError::Keyring(other.to_string()),
        }
    }
}

fn entry(account: &str) -> Result<Entry, SecretError> {
    Entry::new(SERVICE, account).map_err(SecretError::from)
}

/// Stores (or replaces) the master password for `account`.
///
/// Passing an empty string is rejected so callers must call
/// [`clear_master`] explicitly to forget the secret.
pub fn set_master(account: Option<&str>, secret: &str) -> Result<(), SecretError> {
    if secret.is_empty() {
        return Err(SecretError::Keyring("empty secret".into()));
    }
    entry(account.unwrap_or(DEFAULT_ACCOUNT))?
        .set_password(secret)
        .map_err(SecretError::from)
}

/// Retrieves a previously stored master password.
pub fn get_master(account: Option<&str>) -> Result<String, SecretError> {
    entry(account.unwrap_or(DEFAULT_ACCOUNT))?
        .get_password()
        .map_err(SecretError::from)
}

/// Returns true if a secret exists for `account`.
pub fn has_master(account: Option<&str>) -> bool {
    matches!(get_master(account), Ok(s) if !s.is_empty())
}

/// Forgets the secret. Idempotent: returns `Ok(())` when nothing was stored.
pub fn clear_master(account: Option<&str>) -> Result<(), SecretError> {
    clear_secret(account)
}

/// Stores an arbitrary named secret (OAuth tokens, etc.).
pub fn set_secret(account: &str, secret: &str) -> Result<(), SecretError> {
    if secret.is_empty() {
        return Err(SecretError::Keyring("empty secret".into()));
    }
    entry(account)?.set_password(secret).map_err(SecretError::from)
}

pub fn get_secret(account: &str) -> Result<String, SecretError> {
    entry(account)?.get_password().map_err(SecretError::from)
}

pub fn has_secret(account: &str) -> bool {
    matches!(get_secret(account), Ok(s) if !s.is_empty())
}

pub fn clear_secret(account: Option<&str>) -> Result<(), SecretError> {
    let account = account.unwrap_or(DEFAULT_ACCOUNT);
    match entry(account)?.delete_password() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(SecretError::from(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The OS keyring may not be available in CI (no secret service). We
    // gate the integration tests behind an env-var so local runs still
    // exercise the API but CI doesn't fail on missing dbus.
    fn keyring_available() -> bool {
        // Cheap probe: try a non-destructive read on a throwaway account.
        Entry::new(SERVICE, "probe")
            .and_then(|e| match e.get_password() {
                Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(other) => Err(other),
            })
            .is_ok()
    }

    #[test]
    fn empty_secret_rejected() {
        if !keyring_available() {
            eprintln!("skipping: no keyring backend");
            return;
        }
        let err = set_master(Some("tabby-test-empty"), "").unwrap_err();
        assert!(matches!(err, SecretError::Keyring(_)));
    }

    #[test]
    fn set_get_clear_roundtrip() {
        if !keyring_available() {
            eprintln!("skipping: no keyring backend");
            return;
        }
        let acct = format!("tabby-test-{}", uuid::Uuid::new_v4());
        // Guarantee a clean slate
        let _ = clear_master(Some(&acct));

        set_master(Some(&acct), "s3cr3t").unwrap();
        assert!(has_master(Some(&acct)));
        assert_eq!(get_master(Some(&acct)).unwrap(), "s3cr3t");

        clear_master(Some(&acct)).unwrap();
        assert!(!has_master(Some(&acct)));
        // Idempotent
        clear_master(Some(&acct)).unwrap();
    }
}
