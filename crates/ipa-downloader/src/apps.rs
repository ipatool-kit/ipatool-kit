//! Apps/ helpers under isolated data root (~/.ipatool/downloader/Apps).

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ipatool::plist::{self, dict_str};
use ipatool::IpatoolError;
use zip::ZipArchive;

use crate::lists;
use crate::paths;

#[derive(Debug, Clone)]
pub struct IpaMeta {
    pub app_name: String,
    pub version: String,
    pub min_ios: String,
}

#[derive(Debug, Clone)]
pub struct IpaFile {
    pub path: PathBuf,
    pub file_name: String,
    pub meta: IpaMeta,
}

pub fn apps_dir() -> Option<PathBuf> {
    paths::apps_dir()
}

fn sanitize_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => ' ',
            _ => c,
        })
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join("_")
}

pub fn read_ipa_metadata(path: &Path) -> Option<IpaMeta> {
    let file = fs::File::open(path).ok()?;
    let mut zip = ZipArchive::new(file).ok()?;

    let mut best_idx: Option<usize> = None;
    let mut best_depth = usize::MAX;
    for i in 0..zip.len() {
        let name = zip.by_index(i).ok()?.name().to_string();
        if !name.starts_with("Payload/") || !name.ends_with(".app/Info.plist") {
            continue;
        }
        // Prefer Payload/Foo.app/Info.plist (depth 3) over nested bundles.
        let depth = name.matches('/').count();
        if depth < best_depth {
            best_depth = depth;
            best_idx = Some(i);
        }
    }
    let idx = best_idx?;
    let mut entry = zip.by_index(idx).ok()?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf).ok()?;

    let dict = plist::decode_plist_bytes(&buf);
    let mut app_name = dict_str(&dict, "CFBundleName");
    if app_name.is_empty() || app_name == "App" {
        let display = dict_str(&dict, "CFBundleDisplayName");
        if !display.is_empty() {
            app_name = display;
        }
    }
    if app_name.is_empty() {
        app_name = "App".into();
    }
    let version = {
        let v = dict_str(&dict, "CFBundleShortVersionString");
        if v.is_empty() {
            "0".into()
        } else {
            v
        }
    };
    let min_ios = {
        let v = dict_str(&dict, "MinimumOSVersion");
        if v.is_empty() {
            "NA".into()
        } else {
            v
        }
    };

    Some(IpaMeta {
        app_name: sanitize_name(&app_name),
        version,
        min_ios,
    })
}

