//! Apple purchase-history (DAAP) — port of majd/ipatool OwnedApps.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use ipatool_core::client::AppleClient;
use ipatool_core::model::Account;
use ipatool_core::sap::ActionSigner;
use serde::{Deserialize, Serialize};

use crate::error::{IpatoolError, Result};

const DAAP_BASE: &str = "https://pd.itunes.apple.com/WebObjects/MZPurchaseDaap.woa/purchase";
const MEDIA_KIND_APPS: u64 = 131_072;
const MEDIA_KIND_ARCADE: u64 = 262_144;
const MEDIA_KIND_MAC: u64 = 67_108_864;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OwnedApp {
    #[serde(default, alias = "trackId")]
    pub id: i64,
    #[serde(default, alias = "bundleId", rename = "bundleID")]
    pub bundle_id: String,
    #[serde(default, alias = "trackName")]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default, alias = "purchaseDate")]
    pub purchase_date: String,
    #[serde(default)]
    pub platforms: Vec<String>,
}

pub async fn fetch_owned_apps(
    client: &AppleClient,
    account: &Account,
    signer: &dyn ActionSigner,
) -> Result<Vec<OwnedApp>> {
    let guid = client.guid().to_string();
    let store_front_base = account
        .store_front
        .split(',')
        .next()
        .unwrap_or(&account.store_front)
        .to_string();

    let mut apps = Vec::new();
    for store in ["34", "13"] {
        let mut acc = account.clone();
        acc.store_front = format!("{store_front_base},{store}");
        let batch = fetch_owned_apps_for_storefront(client, &acc, &guid, signer).await?;
        apps.extend(batch);
    }
    Ok(merge_owned_apps(apps))
}

async fn fetch_owned_apps_for_storefront(
    client: &AppleClient,
    account: &Account,
    guid: &str,
    signer: &dyn ActionSigner,
) -> Result<Vec<OwnedApp>> {
    let login_body = client
        .http()
        .post(format!("{DAAP_BASE}/login"))
        .headers(owned_headers(account, guid)?)
        .send()
        .await
        .map_err(|e| IpatoolError::Http(e.to_string()))?;
    let login_status = login_body.status().as_u16();
    let login_bytes = login_body
        .bytes()
        .await
        .map_err(|e| IpatoolError::Http(e.to_string()))?;
    check_http("purchase history login", login_status)?;
    check_dmap_status("purchase history login", &login_bytes)?;

    let session_id = first_dmap_u32(&login_bytes, "mlid")
        .ok_or_else(|| IpatoolError::msg("purchase history login: missing session id"))?;

    let query = format!(
        "('com.apple.itunes.extended\\-media\\-kind:{MEDIA_KIND_APPS}','com.apple.itunes.extended\\-media\\-kind:{MEDIA_KIND_ARCADE}','com.apple.itunes.extended\\-media\\-kind:{MEDIA_KIND_MAC}')"
    );
    let update_body = format!("session-id={session_id}&revision-number=(null)&query={query}");
    let update_bytes = update_body.into_bytes();
    let signature = signer
        .sign(&update_bytes)
        .map_err(|e| IpatoolError::msg(format!("SAP sign: {e}")))?;
    let mut headers = owned_headers(account, guid)?;
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        "application/x-www-form-urlencoded"
            .parse()
            .map_err(|e| IpatoolError::msg(format!("header: {e}")))?,
    );
    headers.insert(
        reqwest::header::HeaderName::from_static("x-apple-actionsignature"),
        encode_sig(&signature)?,
    );

    let update_resp = client
        .http()
        .post(format!("{DAAP_BASE}/update"))
        .headers(headers)
        .body(update_bytes)
        .send()
        .await
        .map_err(|e| IpatoolError::Http(e.to_string()))?;
    let update_status = update_resp.status().as_u16();
    let update_data = update_resp
        .bytes()
        .await
        .map_err(|e| IpatoolError::Http(e.to_string()))?;
    check_http("purchase history update", update_status)?;
    check_dmap_status("purchase history update", &update_data)?;

    let latest = first_dmap_u32(&update_data, "musr")
        .ok_or_else(|| IpatoolError::msg("purchase history update: missing revision"))?;

    let items_body = owned_items_body(session_id, latest, &query);
    let items_sig = signer
        .sign(&items_body)
        .map_err(|e| IpatoolError::msg(format!("SAP sign: {e}")))?;
    let mut items_headers = owned_headers(account, guid)?;
    items_headers.insert(
        reqwest::header::CONTENT_TYPE,
        "application/x-dmap-tagged"
            .parse()
            .map_err(|e| IpatoolError::msg(format!("header: {e}")))?,
    );
    items_headers.insert(
        reqwest::header::HeaderName::from_static("x-apple-actionsignature"),
        encode_sig(&items_sig)?,
    );

    let items_resp = client
        .http()
        .post(format!("{DAAP_BASE}/databases/{latest}/items"))
        .headers(items_headers)
        .body(items_body)
        .send()
        .await
        .map_err(|e| IpatoolError::Http(e.to_string()))?;
    let items_status = items_resp.status().as_u16();
    let items_data = items_resp
        .bytes()
        .await
        .map_err(|e| IpatoolError::Http(e.to_string()))?;
    check_http("purchase history items", items_status)?;
    check_dmap_status("purchase history items", &items_data)?;

    parse_owned_apps(&items_data)
}

