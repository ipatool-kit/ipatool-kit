//! Minimal Apple plist (XML + binary) encoder/decoder.

use std::collections::BTreeMap;

use base64::{engine::general_purpose::STANDARD as B64, Engine};

use crate::error::{IpatoolError, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum PlistValue {
    String(String),
    Integer(i64),
    Real(f64),
    Bool(bool),
    Data(Vec<u8>),
    Date(String),
    Dict(PlistDict),
    Array(Vec<PlistValue>),
    Null,
}

pub type PlistDict = BTreeMap<String, PlistValue>;

pub fn base64_encode(data: &[u8]) -> String {
    B64.encode(data)
}

pub fn base64_decode(s: &str) -> Vec<u8> {
    // Match C++: skip invalid chars, stop at '='
    let mut idx = [-1i8; 256];
    for (i, c) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
        .iter()
        .enumerate()
    {
        idx[*c as usize] = i as i8;
    }
    let mut out = Vec::new();
    let mut bits: u32 = 0;
    let mut nbits = 0;
    for &c in s.as_bytes() {
        if c == b'=' {
            break;
        }
        let v = idx[c as usize];
        if v < 0 {
            continue;
        }
        bits = (bits << 6) | (v as u32);
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((bits >> nbits) as u8);
            bits &= (1u32 << nbits) - 1;
        }
    }
    out
}

fn xml_unescape(s: &str) -> String {
    let out = s
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'");
    // numeric character references
    let bytes = out.as_bytes();
    let mut res = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' && i + 2 < bytes.len() && bytes[i + 1] == b'#' {
            if let Some(semi) = bytes[i + 2..].iter().position(|&c| c == b';') {
                let semi = i + 2 + semi;
                let body = &out[i + 2..semi];
                let cp =
                    if let Some(hex) = body.strip_prefix('x').or_else(|| body.strip_prefix('X')) {
                        u32::from_str_radix(hex, 16).ok()
                    } else {
                        body.parse::<u32>().ok()
                    };
                if let Some(cp) = cp {
                    if let Some(ch) = char::from_u32(cp) {
                        res.push(ch);
                        i = semi + 1;
                        continue;
                    }
                }
            }
        }
        res.push(out[i..].chars().next().unwrap());
        i += out[i..].chars().next().unwrap().len_utf8();
    }
    res.replace("&amp;", "&")
}

pub fn decode_plist(src: &str) -> PlistDict {
    let bytes = src.as_bytes();
    if bytes.len() >= 8 && &bytes[..8] == b"bplist00" {
        return decode_bplist(bytes).unwrap_or_default();
    }
    decode_plist_xml(src).unwrap_or_default()
}

pub fn decode_plist_bytes(data: &[u8]) -> PlistDict {
    if data.len() >= 8 && &data[..8] == b"bplist00" {
        return decode_bplist(data).unwrap_or_default();
    }
    match std::str::from_utf8(data) {
        Ok(s) => decode_plist(s),
        Err(_) => PlistDict::new(),
    }
}

fn decode_bplist(data: &[u8]) -> Result<PlistDict> {
    let value: plist::Value =
        plist::from_bytes(data).map_err(|e| IpatoolError::Plist(e.to_string()))?;
    match plist_crate_to_ours(&value) {
        PlistValue::Dict(d) => Ok(d),
        _ => Ok(PlistDict::new()),
    }
}

fn plist_crate_to_ours(v: &plist::Value) -> PlistValue {
    match v {
        plist::Value::String(s) => PlistValue::String(s.clone()),
        plist::Value::Integer(i) => PlistValue::Integer(i.as_signed().unwrap_or(0)),
        plist::Value::Real(r) => PlistValue::Real(*r),
        plist::Value::Boolean(b) => PlistValue::Bool(*b),
        plist::Value::Data(d) => PlistValue::Data(d.clone()),
        plist::Value::Date(d) => PlistValue::Date(d.to_xml_format()),
        plist::Value::Array(a) => PlistValue::Array(a.iter().map(plist_crate_to_ours).collect()),
        plist::Value::Dictionary(d) => {
            let mut out = PlistDict::new();
            for (k, v) in d {
                out.insert(k.clone(), plist_crate_to_ours(v));
            }
            PlistValue::Dict(out)
        }
        _ => PlistValue::Null,
    }
}

fn decode_plist_xml(src: &str) -> Result<PlistDict> {
    // Find root <dict>...</dict>
    let start = src.find("<dict>").or_else(|| src.find("<dict "));
    let end = src.rfind("</dict>");
    let (Some(start), Some(end)) = (start, end) else {
        return Err(IpatoolError::Plist("no dict".into()));
    };
    let inner = &src[start..end + "</dict>".len()];
    match parse_value(inner)? {
        PlistValue::Dict(d) => Ok(d),
        _ => Err(IpatoolError::Plist("root not dict".into())),
    }
}

