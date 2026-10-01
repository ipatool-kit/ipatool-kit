//! iTunes / App Store pure helpers (no network).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct App {
    pub id: i64,
    #[serde(rename = "bundleID", default)]
    pub bundle_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub price: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SearchResult {
    pub count: i32,
    pub results: Vec<App>,
}

pub fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{b:02X}"));
            }
        }
    }
    out
}

pub fn build_query(params: &BTreeMap<String, String>) -> String {
    let mut q = String::new();
    for (k, v) in params {
        if !q.is_empty() {
            q.push('&');
        }
        q.push_str(&url_encode(k));
        q.push('=');
        q.push_str(&url_encode(v));
    }
    q
}

pub fn app_from_json(j: &Value) -> App {
    let mut a = App::default();
    if let Some(v) = j.get("trackId").and_then(|x| x.as_i64()) {
        a.id = v;
    }
    if let Some(v) = j.get("bundleId").and_then(|x| x.as_str()) {
        a.bundle_id = v.to_string();
    }
    if let Some(v) = j.get("trackName").and_then(|x| x.as_str()) {
        a.name = v.to_string();
    }
    if let Some(v) = j.get("version").and_then(|x| x.as_str()) {
        a.version = v.to_string();
    }
    if let Some(v) = j.get("price").and_then(|x| x.as_f64()) {
        a.price = v;
    }
    a
}

pub fn parse_search_json(body: &str) -> SearchResult {
    let mut out = SearchResult::default();
    let Ok(j) = serde_json::from_str::<Value>(body) else {
        return out;
    };
    if let Some(c) = j.get("resultCount").and_then(|x| x.as_i64()) {
        out.count = c as i32;
    }
    if let Some(arr) = j.get("results").and_then(|x| x.as_array()) {
        for item in arr {
            out.results.push(app_from_json(item));
        }
    }
    out
}