fn encode_sig(sig: &[u8]) -> Result<reqwest::header::HeaderValue> {
    let b64 = base64::engine::general_purpose::STANDARD.encode(sig);
    b64.parse()
        .map_err(|e| IpatoolError::msg(format!("sig header: {e}")))
}

fn owned_headers(account: &Account, guid: &str) -> Result<reqwest::header::HeaderMap> {
    let now = SystemTime::now();
    let unix = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Approximate UTC HTTP date without chrono.
    let http_date = httpdate_now(unix);
    let iso = format_iso(unix);

    let mut h = reqwest::header::HeaderMap::new();
    let pairs = [
        ("accept", "*/*"),
        ("accept-language", "en-us"),
        ("client-cloud-daap-request-reason", "5"),
        ("client-cloud-purchase-daap-version", "1.1/Configurator-2.0"),
        ("client-daap-version", "3.12"),
        ("date", http_date.as_str()),
        ("icloud-dsid", account.directory_services_id.as_str()),
        ("x-apple-i-client-time", iso.as_str()),
        ("x-apple-i-locale", "en_US"),
        ("x-apple-i-timezone", "UTC"),
        ("x-apple-store-front", account.store_front.as_str()),
        ("x-apple-tz", "0"),
        ("x-dsid", account.directory_services_id.as_str()),
        ("x-guid", guid),
        ("x-token", account.password_token.as_str()),
    ];
    for (k, v) in pairs {
        let name = reqwest::header::HeaderName::from_bytes(k.as_bytes())
            .map_err(|e| IpatoolError::msg(format!("header name: {e}")))?;
        let val = reqwest::header::HeaderValue::from_str(v)
            .map_err(|e| IpatoolError::msg(format!("header value: {e}")))?;
        h.insert(name, val);
    }
    Ok(h)
}

fn httpdate_now(unix: u64) -> String {
    // Minimal GMT formatter (good enough for Apple DAAP Date header).
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let secs = unix as i64;
    let days = secs.div_euclid(86400);
    let tod = secs.rem_euclid(86400) as u32;
    let hh = tod / 3600;
    let mm = (tod % 3600) / 60;
    let ss = tod % 60;
    // Civil date from days since 1970-01-01 (Thursday).
    let (y, m, d) = civil_from_days(days + 719_468);
    let weekday = DAYS[((days % 7) + 7) as usize % 7];
    format!(
        "{weekday}, {d:02} {} {y} {hh:02}:{mm:02}:{ss:02} GMT",
        MONTHS[(m - 1) as usize]
    )
}