fn parse_value(s: &str) -> Result<PlistValue> {
    let s = s.trim();
    if s.starts_with("<true") {
        return Ok(PlistValue::Bool(true));
    }
    if s.starts_with("<false") {
        return Ok(PlistValue::Bool(false));
    }
    if let Some(rest) = s.strip_prefix("<string>") {
        let end = rest
            .find("</string>")
            .ok_or_else(|| IpatoolError::Plist("string".into()))?;
        return Ok(PlistValue::String(xml_unescape(&rest[..end])));
    }
    if s.starts_with("<string/>") || s.starts_with("<string />") {
        return Ok(PlistValue::String(String::new()));
    }
    if let Some(rest) = s.strip_prefix("<integer>") {
        let end = rest
            .find("</integer>")
            .ok_or_else(|| IpatoolError::Plist("int".into()))?;
        let n: i64 = rest[..end].trim().parse().unwrap_or(0);
        return Ok(PlistValue::Integer(n));
    }
    if let Some(rest) = s.strip_prefix("<real>") {
        let end = rest
            .find("</real>")
            .ok_or_else(|| IpatoolError::Plist("real".into()))?;
        let n: f64 = rest[..end].trim().parse().unwrap_or(0.0);
        return Ok(PlistValue::Real(n));
    }
    if let Some(rest) = s.strip_prefix("<data>") {
        let end = rest
            .find("</data>")
            .ok_or_else(|| IpatoolError::Plist("data".into()))?;
        return Ok(PlistValue::Data(base64_decode(&rest[..end])));
    }
    if let Some(rest) = s.strip_prefix("<date>") {
        let end = rest
            .find("</date>")
            .ok_or_else(|| IpatoolError::Plist("date".into()))?;
        return Ok(PlistValue::Date(xml_unescape(&rest[..end])));
    }
    if s.starts_with("<dict>") {
        return Ok(PlistValue::Dict(parse_dict(s)?));
    }
    if s.starts_with("<array>") {
        return Ok(PlistValue::Array(parse_array(s)?));
    }
    Err(IpatoolError::Plist(format!(
        "unknown value: {}",
        &s[..s.len().min(40)]
    )))
}

fn parse_dict(s: &str) -> Result<PlistDict> {
    let inner = strip_tag(s, "dict")?;
    let mut d = PlistDict::new();
    let mut rest = inner.as_str();
    while let Some(ks) = rest.find("<key>") {
        rest = &rest[ks + 5..];
        let ke = rest
            .find("</key>")
            .ok_or_else(|| IpatoolError::Plist("key".into()))?;
        let key = xml_unescape(rest[..ke].trim());
        rest = rest[ke + 6..].trim_start();
        let (val, consumed) = take_one_value(rest)?;
        d.insert(key, val);
        rest = &rest[consumed..];
    }
    Ok(d)
}

fn parse_array(s: &str) -> Result<Vec<PlistValue>> {
    let inner = strip_tag(s, "array")?;
    let mut out = Vec::new();
    let mut rest = inner.trim();
    while !rest.is_empty() && rest.starts_with('<') {
        let (val, consumed) = take_one_value(rest)?;
        out.push(val);
        rest = rest[consumed..].trim_start();
    }
    Ok(out)
}

fn strip_tag(s: &str, tag: &str) -> Result<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let s = s.trim();
    if !s.starts_with(&open) {
        return Err(IpatoolError::Plist(format!("expected {open}")));
    }
    if !s.ends_with(&close) {
        return Err(IpatoolError::Plist(format!("expected {close}")));
    }
    Ok(s[open.len()..s.len() - close.len()].to_string())
}

fn take_one_value(s: &str) -> Result<(PlistValue, usize)> {
    let s = s.trim_start();
    // self-closing leaves / empty containers
    for tag in [
        "<true/>",
        "<true />",
        "<false/>",
        "<false />",
        "<string/>",
        "<string />",
    ] {
        if s.starts_with(tag) {
            let v = parse_value(tag)?;
            return Ok((v, tag.len()));
        }
    }
    for (tag, v) in [
        ("<dict/>", PlistValue::Dict(PlistDict::new())),
        ("<dict />", PlistValue::Dict(PlistDict::new())),
        ("<array/>", PlistValue::Array(Vec::new())),
        ("<array />", PlistValue::Array(Vec::new())),
        ("<data/>", PlistValue::Data(Vec::new())),
        ("<data />", PlistValue::Data(Vec::new())),
    ] {
        if s.starts_with(tag) {
            return Ok((v, tag.len()));
        }
    }
    // find matching close for dict/array or simple tags
    if s.starts_with("<dict>") {
        let end = find_matching(s, "dict")?;
        let chunk = &s[..end];
        return Ok((parse_value(chunk)?, end));
    }
    if s.starts_with("<array>") {
        let end = find_matching(s, "array")?;
        let chunk = &s[..end];
        return Ok((parse_value(chunk)?, end));
    }
    for tag in ["string", "integer", "real", "data", "date"] {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        if s.starts_with(&open) {
            if let Some(rel) = s[open.len()..].find(&close) {
                let end = open.len() + rel + close.len();
                return Ok((parse_value(&s[..end])?, end));
            }
        }
    }
    Err(IpatoolError::Plist(format!(
        "cannot parse value at {}",
        &s[..s.len().min(40)]
    )))
}

fn find_matching(s: &str, tag: &str) -> Result<usize> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < s.len() {
        if s[i..].starts_with(&open) {
            depth += 1;
            i += open.len();
            continue;
        }
        if s[i..].starts_with(&close) {
            depth -= 1;
            i += close.len();
            if depth == 0 {
                return Ok(i);
            }
            continue;
        }
        i += s[i..].chars().next().unwrap().len_utf8();
    }
    Err(IpatoolError::Plist(format!("unclosed <{tag}>")))
}

pub fn dict_str(d: &PlistDict, key: &str) -> String {
    match d.get(key) {
        Some(PlistValue::String(s)) => s.clone(),
        Some(PlistValue::Date(s)) => s.clone(),
        _ => String::new(),
    }
}

