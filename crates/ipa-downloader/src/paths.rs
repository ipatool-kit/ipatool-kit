//! Local data root for ipatool-kit (`~/.ipatool/downloader`).

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const APPS_ID_LIST_URL: &str =
    "https://raw.githubusercontent.com/ipatool-kit/ipatool-kit/main/assets/Apps_ID_List.txt";

/// Bundled starter list (embedded at compile time).
const APPS_ID_LIST_BUNDLED: &str = include_str!("../../../assets/Apps_ID_List.txt");

/// `~/.ipatool/downloader` or `$IPATOOL_KIT_HOME` / `$IPA_DOWNLOADER_HOME`.
pub fn data_root() -> Option<PathBuf> {
    for key in ["IPATOOL_KIT_HOME", "IPA_DOWNLOADER_HOME"] {
        if let Ok(p) = std::env::var(key) {
            let pb = PathBuf::from(p);
            if fs::create_dir_all(&pb).is_ok() {
                return Some(pb);
            }
        }
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()?;
    let root = PathBuf::from(home).join(".ipatool/downloader");
    fs::create_dir_all(&root).ok()?;
    Some(root)
}

/// Ensure Files/ + Apps/ exist; bootstrap empty history + Apps_ID_List if missing.
pub fn ensure_layout() -> Option<PathBuf> {
    let root = data_root()?;
    let files = root.join("Files");
    let apps = root.join("Apps");
    fs::create_dir_all(&files).ok()?;
    fs::create_dir_all(&apps).ok()?;

    let dl = files.join("Downloaded_IDs.json");
    if !dl.exists() {
        let _ = fs::write(&dl, "{}\n");
    }
    let pr = files.join("Purchased_IDs.json");
    if !pr.exists() {
        let _ = fs::write(&pr, "{}\n");
    }

    let list = files.join("Apps_ID_List.txt");
    if !list.exists() && fetch_apps_id_list(&list).is_err() {
        let _ = fs::write(&list, APPS_ID_LIST_BUNDLED);
    }

    Some(root)
}

fn fetch_apps_id_list(dest: &Path) -> Result<(), String> {
    let resp = ureq::get(APPS_ID_LIST_URL)
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .map_err(|e| e.to_string())?;
    let mut reader = resp.into_reader();
    let mut body = String::new();
    reader
        .read_to_string(&mut body)
        .map_err(|e| e.to_string())?;
    if body.trim().is_empty() {
        return Err("empty Apps_ID_List".into());
    }
    fs::write(dest, body).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn files_dir() -> Option<PathBuf> {
    for key in ["IPATOOL_KIT_FILES", "IPA_DOWNLOADER_FILES"] {
        if let Ok(p) = std::env::var(key) {
            let pb = PathBuf::from(p);
            if pb.is_dir() || fs::create_dir_all(&pb).is_ok() {
                return Some(pb);
            }
        }
    }
    let root = ensure_layout()?;
    Some(root.join("Files"))
}

pub fn apps_dir() -> Option<PathBuf> {
    for key in ["IPATOOL_KIT_APPS", "IPA_DOWNLOADER_APPS"] {
        if let Ok(p) = std::env::var(key) {
            let pb = PathBuf::from(p);
            if pb.is_dir() || fs::create_dir_all(&pb).is_ok() {
                return Some(pb);
            }
        }
    }
    let root = ensure_layout()?;
    Some(root.join("Apps"))
}