pub fn list_ipas() -> Vec<IpaFile> {
    let Some(dir) = apps_dir() else {
        return Vec::new();
    };
    let Ok(rd) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for ent in rd.flatten() {
        let path = ent.path();
        if path.extension().and_then(|e| e.to_str()) != Some("ipa") {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("app.ipa")
            .to_string();
        let meta = read_ipa_metadata(&path).unwrap_or(IpaMeta {
            app_name: file_name.trim_end_matches(".ipa").into(),
            version: "?".into(),
            min_ios: "Error".into(),
        });
        out.push(IpaFile {
            path,
            file_name,
            meta,
        });
    }
    out.sort_by_key(|a| a.file_name.to_lowercase());
    out
}

/// Finalize a just-downloaded IPA already in Apps/ (or elsewhere): rename + history.
pub fn finalize_downloaded_ipa(
    src: &Path,
    app_id: i64,
    preferred_name: &str,
    email: Option<&str>,
) -> Result<String, IpatoolError> {
    let apps = apps_dir().ok_or_else(|| IpatoolError::Message("Apps/ not available".into()))?;
    if !src.exists() {
        return Err(IpatoolError::Message(format!(
            "download missing: {}",
            src.display()
        )));
    }

    let dest_tmp = if src.parent() == Some(apps.as_path()) {
        src.to_path_buf()
    } else {
        let dest_tmp = apps.join(
            src.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("download.ipa"),
        );
        fs::rename(src, &dest_tmp)
            .or_else(|_| fs::copy(src, &dest_tmp).and_then(|_| fs::remove_file(src)))
            .map_err(|e| IpatoolError::Message(format!("move to Apps/: {e}")))?;
        dest_tmp
    };

    let meta = read_ipa_metadata(&dest_tmp).unwrap_or(IpaMeta {
        app_name: "App".into(),
        version: "0".into(),
        min_ios: "NA".into(),
    });

    let mut final_name = meta.app_name.clone();
    if !preferred_name.is_empty()
        && preferred_name != "Unknown"
        && !preferred_name.starts_with("app ")
    {
        final_name = sanitize_name(preferred_name);
    } else if let Some(listed) = lists::name_for_id(app_id) {
        final_name = sanitize_name(&listed);
    }

    let account = email.unwrap_or("unknown");
    let new_name = format!(
        "{}_{}_iOS_{}+_{}.ipa",
        final_name, meta.version, meta.min_ios, account
    )
    .replace(' ', "_");
    let target = apps.join(&new_name);
    if target.exists() && target != dest_tmp {
        let _ = fs::remove_file(&target);
    }
    if dest_tmp != target {
        fs::rename(&dest_tmp, &target)
            .map_err(|e| IpatoolError::Message(format!("rename IPA: {e}")))?;
    }

    if let Some(email) = email {
        let _ = lists::record_downloaded(email, app_id, &final_name);
    }

    Ok(format!("{}  (min iOS {})", target.display(), meta.min_ios))
}

/// Move matching `*.ipa` from dirs into Apps/ and finalize (legacy cwd fallback).
pub fn ingest_downloaded_ipas(
    app_id: i64,
    preferred_name: &str,
    email: Option<&str>,
    from_dirs: &[PathBuf],
) -> Result<Vec<String>, IpatoolError> {
    let mut sources = Vec::new();
    let id_prefix = format!("{app_id}_");
    let id_exact = format!("{app_id}.ipa");
    for dir in from_dirs {
        let Ok(rd) = fs::read_dir(dir) else {
            continue;
        };
        for ent in rd.flatten() {
            let path = ent.path();
            if path.extension().and_then(|e| e.to_str()) != Some("ipa") {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if name.starts_with(&id_prefix)
                || name == id_exact
                || name.contains(&format!("_{app_id}_"))
            {
                sources.push(path);
            }
        }
    }

    let mut reports = Vec::new();
    for src in sources {
        reports.push(finalize_downloaded_ipa(
            &src,
            app_id,
            preferred_name,
            email,
        )?);
    }
    Ok(reports)
}

pub fn clear_all_ipas() -> Result<usize, IpatoolError> {
    let apps = apps_dir().ok_or_else(|| IpatoolError::Message("Apps/ not available".into()))?;
    let mut n = 0usize;
    let Ok(rd) = fs::read_dir(&apps) else {
        return Ok(0);
    };
    for ent in rd.flatten() {
        let path = ent.path();
        if path.extension().and_then(|e| e.to_str()) == Some("ipa") {
            fs::remove_file(&path)
                .map_err(|e| IpatoolError::Message(format!("remove {}: {e}", path.display())))?;
            n += 1;
        }
    }
    Ok(n)
}

pub fn open_tip_jar() -> Result<(), IpatoolError> {
    let url = "https://www.donationalerts.com/r/s00d88";
    #[cfg(target_os = "macos")]
    let status = Command::new("open").arg(url).status();
    #[cfg(target_os = "linux")]
    let status = Command::new("xdg-open").arg(url).status();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let status = Command::new("cmd").args(["/C", "start", url]).status();
    status
        .map_err(|e| IpatoolError::Message(e.to_string()))?
        .success()
        .then_some(())
        .ok_or_else(|| IpatoolError::Message("failed to open tip jar URL".into()))
}

pub fn find_ideviceinstaller() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("IDEVICEINSTALLER") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    which("ideviceinstaller")
}

fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let cand = dir.join(bin);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

pub fn install_ipa(idevice: &Path, ipa: &Path) -> Result<(), IpatoolError> {
    let status = Command::new(idevice)
        .arg("install")
        .arg(ipa)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| IpatoolError::Message(e.to_string()))?;
    if status.success() {
        return Ok(());
    }
    let status = Command::new(idevice)
        .arg("upgrade")
        .arg(ipa)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| IpatoolError::Message(e.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(IpatoolError::Message("ideviceinstaller failed".into()))
    }
}
