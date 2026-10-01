//! Local lists under isolated data root (~/.ipatool/downloader/Files).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Clone)]
pub struct ListedApp {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct HistItem {
    name: String,
    appid: String,
}

pub fn files_dir() -> Option<PathBuf> {
    paths::files_dir()
}

/// `Name: 123456` lines from Apps_ID_List.txt
pub fn load_apps_id_list(dir: &Path) -> Vec<ListedApp> {
    let path = dir.join("Apps_ID_List.txt");
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((name, id)) = line.rsplit_once(':') {
            if let Ok(id) = id.trim().parse::<i64>() {
                out.push(ListedApp {
                    id,
                    name: name.trim().to_string(),
                });
            }
        }
    }
    out
}

fn load_history_file(path: &Path) -> BTreeMap<String, Vec<ListedApp>> {
    let Ok(text) = fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    let Ok(raw) = serde_json::from_str::<BTreeMap<String, Vec<HistItem>>>(&text) else {
        return BTreeMap::new();
    };
    let mut out = BTreeMap::new();
    for (email, items) in raw {
        let apps: Vec<ListedApp> = items
            .into_iter()
            .filter_map(|it| {
                let id = it.appid.trim().parse::<i64>().ok()?;
                Some(ListedApp { id, name: it.name })
            })
            .collect();
        out.insert(email, apps);
    }
    out
}

pub fn downloaded(dir: &Path) -> BTreeMap<String, Vec<ListedApp>> {
    load_history_file(&dir.join("Downloaded_IDs.json"))
}

pub fn purchased(dir: &Path) -> BTreeMap<String, Vec<ListedApp>> {
    load_history_file(&dir.join("Purchased_IDs.json"))
}

/// Look up display name in Apps_ID_List.txt by numeric id.
pub fn name_for_id(id: i64) -> Option<String> {
    let dir = files_dir()?;
    load_apps_id_list(&dir)
        .into_iter()
        .find(|a| a.id == id)
        .map(|a| a.name)
}

/// Resolve a usable display name for an app id.
pub fn resolve_name(id: i64, fallback: &str) -> String {
    if !fallback.is_empty() && fallback != "Unknown" && !fallback.starts_with("app ") {
        return fallback.to_string();
    }
    name_for_id(id).unwrap_or_else(|| format!("app {id}"))
}

fn record_history(kind: &str, email: &str, id: i64, name: &str) -> Result<(), String> {
    let dir = files_dir().ok_or_else(|| "Files/ not found".to_string())?;
    let path = dir.join(kind);
    let mut map: BTreeMap<String, Vec<HistItem>> = if path.exists() {
        let text = fs::read_to_string(&path).unwrap_or_else(|_| "{}".into());
        serde_json::from_str(&text).unwrap_or_default()
    } else {
        BTreeMap::new()
    };

    let apps = map.entry(email.to_string()).or_default();
    let id_s = id.to_string();
    if let Some(existing) = apps.iter_mut().find(|a| a.appid == id_s) {
        if !name.is_empty() {
            existing.name = name.to_string();
        }
    } else {
        apps.push(HistItem {
            name: name.to_string(),
            appid: id_s,
        });
    }

    let text = serde_json::to_string_pretty(&map).map_err(|e| e.to_string())?;
    fs::write(&path, text + "\n").map_err(|e| e.to_string())?;
    Ok(())
}

pub fn record_downloaded(email: &str, id: i64, name: &str) -> Result<(), String> {
    record_history("Downloaded_IDs.json", email, id, name)
}

pub fn record_purchased(email: &str, id: i64, name: &str) -> Result<(), String> {
    record_history("Purchased_IDs.json", email, id, name)
}

