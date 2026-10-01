//! Machine-bound file key derivation.

use crate::crypto;

pub const FILE_KEY_SALT: &str = "nice_key_is_nice";

/// PBKDF2-SHA256(machine_id + FILE_KEY_SALT + passphrase, salt, 100000, 32)
pub fn derive_key_from_machine(machine_id: &str, passphrase: &str, salt: &[u8]) -> [u8; 32] {
    let material = format!("{machine_id}{FILE_KEY_SALT}{passphrase}");
    let key = crypto::pbkdf2(material.as_bytes(), salt, 100_000, 32);
    let mut out = [0u8; 32];
    out.copy_from_slice(&key);
    out
}
