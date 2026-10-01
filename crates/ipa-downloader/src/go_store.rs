//! Wrapper around Go `ipatool list-purchases` (majd/ipatool).
//! Prefers a patched `ipatool-hist` (limit 50k → one Apple fetch).
//! cpp cookies are Netscape; Go expects JSON — temporarily swap for the call.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Instant;

use ipatool::IpatoolError;
use serde::{Deserialize, Serialize};

static COOKIE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Deserialize, Serialize)]
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

#[derive(Debug, Deserialize)]
struct PageJson {
    #[serde(default)]
    count: i64,
    #[serde(default, rename = "totalCount")]
    total_count: i64,
    #[serde(default)]
    page: i64,
    #[serde(default)]
    apps: Vec<OwnedApp>,
}

/// Progress while loading purchase history.
pub struct FetchProgress {
    pub page: i64,
    pub pages_total: Option<i64>,
    pub loaded: usize,
    pub total: Option<i64>,
    pub phase: &'static str,
    pub elapsed_secs: u64,
}

pub fn find_binary() -> Option<PathBuf> {
    use std::sync::OnceLock;
    static CACHED: OnceLock<Option<PathBuf>> = OnceLock::new();
    CACHED.get_or_init(find_binary_uncached).clone()
}

fn find_binary_uncached() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("IPATOOL_GO") {
        let pb = PathBuf::from(p);
        if looks_like_go_ipatool(&pb) {
            return Some(pb);
        }
    }
    // Patched oneshot binary — trust by path (skip slow --help spawn).
    if let Some(dir) = crate::paths::data_root() {
        let hist = dir.join("bin/ipatool-hist");
        if hist.is_file() {
            return Some(hist);
        }
    }
    for cand in ["/opt/homebrew/bin/ipatool", "/usr/local/bin/ipatool"] {
        let pb = PathBuf::from(cand);
        if looks_like_go_ipatool(&pb) {
            return Some(pb);
        }
    }
    which("ipatool").filter(|p| looks_like_go_ipatool(p))
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

fn looks_like_go_ipatool(bin: &Path) -> bool {
    if !bin.is_file() {
        return false;
    }
    let out = Command::new(bin)
        .args(["list-purchases", "--help"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    match out {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout);
            let e = String::from_utf8_lossy(&o.stderr);
            s.contains("List apps owned")
                || e.contains("List apps owned")
                || s.contains("max-results")
        }
        Err(_) => false,
    }
}

fn is_bulk_binary(bin: &Path) -> bool {
    bin.file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|n| n.contains("ipatool-hist"))
        || std::env::var("IPATOOL_GO_BULK").ok().as_deref() == Some("1")
}

struct CookieGuard {
    path: PathBuf,
    backup: Option<Vec<u8>>,
}

impl CookieGuard {
    fn enter() -> Result<Self, IpatoolError> {
        let home = std::env::var("HOME").map_err(|e| IpatoolError::Message(e.to_string()))?;
        let path = PathBuf::from(home).join(".ipatool/cookies");
        let backup = if path.exists() {
            Some(fs::read(&path).map_err(|e| IpatoolError::Message(e.to_string()))?)
        } else {
            None
        };
        // Also park a side copy so a crash mid-run can be recovered.
        if let Some(bytes) = &backup {
            let side = path.with_file_name("cookies.netscape.bak");
            let _ = fs::write(&side, bytes);
        }
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        // Go cookie jar is JSON; empty jar → auth via keychain account token.
        fs::write(&path, b"[]\n").map_err(|e| IpatoolError::Message(e.to_string()))?;
        Ok(Self { path, backup })
    }
}

impl Drop for CookieGuard {
    fn drop(&mut self) {
        match &self.backup {
            Some(bytes) => {
                let _ = fs::write(&self.path, bytes);
            }
            None => {
                let _ = fs::write(
                    &self.path,
                    b"# Netscape HTTP Cookie File\n# empty - re-login if needed\n",
                );
            }
        }
    }
}

