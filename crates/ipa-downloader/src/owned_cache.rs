//! Local owned-apps cache (JSON under data root).

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ipatool::{OwnedApp, IpatoolError};
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Serialize, Deserialize)]
struct CacheFile {
    updated_at: u64,
    apps: Vec<OwnedApp>,
}

pub fn cache_path(email: &str) -> Option<PathBuf> {
    let safe: String = email
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '@' || c == '.' { c } else { '_' })
        .collect();
    paths::files_dir().map(|d| d.join(format!("Owned_Apps_Cache_{safe}.json")))
}

pub fn load_cache(email: &str) -> Option<(u64, Vec<OwnedApp>)> {
    let path = cache_path(email)?;
    let raw = fs::read_to_string(path).ok()?;
    let c: CacheFile = serde_json::from_str(&raw).ok()?;
    Some((c.updated_at, c.apps))
}

pub fn save_cache(email: &str, apps: &[OwnedApp]) -> Result<(), IpatoolError> {
    let path = cache_path(email).ok_or_else(|| IpatoolError::msg("data root unavailable"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let updated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let file = CacheFile {
        updated_at,
        apps: apps.to_vec(),
    };
    fs::write(path, serde_json::to_string_pretty(&file)?)?;
    Ok(())
}
