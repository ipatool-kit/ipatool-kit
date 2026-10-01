//! Device identity helpers.

/// True for addresses that must never be sent to Apple.
pub fn is_placeholder_mac(mac: Option<&[u8]>) -> bool {
    let Some(m) = mac else {
        return true;
    };
    if m.len() != 6 {
        return true;
    }
    let all_zero = m.iter().all(|&b| b == 0);
    let all_ff = m.iter().all(|&b| b == 0xff);
    if all_zero || all_ff {
        return true;
    }
    if m == [0x02, 0, 0, 0, 0, 0] {
        return true;
    }
    if m == [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF] {
        return true;
    }
    // multicast bit
    if m[0] & 0x01 != 0 {
        return true;
    }
    false
}