/// Fetch all owned apps (incl. deleted) via Apple purchase-history DAAP.
pub fn list_all_purchases(
    platform: &str,
    mut on_progress: impl FnMut(FetchProgress),
) -> Result<Vec<OwnedApp>, IpatoolError> {
    let bin = find_binary().ok_or_else(|| {
        IpatoolError::Message(
            "Go ipatool not found (need `brew install ipatool` or set IPATOOL_GO)".into(),
        )
    })?;

    let _lock = COOKIE_LOCK
        .lock()
        .map_err(|_| IpatoolError::Message("cookie lock poisoned".into()))?;
    let _guard = CookieGuard::enter()?;

    let platform = if platform.is_empty() {
        "iphone"
    } else {
        platform
    };
    let started = Instant::now();

    on_progress(FetchProgress {
        page: 0,
        pages_total: None,
        loaded: 0,
        total: None,
        phase: "connecting to Apple…",
        elapsed_secs: 0,
    });

    if is_bulk_binary(&bin) {
        return fetch_bulk(&bin, platform, started, &mut on_progress);
    }

    // Stock brew ipatool: max 100/page; each page re-fetches the full Apple list (~30–45s).
    fetch_paged(&bin, platform, started, &mut on_progress)
}

fn platform_args(platform: &str) -> Vec<&'static str> {
    // Empty platform = full Apple purchase history (iPhone + iPad + Mac + …).
    // Filtering to "iphone" drops owned Mac apps and some older records.
    match platform {
        "iphone" => vec!["--platform", "iphone"],
        "ipad" => vec!["--platform", "ipad"],
        "macos" | "mac" => vec!["--platform", "macos"],
        "appletv" | "tv" => vec!["--platform", "appletv"],
        _ => Vec::new(),
    }
}

fn fetch_bulk(
    bin: &Path,
    platform: &str,
    started: Instant,
    on_progress: &mut impl FnMut(FetchProgress),
) -> Result<Vec<OwnedApp>, IpatoolError> {
    on_progress(FetchProgress {
        page: 1,
        pages_total: Some(1),
        loaded: 0,
        total: None,
        phase: "downloading full purchase history (1 request, ~30–60s)…",
        elapsed_secs: started.elapsed().as_secs(),
    });

    let bin = bin.to_path_buf();
    let platform = platform.to_string();
    let (tx, rx) = std::sync::mpsc::channel::<Result<String, IpatoolError>>();
    std::thread::spawn(move || {
        let mut args = vec![
            "list-purchases",
            "-l",
            "50000",
            "-p",
            "1",
            "--format",
            "json",
            "--non-interactive",
        ];
        let plat = platform_args(&platform);
        args.extend(plat.iter().copied());
        let r = run_json(&bin, &args);
        let _ = tx.send(r);
    });

    let out = loop {
        match rx.try_recv() {
            Ok(r) => break r?,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                on_progress(FetchProgress {
                    page: 1,
                    pages_total: Some(1),
                    loaded: 0,
                    total: None,
                    phase: "waiting for Apple…",
                    elapsed_secs: started.elapsed().as_secs(),
                });
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return Err(IpatoolError::Message("purchase-history worker died".into()));
            }
        }
    };

    let parsed: PageJson =
        serde_json::from_str(&out).map_err(|e| IpatoolError::Message(format!("{e}: {out}")))?;

    let mut all = parsed.apps;
    let total = if parsed.total_count > 0 {
        parsed.total_count
    } else {
        all.len() as i64
    };

    on_progress(FetchProgress {
        page: 1,
        pages_total: Some(1),
        loaded: all.len(),
        total: Some(total),
        phase: "done",
        elapsed_secs: started.elapsed().as_secs(),
    });

    dedup(&mut all);
    let _ = parsed.count;
    let _ = parsed.page;
    Ok(all)
}