/// Merge many purchased apps in one read/write (avoids O(n²) freeze on big libraries).
pub fn record_purchased_bulk(email: &str, apps: &[(i64, String)]) -> Result<(), String> {
    if apps.is_empty() {
        return Ok(());
    }
    let dir = files_dir().ok_or_else(|| "Files/ not found".to_string())?;
    let path = dir.join("Purchased_IDs.json");
    let mut map: BTreeMap<String, Vec<HistItem>> = if path.exists() {
        let text = fs::read_to_string(&path).unwrap_or_else(|_| "{}".into());
        serde_json::from_str(&text).unwrap_or_default()
    } else {
        BTreeMap::new()
    };

    let list = map.entry(email.to_string()).or_default();
    let mut by_id: BTreeMap<String, usize> = list
        .iter()
        .enumerate()
        .map(|(i, it)| (it.appid.clone(), i))
        .collect();

    for (id, name) in apps {
        let id_s = id.to_string();
        if let Some(&idx) = by_id.get(&id_s) {
            if !name.is_empty() {
                list[idx].name = name.clone();
            }
        } else {
            by_id.insert(id_s.clone(), list.len());
            list.push(HistItem {
                name: name.clone(),
                appid: id_s,
            });
        }
    }

    let text = serde_json::to_string_pretty(&map).map_err(|e| e.to_string())?;
    fs::write(&path, text + "\n").map_err(|e| e.to_string())?;
    Ok(())
}

/// Clear one account's entries (or whole file if `email` is None) from history JSON.
pub fn clear_history(kind: &str, email: Option<&str>) -> Result<String, String> {
    let dir = files_dir().ok_or_else(|| "Files/ not found".to_string())?;
    let path = dir.join(kind);
    if !path.exists() {
        return Err(format!("{kind} empty"));
    }
    if let Some(email) = email {
        let text = fs::read_to_string(&path).unwrap_or_else(|_| "{}".into());
        let mut map: BTreeMap<String, Vec<HistItem>> =
            serde_json::from_str(&text).unwrap_or_default();
        let keys: Vec<String> = map
            .keys()
            .filter(|k| k.eq_ignore_ascii_case(email))
            .cloned()
            .collect();
        if keys.is_empty() {
            return Err(format!("no history for {email}"));
        }
        for k in keys {
            map.remove(&k);
        }
        let text = serde_json::to_string_pretty(&map).map_err(|e| e.to_string())?;
        fs::write(&path, text + "\n").map_err(|e| e.to_string())?;
        Ok(format!("cleared {email} from {kind}"))
    } else {
        fs::write(&path, "{}\n").map_err(|e| e.to_string())?;
        Ok(format!("cleared {kind}"))
    }
}

pub fn history_for_account(
    map: &BTreeMap<String, Vec<ListedApp>>,
    email: Option<&str>,
) -> Vec<ListedApp> {
    if let Some(email) = email {
        if let Some(v) = map.get(email) {
            return v.clone();
        }
        for (k, v) in map {
            if k.eq_ignore_ascii_case(email) {
                return v.clone();
            }
        }
    }
    let mut all = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for apps in map.values() {
        for a in apps {
            if seen.insert(a.id) {
                all.push(a.clone());
            }
        }
    }
    all
}

/// Search curated list + download/purchase history (substring, case-insensitive).
pub fn search_local(query: &str, email: Option<&str>) -> Vec<ListedApp> {
    let q = query.to_lowercase();
    let Some(dir) = files_dir() else {
        return Vec::new();
    };

    let mut by_id: BTreeMap<i64, ListedApp> = BTreeMap::new();

    for a in history_for_account(&downloaded(&dir), email) {
        if a.name.to_lowercase().contains(&q) {
            by_id.entry(a.id).or_insert(a);
        }
    }
    for a in history_for_account(&purchased(&dir), email) {
        if a.name.to_lowercase().contains(&q) {
            by_id.entry(a.id).or_insert(a);
        }
    }
    for a in load_apps_id_list(&dir) {
        if a.name.to_lowercase().contains(&q) {
            by_id.entry(a.id).or_insert(a);
        }
    }

    let mut hist_ids = std::collections::BTreeSet::new();
    for a in history_for_account(&downloaded(&dir), email)
        .into_iter()
        .chain(history_for_account(&purchased(&dir), email))
    {
        hist_ids.insert(a.id);
    }

    let mut hist = Vec::new();
    let mut rest = Vec::new();
    for (id, app) in by_id {
        if hist_ids.contains(&id) {
            hist.push(app);
        } else {
            rest.push(app);
        }
    }
    hist.extend(rest);
    hist
}