fn format_iso(unix: u64) -> String {
    let secs = unix as i64;
    let days = secs.div_euclid(86400);
    let tod = secs.rem_euclid(86400) as u32;
    let hh = tod / 3600;
    let mm = (tod % 3600) / 60;
    let ss = tod % 60;
    let (y, m, d) = civil_from_days(days + 719_468);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Howard Hinnant civil_from_days (proleptic Gregorian).
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

fn owned_items_body(session_id: u32, latest: u32, query: &str) -> Vec<u8> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0);
    let mut payload = Vec::new();
    payload.extend(dmap_u32("mstc", now));
    payload.extend(dmap_u32("mlid", session_id));
    payload.extend(dmap_u8("mikd", 2));
    payload.extend(dmap_u32("musr", latest));
    payload.extend(dmap_u32("mder", 0));
    payload.extend(dmap_str("mque", query));
    payload.extend(dmap_tag("aetl", &[]));
    dmap_tag("adsr", &payload)
}

fn dmap_tag(name: &str, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

fn dmap_u8(name: &str, v: u8) -> Vec<u8> {
    dmap_tag(name, &[v])
}

fn dmap_u32(name: &str, v: u32) -> Vec<u8> {
    dmap_tag(name, &v.to_be_bytes())
}

fn dmap_str(name: &str, v: &str) -> Vec<u8> {
    dmap_tag(name, v.as_bytes())
}

fn check_http(label: &str, status: u16) -> Result<()> {
    if status == 401 || status == 403 {
        return Err(IpatoolError::msg(format!(
            "{label}: token expired (HTTP {status})"
        )));
    }
    if status != 200 {
        return Err(IpatoolError::msg(format!(
            "{label}: HTTP {status}"
        )));
    }
    Ok(())
}

fn check_dmap_status(label: &str, data: &[u8]) -> Result<()> {
    if let Some(status) = first_dmap_u32(data, "mstt") {
        if status == 401 || status == 403 {
            return Err(IpatoolError::msg(format!(
                "{label}: token expired (DAAP {status})"
            )));
        }
        if status != 200 {
            return Err(IpatoolError::msg(format!(
                "{label}: DAAP status {status}"
            )));
        }
    }
    Ok(())
}

fn first_dmap_u32(data: &[u8], target: &str) -> Option<u32> {
    let mut found = None;
    let _ = walk_dmap(data, &mut |tag, payload| {
        if found.is_none() && tag == target {
            found = match payload.len() {
                4 => payload.try_into().ok().map(u32::from_be_bytes),
                8 => payload
                    .try_into()
                    .ok()
                    .map(|b| u64::from_be_bytes(b) as u32),
                _ => None,
            };
        }
        Ok(())
    });
    found
}

fn walk_dmap(data: &[u8], f: &mut dyn FnMut(&str, &[u8]) -> Result<()>) -> Result<()> {
    let mut i = 0;
    while i + 8 <= data.len() {
        let tag = std::str::from_utf8(&data[i..i + 4])
            .map_err(|_| IpatoolError::msg("invalid DMAP tag"))?;
        let len = u32::from_be_bytes(data[i + 4..i + 8].try_into().unwrap()) as usize;
        i += 8;
        if i + len > data.len() {
            return Err(IpatoolError::msg("truncated DMAP payload"));
        }
        let payload = &data[i..i + len];
        f(tag, payload)?;
        // Containers recurse.
        if is_container(tag) {
            walk_dmap(payload, f)?;
        }
        i += len;
    }
    Ok(())
}

fn is_container(tag: &str) -> bool {
    matches!(
        tag,
        "adbs" | "adsr" | "aply" | "avdb" | "mbcl" | "mccr" | "mcty" | "mdcl" | "mlcl" | "mlit"
            | "mlog" | "msrv" | "mupd"
    )
}

fn parse_owned_apps(data: &[u8]) -> Result<Vec<OwnedApp>> {
    let mut apps = Vec::new();
    walk_dmap(data, &mut |tag, payload| {
        if tag == "mlit" {
            if let Some(app) = parse_owned_app(payload)? {
                apps.push(app);
            }
        }
        Ok(())
    })?;
    Ok(merge_owned_apps(apps))
}

fn parse_owned_app(data: &[u8]) -> Result<Option<OwnedApp>> {
    let mut app = OwnedApp {
        id: 0,
        bundle_id: String::new(),
        name: String::new(),
        version: String::new(),
        purchase_date: String::new(),
        platforms: Vec::new(),
    };
    let mut media_kind = 0u64;
    let mut supported = 0u64;

    walk_dmap(data, &mut |tag, payload| {
        match tag {
            "aeMk" | "aeSS" => {
                let v = dmap_int(payload)?;
                if tag == "aeMk" {
                    media_kind = v;
                } else {
                    supported = v;
                }
            }
            "aeSI" => app.id = dmap_int(payload)? as i64,
            "aeBI" => app.bundle_id = String::from_utf8_lossy(payload).into_owned(),
            "aeLN" => app.name = String::from_utf8_lossy(payload).into_owned(),
            "minm" => {
                if app.name.is_empty() {
                    app.name = String::from_utf8_lossy(payload).into_owned();
                }
            }
            "aePd" => app.version = String::from_utf8_lossy(payload).into_owned(),
            "asdp" if payload.len() == 4 => {
                let ts = u32::from_be_bytes(payload.try_into().unwrap());
                app.purchase_date = format_iso(ts as u64);
            }
            _ => {}
        }
        Ok(())
    })?;

    if app.id == 0 {
        return Ok(None);
    }

    match media_kind {
        MEDIA_KIND_MAC => app.platforms.push("macos".into()),
        MEDIA_KIND_APPS | MEDIA_KIND_ARCADE => {
            if supported & 1 != 0 {
                app.platforms.push("iphone".into());
            }
            if supported & 2 != 0 {
                app.platforms.push("ipad".into());
            }
            if supported & 16 != 0 {
                app.platforms.push("visionos".into());
            }
            if app.platforms.is_empty() {
                app.platforms.push("iphone".into());
            }
        }
        _ => {
            if app.platforms.is_empty() {
                app.platforms.push("iphone".into());
            }
        }
    }

    Ok(Some(app))
}

fn dmap_int(payload: &[u8]) -> Result<u64> {
    Ok(match payload.len() {
        1 => payload[0] as u64,
        2 => u16::from_be_bytes(payload.try_into().unwrap()) as u64,
        4 => u32::from_be_bytes(payload.try_into().unwrap()) as u64,
        8 => u64::from_be_bytes(payload.try_into().unwrap()),
        n => {
            return Err(IpatoolError::msg(format!(
                "DMAP int length {n}"
            )))
        }
    })
}

fn merge_owned_apps(apps: Vec<OwnedApp>) -> Vec<OwnedApp> {
    let mut map: BTreeMap<i64, OwnedApp> = BTreeMap::new();
    for app in apps {
        map.entry(app.id)
            .and_modify(|existing| {
                for p in &app.platforms {
                    if !existing.platforms.contains(p) {
                        existing.platforms.push(p.clone());
                    }
                }
                if app.purchase_date > existing.purchase_date {
                    existing.purchase_date = app.purchase_date.clone();
                }
                if existing.name.is_empty() {
                    existing.name = app.name.clone();
                }
                if existing.bundle_id.is_empty() {
                    existing.bundle_id = app.bundle_id.clone();
                }
                if existing.version.is_empty() {
                    existing.version = app.version.clone();
                }
            })
            .or_insert(app);
    }
    let mut out: Vec<_> = map.into_values().collect();
    out.sort_by(|a, b| b.purchase_date.cmp(&a.purchase_date));
    out
}
