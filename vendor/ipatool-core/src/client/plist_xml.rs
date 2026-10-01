use crate::error::ClientError;

/// Match majd/ipatool `pkg/http/client.go` XML normalization:
/// Document wrapper → embedded plist → embedded dict → bare `<key>` wrap.
pub fn normalize_plist_xml(body: &[u8]) -> Vec<u8> {
    let mut normalized = trim_ascii(body).to_vec();
    if normalized.is_empty() {
        return normalized;
    }

    if let Some(inner) = extract_document_inner(&normalized) {
        normalized = inner;
    }

    if let Some(plist) = extract_embedded_plist(&normalized) {
        normalized = plist;
    }

    // Go always unwraps an embedded dict (including from inside <plist>) and
    // returns early so the bare-`<key>` path cannot re-wrap a full document.
    if let Some(dict) = extract_embedded_dict(&normalized) {
        return wrap_plist(&dict);
    }

    if bytes_contains_ci(&normalized, b"<key>") {
        let mut wrapped = b"<dict>".to_vec();
        wrapped.extend_from_slice(&normalized);
        wrapped.extend_from_slice(b"</dict>");
        return wrap_plist(&wrapped);
    }

    normalized
}

/// Go `looksLikePropertyList`: do not hand HTML/plaintext to the plist parser.
pub fn looks_like_property_list(body: &[u8]) -> bool {
    let trimmed = trim_ascii(body);
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.starts_with(b"bplist") {
        return true;
    }
    for marker in [b"<?xml" as &[u8], b"<plist", b"<dict", b"<key"] {
        if bytes_contains_ci(trimmed, marker) {
            return true;
        }
    }
    false
}

fn looks_like_html(body: &[u8]) -> bool {
    let t = trim_ascii(body);
    let lower = to_lower_prefix(t, 64);
    lower.starts_with(b"<!doctype html") || lower.starts_with(b"<html")
}

/// Dump the raw Apple body so the UI error can point at a full file (no clipping).
pub fn dump_response_body(body: &[u8]) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .map(|h| h.join(".ipatool").join("last-response-body.txt"))
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!(
                "ipatool-last-response-{}.txt",
                std::process::id()
            ))
        });
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&path, body).ok()?;
    Some(path)
}

fn html_title(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let start = lower.find("<title>")? + 7;
    let end = lower[start..].find("</title>")? + start;
    let title = text[start..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if title.is_empty() {
        None
    } else {
        Some(title)
    }
}

/// Readable text from HTML/plain: drop script/style, strip tags, keep real words.
fn visible_text(body: &[u8], max_chars: usize) -> String {
    let mut text = String::from_utf8_lossy(body).into_owned();
    for tag in ["script", "style"] {
        loop {
            let lower = text.to_ascii_lowercase();
            let open = format!("<{tag}");
            let close = format!("</{tag}>");
            let Some(s) = lower.find(&open) else { break };
            let Some(rel) = lower[s..].find(&close) else { break };
            let e = s + rel + close.len();
            text.replace_range(s..e, " ");
        }
    }

    let mut out = String::new();
    let mut in_tag = false;
    let mut prev_space = false;
    for c in text.chars() {
        if c == '<' {
            in_tag = true;
            continue;
        }
        if c == '>' {
            in_tag = false;
            if !prev_space && !out.is_empty() {
                out.push(' ');
                prev_space = true;
            }
            continue;
        }
        if in_tag {
            continue;
        }
        if c.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
                prev_space = true;
            }
            continue;
        }
        prev_space = false;
        out.push(c);
        if out.chars().count() >= max_chars {
            out.push('…');
            break;
        }
    }
    out.trim().to_string()
}

/// Full diagnostic for a non-plist Apple body. Writes the raw bytes to a temp file.
pub fn describe_non_plist(
    label: &str,
    url: &str,
    status: Option<u16>,
    content_type: Option<&str>,
    body: &[u8],
) -> String {
    let dump = dump_response_body(body)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(failed to write body dump)".into());

    let mut parts = Vec::new();
    parts.push(format!("{label}: Apple returned a non-plist response"));
    if let Some(s) = status {
        parts.push(format!("HTTP {s}"));
    }
    if !url.is_empty() {
        parts.push(format!("url={url}"));
    }
    if let Some(ct) = content_type.filter(|s| !s.is_empty()) {
        parts.push(format!("content-type={ct}"));
    }
    parts.push(format!("body-bytes={}", body.len()));
    parts.push(format!("full-body-file={dump}"));

    if body.is_empty() {
        parts.push("body is empty".into());
        return parts.join("\n");
    }

    let lossy = String::from_utf8_lossy(body);
    if looks_like_html(body) {
        if let Some(title) = html_title(&lossy) {
            parts.push(format!("html-title={title}"));
        } else {
            parts.push("body is HTML (no <title>)".into());
        }
    }

    let visible = visible_text(body, 4000);
    if !visible.is_empty() {
        parts.push(format!("body-text:\n{visible}"));
    } else {
        // Binary / undecodable — still show a hex/utf8 preview in the message.
        let preview = lossy.chars().take(2000).collect::<String>();
        parts.push(format!("body-raw-preview:\n{preview}"));
    }

    parts.join("\n")
}

pub fn parse_plist_response<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, ClientError> {
    parse_plist_http::<T>("response", "", None, None, body)
}

