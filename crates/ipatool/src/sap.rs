//! SAP wire helpers (no Unicorn).

use crate::error::{IpatoolError, Result};
use crate::plist::{self, base64_decode, base64_encode};

/// Parse `"AA:BB:CC:DD:EE:FF"` or dash-separated MAC into bytes.
pub fn hardware_id_from_mac(mac: &str) -> Result<Vec<u8>> {
    let mut result = Vec::with_capacity(6);
    let bytes = mac.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i] == b':' || bytes[i] == b'-') {
            i += 1;
        }
        if i + 1 >= bytes.len() {
            break;
        }
        let hi = hex_nibble(bytes[i]);
        let lo = hex_nibble(bytes[i + 1]);
        if hi < 0 || lo < 0 {
            break;
        }
        result.push(((hi as u8) << 4) | (lo as u8));
        i += 2;
    }
    if result.is_empty() || result.len() > 20 {
        return Err(IpatoolError::InvalidMac(mac.to_string()));
    }
    Ok(result)
}

fn hex_nibble(c: u8) -> i32 {
    match c {
        b'0'..=b'9' => (c - b'0') as i32,
        b'A'..=b'F' => (c - b'A' + 10) as i32,
        b'a'..=b'f' => (c - b'a' + 10) as i32,
        _ => -1,
    }
}

/// 24-byte length-prefixed hardware block (host-endian uint32 + id + zero pad).
/// Mirrors private `SapMachine::HardwareBlock` / `StoreAgentMachine::HardwareBlock`.
pub fn hardware_block(id: &[u8]) -> Result<[u8; 24]> {
    if id.is_empty() || id.len() > 20 {
        return Err(IpatoolError::InvalidHardwareId);
    }
    let mut block = [0u8; 24];
    let sz = id.len() as u32;
    block[..4].copy_from_slice(&sz.to_ne_bytes());
    block[4..4 + id.len()].copy_from_slice(id);
    Ok(block)
}

pub fn sap_base64_encode(data: &[u8]) -> String {
    // Same alphabet as plist base64 in this codebase.
    base64_encode(data)
}

pub fn sap_base64_decode(b64: &str) -> Vec<u8> {
    base64_decode(b64)
}

/// One-entry XML plist with a single `<data>` value (SapPlist::MakeData).
pub fn sap_plist_make_data(key: &str, value: &[u8]) -> Vec<u8> {
    let b64 = sap_base64_encode(value);
    let key = plist::xml_escape(key);
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
\t<key>{key}</key>\n\
\t<data>{b64}</data>\n\
</dict>\n\
</plist>\n"
    );
    xml.into_bytes()
}

pub fn sap_plist_extract_data(xml: &[u8], key: &str) -> Result<Vec<u8>> {
    let s = std::str::from_utf8(xml).map_err(|e| IpatoolError::Plist(e.to_string()))?;
    let key_tag = format!("<key>{}</key>", plist::xml_escape(key));
    let pos = s
        .find(&key_tag)
        .ok_or_else(|| IpatoolError::Plist(format!("missing key {key}")))?;
    let after = &s[pos + key_tag.len()..];
    let data_start = after
        .find("<data>")
        .ok_or_else(|| IpatoolError::Plist("missing data".into()))?;
    let rest = &after[data_start + 6..];
    let data_end = rest
        .find("</data>")
        .ok_or_else(|| IpatoolError::Plist("unclosed data".into()))?;
    Ok(base64_decode(&rest[..data_end]))
}
