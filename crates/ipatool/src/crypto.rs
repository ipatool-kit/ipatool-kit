//! SHA-256, HMAC-SHA256, PBKDF2-HMAC-SHA256, AES-GCM (pure Rust via `aes-gcm`).

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes128Gcm, Aes256Gcm, Key, KeyInit, Nonce};
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use sha2::{Digest, Sha256};

use crate::error::{IpatoolError, Result};

type HmacSha256 = Hmac<Sha256>;

pub fn digest(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().into()
}

pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

pub fn pbkdf2(password: &[u8], salt: &[u8], iterations: u32, keylen: usize) -> Vec<u8> {
    let mut out = vec![0u8; keylen];
    pbkdf2_hmac::<Sha256>(password, salt, iterations, &mut out);
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcmOutput {
    pub ciphertext: Vec<u8>,
    pub tag: Vec<u8>,
}

pub fn gcm_encrypt(key: &[u8], iv: &[u8], plaintext: &[u8], aad: &[u8]) -> Result<GcmOutput> {
    if iv.len() != 12 {
        return Err(IpatoolError::msg("AES-GCM IV must be 12 bytes"));
    }
    let nonce = Nonce::from_slice(iv);
    let payload = Payload {
        msg: plaintext,
        aad,
    };
    let ct_tag = match key.len() {
        16 => Aes128Gcm::new(Key::<Aes128Gcm>::from_slice(key)).encrypt(nonce, payload),
        32 => Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key)).encrypt(nonce, payload),
        _ => return Err(IpatoolError::msg("AES-GCM key must be 16 or 32 bytes")),
    }
    .map_err(|_| IpatoolError::msg("AES-GCM encrypt failed"))?;

    if ct_tag.len() < 16 {
        return Err(IpatoolError::msg("AES-GCM encrypt produced short output"));
    }
    let (ct, tag) = ct_tag.split_at(ct_tag.len() - 16);
    Ok(GcmOutput {
        ciphertext: ct.to_vec(),
        tag: tag.to_vec(),
    })
}

pub fn gcm_decrypt(
    key: &[u8],
    iv: &[u8],
    ciphertext: &[u8],
    tag: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>> {
    if tag.is_empty() || tag.len() != 16 {
        return Err(IpatoolError::AuthTagMismatch);
    }
    if iv.len() != 12 {
        return Err(IpatoolError::msg("AES-GCM IV must be 12 bytes"));
    }
    let nonce = Nonce::from_slice(iv);
    let mut combined = Vec::with_capacity(ciphertext.len() + 16);
    combined.extend_from_slice(ciphertext);
    combined.extend_from_slice(tag);
    let payload = Payload {
        msg: combined.as_slice(),
        aad,
    };
    match key.len() {
        16 => Aes128Gcm::new(Key::<Aes128Gcm>::from_slice(key))
            .decrypt(nonce, payload)
            .map_err(|_| IpatoolError::AuthTagMismatch),
        32 => Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key))
            .decrypt(nonce, payload)
            .map_err(|_| IpatoolError::AuthTagMismatch),
        _ => Err(IpatoolError::msg("AES-GCM key must be 16 or 32 bytes")),
    }
}