fn fetch_paged(
    bin: &Path,
    platform: &str,
    started: Instant,
    on_progress: &mut impl FnMut(FetchProgress),
) -> Result<Vec<OwnedApp>, IpatoolError> {
    let mut all = Vec::new();
    let mut page = 1i64;
    let mut total = i64::MAX;

    while (all.len() as i64) < total && page <= 200 {
        let pages_total = if total < i64::MAX {
            Some((total + 99) / 100)
        } else {
            None
        };
        on_progress(FetchProgress {
            page,
            pages_total,
            loaded: all.len(),
            total: if total < i64::MAX { Some(total) } else { None },
            phase: "fetching page from Apple…",
            elapsed_secs: started.elapsed().as_secs(),
        });

        let page_s = page.to_string();
        let mut args = vec![
            "list-purchases",
            "-l",
            "100",
            "-p",
            page_s.as_str(),
            "--format",
            "json",
            "--non-interactive",
        ];
        let plat = platform_args(platform);
        args.extend(plat.iter().copied());
        let out = run_json(bin, &args)?;
        let parsed: PageJson =
            serde_json::from_str(&out).map_err(|e| IpatoolError::Message(format!("{e}: {out}")))?;
        if parsed.total_count > 0 {
            total = parsed.total_count;
        }
        if parsed.apps.is_empty() {
            break;
        }
        let n = parsed.apps.len();
        all.extend(parsed.apps);

        on_progress(FetchProgress {
            page,
            pages_total: if total < i64::MAX {
                Some((total + 99) / 100)
            } else {
                None
            },
            loaded: all.len(),
            total: if total < i64::MAX { Some(total) } else { None },
            phase: "page ok",
            elapsed_secs: started.elapsed().as_secs(),
        });

        if n < 100 {
            break;
        }
        page += 1;
        let _ = parsed.count;
        let _ = parsed.page;
    }

    dedup(&mut all);
    Ok(all)
}

fn dedup(all: &mut Vec<OwnedApp>) {
    let mut seen = std::collections::BTreeSet::new();
    all.retain(|a| a.id != 0 && seen.insert(a.id));
}

fn run_json(bin: &Path, args: &[&str]) -> Result<String, IpatoolError> {
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
    if let Some(line) = stdout
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))
    {
        return Ok(line.to_string());
    }
    Ok(stdout)
}

#[derive(Debug, Deserialize, Serialize)]
struct OwnedCache {
    email: String,
    fetched_at: String,
    apps: Vec<OwnedApp>,
}

pub fn cache_path(email: &str) -> Option<PathBuf> {
    let dir = crate::paths::files_dir()?;
    let safe: String = email
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '@' || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    Some(dir.join(format!("Owned_Apps_Cache_{safe}.json")))
}

pub fn load_cache(email: &str) -> Option<(String, Vec<OwnedApp>)> {
    let try_paths: Vec<PathBuf> = {
        let mut v = Vec::new();
        if let Some(p) = cache_path(email) {
            v.push(p);
        }
        if let Some(dir) = crate::paths::files_dir() {
            v.push(dir.join("Owned_Apps_Cache__last.json"));
        }
        v
    };
    for path in try_paths {
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let c: OwnedCache = match serde_json::from_str(&text) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if c.apps.is_empty() {
            continue;
        }
        if !c.email.is_empty() && !email.is_empty() && !c.email.eq_ignore_ascii_case(email) {
            continue;
        }
        return Some((c.fetched_at, c.apps));
    }
    None
}

pub fn save_cache(email: &str, apps: &[OwnedApp]) -> Result<(), String> {
    let path = cache_path(email).ok_or_else(|| "Files/ missing".to_string())?;
    let c = OwnedCache {
        email: email.to_string(),
        fetched_at: chrono_like_now(),
        apps: apps.to_vec(),
    };
    let text = serde_json::to_string(&c).map_err(|e| e.to_string())?;
    fs::write(&path, text.clone() + "\n").map_err(|e| e.to_string())?;
    if let Some(dir) = crate::paths::files_dir() {
        let _ = fs::write(dir.join("Owned_Apps_Cache__last.json"), text + "\n");
    }
    Ok(())
}

fn chrono_like_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{secs}")
}
