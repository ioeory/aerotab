//! Resolve [`AuthMethod::VaultRef`] into inline credentials using the vault.

use crate::ssh::SshError;
use crate::ssh::{AuthMethod, SshProfile};
use crate::vault::{EntryKind, VaultError, VaultStore};

/// Replace vault references in `profile` (including `jump_via`) with concrete auth.
pub async fn resolve_profile_vault_auth(
    vault: Option<&VaultStore>,
    profile: &mut SshProfile,
) -> Result<(), SshError> {
    resolve_auth(vault, &mut profile.auth).await?;
    resolve_jump_vault_auth(vault, &mut profile.jump_via).await
}

async fn resolve_jump_vault_auth(
    vault: Option<&VaultStore>,
    hops: &mut [SshProfile],
) -> Result<(), SshError> {
    for hop in hops {
        resolve_auth(vault, &mut hop.auth).await?;
        if !hop.jump_via.is_empty() {
            Box::pin(resolve_jump_vault_auth(vault, &mut hop.jump_via)).await?;
        }
    }
    Ok(())
}

async fn resolve_auth(vault: Option<&VaultStore>, auth: &mut AuthMethod) -> Result<(), SshError> {
    let AuthMethod::VaultRef {
        entry_id,
        passphrase_entry_id,
    } = auth.clone()
    else {
        return Ok(());
    };

    let store = vault.ok_or_else(|| {
        SshError::Connect("vault is not configured; unlock Vault in settings".into())
    })?;
    if !store.is_unlocked().await {
        return Err(SshError::Connect(
            "vault is locked; unlock Vault before connecting".into(),
        ));
    }

    let entry = store.get(&entry_id).await.map_err(vault_err)?;

    let passphrase = if let Some(pid) = passphrase_entry_id.as_ref() {
        Some(store.get(pid).await.map_err(vault_err)?.secret)
    } else {
        None
    };

    *auth = match entry.kind {
        EntryKind::Password | EntryKind::Token => AuthMethod::Password {
            secret: entry.secret,
        },
        EntryKind::Key => AuthMethod::PrivateKeyMemory {
            pem: normalize_vault_private_key(&entry.secret),
            passphrase,
        },
        EntryKind::Note => {
            return Err(SshError::Connect(format!(
                "vault entry \"{}\" is a note, not usable for SSH auth",
                entry.label
            )));
        }
    };
    Ok(())
}

fn vault_err(e: VaultError) -> SshError {
    match e {
        VaultError::Locked => SshError::Connect("vault is locked".into()),
        VaultError::NotFound(id) => SshError::Connect(format!("vault entry not found: {id}")),
        other => SshError::Connect(other.to_string()),
    }
}

/// Normalize private key text from vault storage (line endings, escaped newlines).
fn normalize_vault_private_key(raw: &str) -> String {
    let trimmed = raw.trim();
    let unescaped = if trimmed.contains("\\n") && !trimmed.contains('\n') {
        trimmed.replace("\\n", "\n")
    } else {
        trimmed.to_string()
    };
    let normalized = unescaped.replace("\r\n", "\n").replace('\r', "\n");
    if normalized.ends_with('\n') {
        normalized
    } else {
        format!("{normalized}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_vault_private_key_unescapes_literal_newlines() {
        let raw = "-----BEGIN OPENSSH PRIVATE KEY-----\\nLINE\\n-----END OPENSSH PRIVATE KEY-----";
        let out = normalize_vault_private_key(raw);
        assert!(out.contains('\n'));
        assert!(!out.contains("\\n"));
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn normalize_vault_private_key_preserves_real_newlines() {
        let raw = "-----BEGIN OPENSSH PRIVATE KEY-----\nLINE\n-----END OPENSSH PRIVATE KEY-----\n";
        let out = normalize_vault_private_key(raw);
        assert_eq!(out.matches('\n').count(), 3);
    }

    #[test]
    fn decodes_in_memory_key_with_passphrase() {
        let key = russh::keys::key::KeyPair::generate_ed25519().expect("ed25519 key");
        let mut pem = Vec::new();
        russh::keys::encode_pkcs8_pem_encrypted(&key, b"s3cret", 16, &mut pem).expect("encode");
        let pem = normalize_vault_private_key(&String::from_utf8(pem).expect("utf8"));
        assert!(russh::keys::decode_secret_key(&pem, Some("s3cret")).is_ok());
        assert!(russh::keys::decode_secret_key(&pem, Some("wrong")).is_err());
    }

    #[tokio::test]
    async fn resolve_without_vault_reference_writes_nothing() {
        let mut auth = AuthMethod::Agent;
        resolve_auth(None, &mut auth).await.expect("noop");
        assert!(matches!(auth, AuthMethod::Agent));
    }

    #[test]
    fn in_memory_key_is_never_serialized() {
        let auth = AuthMethod::PrivateKeyMemory {
            pem: "secret".into(),
            passphrase: None,
        };
        assert!(serde_json::to_string(&auth).is_err());
    }

    #[test]
    fn normalize_vault_private_key_normalizes_crlf() {
        let raw = "-----BEGIN RSA PRIVATE KEY-----\r\nBODY\r\n-----END RSA PRIVATE KEY-----";
        let out = normalize_vault_private_key(raw);
        assert!(!out.contains('\r'));
        assert!(out.ends_with('\n'));
    }
}
