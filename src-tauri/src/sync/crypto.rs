//! End-to-end encryption envelope for sync records.
//!
//! Wire-level format (frozen target; magic+version are stable):
//!
//! ```text
//! magic       : "TBSY"        (4 B)
//! version     : u8            (currently 1)
//! kdf_salt    : [u8; 16]
//! payload_nonce: [u8; 12]
//! wrapped_dek : [u8; 48]   // ChaCha20-Poly1305(KEK, zero-nonce)(DEK) || tag
//! ciphertext  : ...        // ChaCha20-Poly1305(DEK, payload_nonce)(plain, AAD = header)
//! tag         : [u8; 16]   // appended by AEAD
//! ```
//!
//! `AAD` for the payload AEAD is the entire byte range from `magic` through
//! `wrapped_dek` (so any tampering with the header invalidates the tag).
//!
//! KEK is derived per-envelope via Argon2id with a fresh `kdf_salt`. Because
//! the KEK is therefore unique per envelope, wrapping the DEK with a constant
//! zero nonce is safe (no nonce reuse across envelopes).

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    ChaCha20Poly1305, Key, Nonce,
};
use rand::{rngs::OsRng, RngCore};
use zeroize::Zeroize;

pub const MAGIC: &[u8; 4] = b"TBSY";
pub const VERSION: u8 = 1;
pub const SALT_LEN: usize = 16;
pub const NONCE_LEN: usize = 12;
pub const DEK_LEN: usize = 32;
pub const WRAPPED_DEK_LEN: usize = DEK_LEN + 16; // 16B Poly1305 tag
pub const HEADER_LEN: usize = 4 + 1 + SALT_LEN + NONCE_LEN + WRAPPED_DEK_LEN;

/// Argon2id parameters. Defaults align with docs/sync-protocol.md
/// (m=64 MiB, t=3, p=1). Tests may override via [`KdfParams::test_cheap`].
#[derive(Debug, Clone, Copy)]
pub struct KdfParams {
    pub mem_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl KdfParams {
    pub const DEFAULT: Self = Self {
        mem_kib: 64 * 1024,
        iterations: 3,
        parallelism: 1,
    };

    /// Cheap params for unit tests so they finish in milliseconds.
    pub const fn test_cheap() -> Self {
        Self {
            mem_kib: 8,
            iterations: 1,
            parallelism: 1,
        }
    }
}

impl Default for KdfParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("invalid envelope")]
    InvalidEnvelope,
    #[error("unsupported version: {0}")]
    UnsupportedVersion(u8),
    #[error("decryption failed")]
    DecryptionFailed,
    #[error("kdf failure: {0}")]
    KdfFailure(&'static str),
}