/// Prefer this at HTTP call sites so errors carry status + URL + full body dump.
pub fn parse_plist_http<T: serde::de::DeserializeOwned>(
    label: &str,
    url: &str,
    status: Option<u16>,
    content_type: Option<&str>,
    body: &[u8],
) -> Result<T, ClientError> {
    if looks_like_html(body) || !looks_like_property_list(body) {
        return Err(ClientError::UnexpectedResponse(describe_non_plist(
            label,
            url,
            status,
            content_type,
            body,
        )));
    }

    let normalized = normalize_plist_xml(body);
    if !looks_like_property_list(&normalized) {
        return Err(ClientError::UnexpectedResponse(describe_non_plist(
            label,
            url,
            status,
            content_type,
            body,
        )));
    }

    let cursor = std::io::Cursor::new(&normalized);
    plist::from_reader(cursor).map_err(|e| {
        ClientError::UnexpectedResponse(format!(
            "{}\nplist-parse-error={e}",
            describe_non_plist(label, url, status, content_type, body)
        ))
    })
}

fn wrap_plist(dict_or_plist: &[u8]) -> Vec<u8> {
    let t = trim_ascii(dict_or_plist);
    if bytes_contains_ci(t, b"<plist") {
        return t.to_vec();
    }
    let mut out = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n"
        .to_vec();
    out.extend_from_slice(t);
    out.extend_from_slice(b"\n</plist>");
    out
}

fn extract_document_inner(body: &[u8]) -> Option<Vec<u8>> {
    let lower = to_ascii_lower(body);
    let start = lower.windows(9).position(|w| w == b"<document")?;
    let after = start + body[start..].iter().position(|&b| b == b'>')? + 1;
    let end_rel = lower[after..]
        .windows(11)
        .position(|w| w == b"</document>")?;
    let inner = trim_ascii(&body[after..after + end_rel]);
    if inner.is_empty() {
        None
    } else {
        Some(inner.to_vec())
    }
}

fn extract_embedded_plist(body: &[u8]) -> Option<Vec<u8>> {
    let lower = to_ascii_lower(body);
    let start = lower.windows(6).position(|w| w == b"<plist")?;
    let end_rel = lower[start..]
        .windows(8)
        .rposition(|w| w == b"</plist>")?;
    let end = start + end_rel + 8;
    Some(trim_ascii(&body[start..end]).to_vec())
}

fn extract_embedded_dict(body: &[u8]) -> Option<Vec<u8>> {
    let lower = to_ascii_lower(body);
    let start = lower.windows(5).position(|w| w == b"<dict")?;
    let end_rel = lower[start..]
        .windows(7)
        .rposition(|w| w == b"</dict>")?;
    let end = start + end_rel + 7;
    Some(trim_ascii(&body[start..end]).to_vec())
}

fn trim_ascii(body: &[u8]) -> &[u8] {
    let start = body
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(body.len());
    let end = body
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map(|i| i + 1)
        .unwrap_or(start);
    &body[start..end]
}

fn bytes_contains_ci(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || hay.len() < needle.len() {
        return false;
    }
    hay.windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle))
}

fn to_ascii_lower(body: &[u8]) -> Vec<u8> {
    body.iter().map(u8::to_ascii_lowercase).collect()
}

fn to_lower_prefix(body: &[u8], n: usize) -> Vec<u8> {
    body.iter()
        .take(n)
        .map(u8::to_ascii_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_normalize_standard_plist() {
        let input = br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>test</key>
    <string>value</string>
</dict>
</plist>"#;
        let result: HashMap<String, String> = parse_plist_response(input).unwrap();
        assert_eq!(result.get("test"), Some(&"value".to_string()));
    }

    #[test]
    fn test_normalize_wrapped_in_document() {
        let input = br#"<Document>
<plist version="1.0">
<dict>
    <key>hello</key>
    <string>world</string>
</dict>
</plist>
</Document>"#;
        let result: HashMap<String, String> = parse_plist_response(input).unwrap();
        assert_eq!(result.get("hello"), Some(&"world".to_string()));
    }

    #[test]
    fn test_normalize_bare_dict() {
        let input = br#"<dict>
    <key>foo</key>
    <string>bar</string>
</dict>"#;
        let result: HashMap<String, String> = parse_plist_response(input).unwrap();
        assert_eq!(result.get("foo"), Some(&"bar".to_string()));
    }

    #[test]
    fn test_normalize_bare_keys() {
        let input = br#"<key>name</key>
<string>test</string>"#;
        let result: HashMap<String, String> = parse_plist_response(input).unwrap();
        assert_eq!(result.get("name"), Some(&"test".to_string()));
    }

    #[test]
    fn html_error_page_is_rejected_clearly() {
        let input = b"<!DOCTYPE html><html><head><title>Access Denied</title></head><body>blocked by edge</body></html>";
        let err = parse_plist_http::<HashMap<String, plist::Value>>(
            "bag",
            "https://init.itunes.apple.com/bag.xml?guid=X",
            Some(403),
            Some("text/html"),
            input,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("HTTP 403"), "{msg}");
        assert!(msg.contains("html-title=Access Denied"), "{msg}");
        assert!(msg.contains("blocked by edge"), "{msg}");
        assert!(msg.contains("full-body-file="), "{msg}");
        assert!(!msg.contains("Serde("), "{msg}");
        assert!(!msg.contains("html head title"), "{msg}");
    }

    #[test]
    fn document_bag_urlbag_parses() {
        let input = br#"<?xml version="1.0"?>
<Document xmlns="http://www.apple.com/itms/">
<Protocol>
<plist version="1.0"><dict>
<key>urlBag</key><dict>
<key>authenticateAccount</key>
<string>https://buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/authenticate</string>
</dict>
</dict></plist>
</Document>"#;
        let outer: HashMap<String, plist::Value> = parse_plist_response(input).unwrap();
        let ub = outer.get("urlBag").and_then(|v| v.as_dictionary()).unwrap();
        assert!(ub.contains_key("authenticateAccount"));
    }
}
