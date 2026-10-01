//! Thin wrapper around the local `ipatool-cpp` binary for auth / purchase / download.
//! Native Rust `HttpClient` has no SAP — auth/purchase/download go through this backend.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ipatool::client::AuthInfo;
use ipatool::IpatoolError;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct AuthJson {
    #[serde(default)]
    email: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    storefront: String,
    #[serde(default)]
    success: bool,
}

#[derive(Debug, Clone)]
pub struct VersionMeta {
    pub external_id: String,
    pub display_version: String,
}

pub fn find_binary() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("IPATOOL_CPP") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }

    if let Some(root) = crate::paths::data_root() {
        for name in [
            "ipatool-cpp",
            "ipatool-cpp-macOS-arm64",
            "ipatool-cpp-macOS-amd64",
            "ipatool-cpp-linux-x86_64",
            "ipatool-cpp-windows-x86_64.exe",
        ] {
            let p = root.join("bin").join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }

    which("ipatool-cpp")
        .or_else(|| which("ipatool-cpp-macOS-arm64"))
        .or_else(|| which("ipatool-cpp-macOS-amd64"))
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

pub fn auth_info(bin: &Path) -> Result<AuthInfo, IpatoolError> {
    let out = run_json(bin, &["auth", "info", "--format", "json"])?;
    let j: AuthJson =
        serde_json::from_str(&out).map_err(|e| IpatoolError::Message(e.to_string()))?;
    if !j.success && j.email.is_empty() {
        return Err(IpatoolError::Message("not logged in".into()));
    }
    Ok(AuthInfo {
        email: j.email,
        name: j.name,
        storefront: j.storefront,
        success: true,
    })
}

pub fn auth_login(
    bin: &Path,
    email: &str,
    password: &str,
    auth_code: Option<&str>,
) -> Result<AuthInfo, IpatoolError> {
    let mut args = vec![
        "auth".into(),
        "login".into(),
        "-e".into(),
        email.into(),
        "-p".into(),
        password.into(),
        "--format".into(),
        "json".into(),
    ];
    if let Some(code) = auth_code {
        if !code.is_empty() {
            args.push("--auth-code".into());
            args.push(code.into());
        }
    }
    let out = run_json_owned(bin, &args)?;
    let j: AuthJson =
        serde_json::from_str(&out).map_err(|e| IpatoolError::Message(e.to_string()))?;
    Ok(AuthInfo {
        email: j.email,
        name: j.name,
        storefront: j.storefront,
        success: true,
    })
}

pub fn auth_revoke(bin: &Path) -> Result<(), IpatoolError> {
    let status = Command::new(bin)
        .args(["auth", "revoke"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .status()
        .map_err(|e| IpatoolError::Message(format!("failed to run {}: {e}", bin.display())))?;
    if status.success() {
        Ok(())
    } else {
        Err(IpatoolError::Message("auth revoke failed".into()))
    }
}

pub fn purchase(bin: &Path, app_id: i64) -> Result<(), IpatoolError> {
    let status = Command::new(bin)
        .args(["purchase", "-i", &app_id.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| IpatoolError::Message(e.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(IpatoolError::Message("purchase failed".into()))
    }
}

/// Download into `output` path (`-o`). Creates parent dirs.
pub fn download(
    bin: &Path,
    app_id: i64,
    output: &Path,
    external_version_id: Option<&str>,
) -> Result<(), IpatoolError> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| IpatoolError::Message(format!("create Apps/: {e}")))?;
    }
    let mut cmd = Command::new(bin);
    cmd.arg("download")
        .arg("-i")
        .arg(app_id.to_string())
        .arg("-o")
        .arg(output)
        .arg("--purchase");
    if let Some(ev) = external_version_id {
        cmd.arg("--external-version-id").arg(ev);
    }
    let status = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| IpatoolError::Message(e.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(IpatoolError::Message("download failed".into()))
    }
}

pub fn list_versions(bin: &Path, app_id: i64) -> Result<Vec<String>, IpatoolError> {
    let out = run_json(
        bin,
        &[
            "list-versions",
            "-i",
            &app_id.to_string(),
            "--format",
            "json",
        ],
    )?;
    let v: Value = serde_json::from_str(&out).map_err(|e| IpatoolError::Message(e.to_string()))?;
    let mut ids = Vec::new();
    if let Some(arr) = v
        .get("externalVersionIdentifiers")
        .and_then(|x| x.as_array())
    {
        for item in arr {
            if let Some(s) = item.as_str() {
                ids.push(s.to_string());
            } else if let Some(n) = item.as_u64() {
                ids.push(n.to_string());
            } else if let Some(n) = item.as_i64() {
                ids.push(n.to_string());
            }
        }
    }
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Err(IpatoolError::Message("no versions found".into()));
    }
    Ok(ids)
}

pub fn get_version_metadata(
    bin: &Path,
    app_id: i64,
    external_version_id: &str,
) -> Result<VersionMeta, IpatoolError> {
    let out = run_capture(
        bin,
        &[
            "get-version-metadata",
            "-i",
            &app_id.to_string(),
            "--external-version-id",
            external_version_id,
            "--format",
            "json",
        ],
    )?;
    // Prefer JSON; fall back to text `displayVersion=…` like the PS1 script.
    if let Ok(v) = serde_json::from_str::<Value>(&out) {
        let display = v
            .get("displayVersion")
            .and_then(|x| x.as_str())
            .unwrap_or("NA")
            .to_string();
        return Ok(VersionMeta {
            external_id: external_version_id.to_string(),
            display_version: strip_ansi(&display),
        });
    }
    let display = out
        .split("displayVersion=")
        .nth(1)
        .map(|s| {
            s.split(|c: char| c.is_whitespace() || c == ',')
                .next()
                .unwrap_or("NA")
        })
        .unwrap_or("NA");
    Ok(VersionMeta {
        external_id: external_version_id.to_string(),
        display_version: strip_ansi(display),
    })
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c2 in chars.by_ref() {
                    if c2.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out.trim().to_string()
}

fn run_json(bin: &Path, args: &[&str]) -> Result<String, IpatoolError> {
    run_capture(bin, args)
}

fn run_capture(bin: &Path, args: &[&str]) -> Result<String, IpatoolError> {
    let output = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| IpatoolError::Message(format!("failed to run {}: {e}", bin.display())))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let msg = if !err.trim().is_empty() {
            err.trim().to_string()
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("{} exited {}", bin.display(), output.status)
        };
        return Err(IpatoolError::Message(msg));
    }
    Ok(stdout)
}

fn run_json_owned(bin: &Path, args: &[String]) -> Result<String, IpatoolError> {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_json(bin, &refs)
}