/// Derives the KEK from a master password using Argon2id.
pub fn derive_kek(
    password: &[u8],
    salt: &[u8; SALT_LEN],
    params: KdfParams,
) -> Result<[u8; 32], CryptoError> {
    let p = Params::new(
        params.mem_kib,
        params.iterations,
        params.parallelism,
        Some(32),
    )
    .map_err(|_| CryptoError::KdfFailure("invalid params"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, p);
    let mut out = [0u8; 32];
    argon
        .hash_password_into(password, salt, &mut out)
        .map_err(|_| CryptoError::KdfFailure("hash_password_into"))?;
    Ok(out)
}

/// Encrypts `plaintext` into a self-contained envelope.
pub fn encrypt(
    password: &[u8],
    plaintext: &[u8],
    params: KdfParams,
) -> Result<Vec<u8>, CryptoError> {
    let mut salt = [0u8; SALT_LEN];
    OsRng.fill_bytes(&mut salt);
    let mut payload_nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut payload_nonce);
    let mut dek = [0u8; DEK_LEN];
    OsRng.fill_bytes(&mut dek);

    let mut kek = derive_kek(password, &salt, params)?;

    // Wrap DEK with KEK using zero nonce (safe: KEK is unique per salt).
    let kek_cipher = ChaCha20Poly1305::new(Key::from_slice(&kek));
    let wrap_nonce = Nonce::from_slice(&[0u8; NONCE_LEN]);
    let wrapped_dek = kek_cipher
        .encrypt(wrap_nonce, dek.as_ref())
        .map_err(|_| CryptoError::KdfFailure("wrap"))?;
    debug_assert_eq!(wrapped_dek.len(), WRAPPED_DEK_LEN);

    // Build header (used as AAD for payload).
    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(MAGIC);
    header.push(VERSION);
    header.extend_from_slice(&salt);
    header.extend_from_slice(&payload_nonce);
    header.extend_from_slice(&wrapped_dek);

    // Encrypt payload with DEK + AAD.
    let dek_cipher = ChaCha20Poly1305::new(Key::from_slice(&dek));
    let ciphertext = dek_cipher
        .encrypt(
            Nonce::from_slice(&payload_nonce),
            Payload {
                msg: plaintext,
                aad: &header,
            },
        )
        .map_err(|_| CryptoError::KdfFailure("seal"))?;

    kek.zeroize();
    dek.zeroize();

    let mut out = header;
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypts an envelope. Returns the plaintext.
pub fn decrypt(
    password: &[u8],
    envelope: &[u8],
    params: KdfParams,
) -> Result<Vec<u8>, CryptoError> {
    if envelope.len() < HEADER_LEN + 16 {
        return Err(CryptoError::InvalidEnvelope);
    }
    if &envelope[0..4] != MAGIC {
        return Err(CryptoError::InvalidEnvelope);
    }
    let version = envelope[4];
    if version != VERSION {
        return Err(CryptoError::UnsupportedVersion(version));
    }
    let salt: [u8; SALT_LEN] = envelope[5..5 + SALT_LEN]
        .try_into()
        .map_err(|_| CryptoError::InvalidEnvelope)?;
    let nonce_start = 5 + SALT_LEN;
    let payload_nonce: [u8; NONCE_LEN] = envelope[nonce_start..nonce_start + NONCE_LEN]
        .try_into()
        .map_err(|_| CryptoError::InvalidEnvelope)?;
    let wrap_start = nonce_start + NONCE_LEN;
    let wrapped_dek = &envelope[wrap_start..wrap_start + WRAPPED_DEK_LEN];
    let header = &envelope[..HEADER_LEN];
    let ciphertext = &envelope[HEADER_LEN..];

    let mut kek = derive_kek(password, &salt, params)?;
    let kek_cipher = ChaCha20Poly1305::new(Key::from_slice(&kek));
    let wrap_nonce = Nonce::from_slice(&[0u8; NONCE_LEN]);
    let mut dek_vec = kek_cipher
        .decrypt(wrap_nonce, wrapped_dek)
        .map_err(|_| CryptoError::DecryptionFailed)?;
    if dek_vec.len() != DEK_LEN {
        kek.zeroize();
        dek_vec.zeroize();
        return Err(CryptoError::InvalidEnvelope);
    }
    let dek_cipher = ChaCha20Poly1305::new(Key::from_slice(&dek_vec));
    let plaintext = dek_cipher
        .decrypt(
            Nonce::from_slice(&payload_nonce),
            Payload {
                msg: ciphertext,
                aad: header,
            },
        )
        .map_err(|_| CryptoError::DecryptionFailed)?;

    kek.zeroize();
    dek_vec.zeroize();
    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> KdfParams {
        KdfParams::test_cheap()
    }

    #[test]
    fn roundtrip() {
        let env = encrypt(b"hunter2", b"hello tabby", p()).unwrap();
        let pt = decrypt(b"hunter2", &env, p()).unwrap();
        assert_eq!(pt, b"hello tabby");
    }

    #[test]
    fn roundtrip_empty_payload() {
        let env = encrypt(b"pw", b"", p()).unwrap();
        let pt = decrypt(b"pw", &env, p()).unwrap();
        assert_eq!(pt, b"");
    }

    #[test]
    fn wrong_password_fails() {
        let env = encrypt(b"correct", b"secret", p()).unwrap();
        let err = decrypt(b"wrong", &env, p()).unwrap_err();
        assert!(matches!(err, CryptoError::DecryptionFailed));
    }

    #[test]
    fn tamper_header_detected() {
        let mut env = encrypt(b"pw", b"payload", p()).unwrap();
        // Flip a byte inside the salt region.
        env[6] ^= 0x01;
        assert!(matches!(
            decrypt(b"pw", &env, p()).unwrap_err(),
            // Either KDF derives a different KEK (wrap fails) or AAD mismatch.
            CryptoError::DecryptionFailed
        ));
    }

    #[test]
    fn tamper_ciphertext_detected() {
        let mut env = encrypt(b"pw", b"payload-data", p()).unwrap();
        let last = env.len() - 1;
        env[last] ^= 0xff;
        assert!(matches!(
            decrypt(b"pw", &env, p()).unwrap_err(),
            CryptoError::DecryptionFailed
        ));
    }

    #[test]
    fn bad_magic_rejected() {
        let mut env = encrypt(b"pw", b"x", p()).unwrap();
        env[0] = b'X';
        assert!(matches!(
            decrypt(b"pw", &env, p()).unwrap_err(),
            CryptoError::InvalidEnvelope
        ));
    }

    #[test]
    fn unknown_version_rejected() {
        let mut env = encrypt(b"pw", b"x", p()).unwrap();
        env[4] = 0xff;
        assert!(matches!(
            decrypt(b"pw", &env, p()).unwrap_err(),
            CryptoError::UnsupportedVersion(0xff)
        ));
    }

    #[test]
    fn unique_envelopes_for_same_input() {
        let a = encrypt(b"pw", b"same", p()).unwrap();
        let b = encrypt(b"pw", b"same", p()).unwrap();
        assert_ne!(a, b, "salt/nonce/DEK randomness must differ");
    }
}
