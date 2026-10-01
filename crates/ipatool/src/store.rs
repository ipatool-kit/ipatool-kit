//! Sync facade over vendored `ipatool-kit-core` (SAP auth + download) and native DAAP purchases.
//!
//! Core patches vs crates.io 0.1.8 / Go majd/ipatool parity (see `vendor/ipatool-kit-core`):
//! bag `?guid=` + `urlBag`, HTML-safe plist parse, `serialNumber` on product POSTs,
//! volumeStore→redownload→updateProduct, purchase `jingleDocType`/`status`, auth HTTP
//! retry, login Content-Type `application/x-www-form-urlencoded`. IPA-only: macOS `.pkg`
//! decrypt path is intentionally not ported (clear error if Apple returns a Mac package).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use ipatool_core::api::{auth, bag, download as core_download, purchase as core_purchase, versions};
use ipatool_core::client::AppleClient;
use ipatool_core::guid::generate_machine_identity;
use ipatool_core::ipa::patch::patch_ipa;
use ipatool_core::model::Account;
use ipatool_core::sap::{self, ActionSigner};
use tokio::runtime::Runtime;

use crate::client::{AuthInfo, DownloadRequest, LoginRequest, VersionInfo};
use crate::error::{IpatoolError, Result};
use crate::purchases::{self, OwnedApp};
use crate::session;

fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

fn map_err(e: impl std::fmt::Display) -> IpatoolError {
    IpatoolError::msg(e.to_string())
}

fn new_client() -> Result<AppleClient> {
    session::ensure_dirs()?;
    let machine = generate_machine_identity().map_err(map_err)?;
    let cookies = session::cookies_path();
    let cache = session::cache_dir().ok_or_else(|| IpatoolError::msg("HOME not set"))?;
    AppleClient::new(machine, cookies.as_deref(), cache).map_err(map_err)
}

fn save_cookies(client: &AppleClient) {
    if let Some(path) = session::cookies_path() {
        if client.save_cookies(&path).is_ok() {
            session::chmod_secret_file(&path);
        }
    }
}

fn note(progress: &mut dyn FnMut(&str), msg: &str) {
    progress(msg);
}

/// True when ipatool-kit-core SAP framework blobs are already cached under ~/.ipatool/cache.
pub fn sap_cache_ready() -> bool {
    let Some(dir) = session::cache_dir() else {
        return false;
    };
    ["CommerceKit", "CommerceCore", "CoreFP", "CoreFP.icxs"]
        .iter()
        .all(|name| dir.join(name).is_file())
}

fn sap_progress_msg() -> &'static str {
    if sap_cache_ready() {
        "progress.sap_cached"
    } else {
        "progress.sap_download"
    }
}

/// Normalize 2FA like majd/ipatool: strip spaces, require exactly 6 digits.
pub(crate) fn normalize_auth_code(code: Option<&str>) -> Result<Option<String>> {
    let Some(code) = code else {
        return Ok(None);
    };
    let mut code = code.trim().to_string();
    if code.starts_with("\u{1b}[200~") && code.ends_with("\u{1b}[201~") {
        code = code
            .trim_start_matches("\u{1b}[200~")
            .trim_end_matches("\u{1b}[201~")
            .to_string();
    }
    let digits: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    if digits.is_empty() {
        return Ok(None);
    }
    if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(IpatoolError::msg("2FA code must contain exactly six digits"));
    }
    Ok(Some(digits))
}

pub fn auth_info() -> Result<AuthInfo> {
    let Some(acc) = session::load_account()? else {
        return Err(IpatoolError::msg("not logged in"));
    };
    Ok(AuthInfo {
        email: acc.email,
        name: acc.name,
        storefront: acc.store_front,
        success: true,
    })
}

pub fn login(req: &LoginRequest) -> Result<AuthInfo> {
    login_with(req, &mut |_| {})
}

pub fn login_with(req: &LoginRequest, progress: &mut dyn FnMut(&str)) -> Result<AuthInfo> {
    note(progress, "progress.preparing_session");
    session::ensure_dirs()?;
    let auth_code = normalize_auth_code(req.auth_code.as_deref())?;
    let mut client = new_client()?;

    note(progress, "progress.fetch_bag");
    let bag_cfg = runtime()
        .block_on(bag::fetch_bag(&client))
        .map_err(map_err)?;

    note(progress, sap_progress_msg());
    let signer = runtime()
        .block_on(sap::new_default_signer(
            &client,
            &bag_cfg.sap,
            client.hardware_id(),
        ))
        .map_err(map_err)?;

    note(
        progress,
        if auth_code.is_some() {
            "progress.auth_2fa"
        } else {
            "progress.auth"
        },
    );
    let account = runtime()
        .block_on(auth::login(
            &client,
            &req.email,
            &req.password,
            auth_code.as_deref(),
            &bag_cfg.auth_endpoint,
            Some(&signer as &dyn ActionSigner),
        ))
        .map_err(|e| {
            let s = e.to_string();
            let lower = s.to_lowercase();
            if auth_code.is_some()
                && (lower.contains("badlogin")
                    || lower.contains("verification")
                    || lower.contains("auth code"))
            {
                IpatoolError::msg("apple did not complete verification; try a fresh 2FA code")
            } else if lower.contains("auth code")
                || lower.contains("2fa")
                || lower.contains("verification")
                || lower.contains("badlogin")
            {
                IpatoolError::msg("auth code required")
            } else {
                map_err(e)
            }
        })?;

    // Keep password in keyring (via save_account) for silent reauth; not in account.json.
    note(progress, "progress.saving_session");
    let mut account = account;
    account.password = Some(req.password.clone());
    client.set_account(account.clone());
    session::save_account(&account)?;
    save_cookies(&client);

    note(progress, "progress.login_complete");
    Ok(AuthInfo {
        email: account.email,
        name: account.name,
        storefront: account.store_front,
        success: true,
    })
}

pub fn revoke() -> Result<()> {
    session::delete_account()?;
    if let Some(p) = session::cookies_path() {
        let _ = std::fs::remove_file(p);
    }
    Ok(())
}

pub fn require_account() -> Result<Account> {
    session::load_account()?.ok_or_else(|| IpatoolError::msg("not logged in"))
}

pub fn purchase(app_id: i64) -> Result<()> {
    purchase_with(app_id, &mut |_| {})
}

pub fn purchase_with(app_id: i64, progress: &mut dyn FnMut(&str)) -> Result<()> {
    note(progress, "progress.loading_account");
    let account = require_account()?;
    let client = new_client()?;
    note(progress, "progress.purchasing");
    runtime()
        .block_on(core_purchase::purchase(&client, app_id, &account))
        .map_err(map_err)?;
    save_cookies(&client);
    note(progress, "progress.purchase_done");
    Ok(())
}

pub fn list_versions(app_id: i64) -> Result<Vec<String>> {
    list_versions_with(app_id, &mut |_| {})
}

pub fn list_versions_with(app_id: i64, progress: &mut dyn FnMut(&str)) -> Result<Vec<String>> {
    note(progress, "progress.loading_account");
    let account = require_account()?;
    let client = new_client()?;
    note(progress, "progress.fetch_versions");
    let out = runtime()
        .block_on(versions::list_versions(&client, app_id, &account))
        .map_err(map_err)?;
    Ok(out
        .versions
        .into_iter()
        .map(|v| v.external_version_id)
        .collect())
}

pub fn get_version_metadata(app_id: i64, external_version_id: &str) -> Result<VersionInfo> {
    get_version_metadata_with(app_id, external_version_id, &mut |_| {})
}

pub fn get_version_metadata_with(
    app_id: i64,
    external_version_id: &str,
    progress: &mut dyn FnMut(&str),
) -> Result<VersionInfo> {
    note(progress, "progress.version_meta");
    let account = require_account()?;
    let client = new_client()?;
    let meta = runtime()
        .block_on(versions::get_version_metadata(
            &client,
            app_id,
            &account,
            external_version_id,
        ))
        .map_err(map_err)?;
    Ok(VersionInfo {
        external_id: external_version_id.to_string(),
        display_version: meta
            .bundle_short_version
            .or(meta.bundle_version)
            .unwrap_or_default(),
        release_date: meta.release_date.unwrap_or_default(),
    })
}

pub fn download(req: &DownloadRequest) -> Result<String> {
    download_with(req, &mut |_| {}, false)
}

/// `show_file_progress` enables indicatif bar (needs a normal, non-raw TTY).
pub fn download_with(
    req: &DownloadRequest,
    progress: &mut dyn FnMut(&str),
    show_file_progress: bool,
) -> Result<String> {
    let app_id = req
        .app_id
        .ok_or_else(|| IpatoolError::msg("app id required"))?;
    note(progress, "progress.loading_account");
    let mut account = require_account()?;
    let client = new_client()?;

    // Match Go `ipatool download --purchase`: try download first; only call
    // buyProduct when Apple returns license-not-found. Eager purchase breaks
    // already-owned apps that now return failureType 2040 on buyProduct.
    let item = {
        let mut last_err: Option<ipatool_core::error::ClientError> = None;
        let mut acquired_license = false;
        let mut downloaded = None;

        for _ in 0..3 {
            if last_err.as_ref().is_some_and(|e| e.is_token_expired()) {
                if account.password.is_none() {
                    break;
                }
                note(progress, "progress.reauth");
                note(progress, "progress.fetch_bag");
                let bag_cfg = runtime()
                    .block_on(bag::fetch_bag(&client))
                    .map_err(map_err)?;
                note(progress, sap_progress_msg());
                let signer = runtime()
                    .block_on(sap::new_default_signer(
                        &client,
                        &bag_cfg.sap,
                        client.hardware_id(),
                    ))
                    .map_err(map_err)?;
                let pw = account.password.clone().unwrap();
                note(progress, "progress.auth");
                account = runtime()
                    .block_on(auth::login(
                        &client,
                        &account.email,
                        &pw,
                        None,
                        &bag_cfg.auth_endpoint,
                        Some(&signer as &dyn ActionSigner),
                    ))
                    .map_err(map_err)?;
                account.password = Some(pw);
                session::save_account(&account)?;
                save_cookies(&client);
            }

            if last_err.as_ref().is_some_and(|e| e.is_license_not_found())
                && req.purchase
                && !acquired_license
            {
                note(progress, "progress.ensure_license");
                match runtime().block_on(core_purchase::purchase(&client, app_id, &account)) {
                    Ok(()) => {}
                    Err(e) if e.is_license_already_exists() => {}
                    Err(e) => return Err(map_err(e)),
                }
                acquired_license = true;
                save_cookies(&client);
            }

            note(
                progress,
                if last_err.is_some() {
                    "progress.retry_download_info"
                } else {
                    "progress.download_info"
                },
            );
            match runtime().block_on(core_download::get_download_info(
                &client,
                app_id,
                &account,
                req.external_version_id.as_deref(),
            )) {
                Ok(item) => {
                    downloaded = Some(item);
                    break;
                }
                Err(e) => {
                    let retry = (e.is_token_expired() && account.password.is_some())
                        || (e.is_license_not_found() && req.purchase && !acquired_license);
                    last_err = Some(e);
                    if !retry {
                        break;
                    }
                }
            }
        }

        match downloaded {
            Some(item) => item,
            None => {
                return Err(map_err(
                    last_err.unwrap_or_else(|| {
                        ipatool_core::error::ClientError::UnexpectedResponse(
                            "download failed".into(),
                        )
                    }),
                ))
            }
        }
    };

    let out = req
        .output
        .clone()
        .unwrap_or_else(|| format!("{app_id}.ipa"));
    let out_path = PathBuf::from(&out);
    let tmp = out_path.with_extension("ipa.partial");

    note(progress, "progress.downloading_ipa");
    runtime()
        .block_on(core_download::download_file(
            &client,
            &item.url,
            &tmp,
            show_file_progress,
        ))
        .map_err(map_err)?;

    note(progress, "progress.patching");
    patch_ipa(&tmp, &out_path, &item, &account.email).map_err(map_err)?;
    let _ = std::fs::remove_file(&tmp);
    save_cookies(&client);
    note(progress, "progress.download_complete");
    Ok(out_path.display().to_string())
}

pub fn list_purchases() -> Result<Vec<OwnedApp>> {
    list_purchases_with(&mut |_| {})
}

pub fn list_purchases_with(progress: &mut dyn FnMut(&str)) -> Result<Vec<OwnedApp>> {
    note(progress, "progress.loading_account");
    let account = require_account()?;
    let client = new_client()?;

    note(progress, "progress.fetch_bag");
    let bag = runtime()
        .block_on(bag::fetch_bag(&client))
        .map_err(map_err)?;

    note(progress, sap_progress_msg());
    let signer = runtime()
        .block_on(sap::new_default_signer(
            &client,
            &bag.sap,
            client.hardware_id(),
        ))
        .map_err(map_err)?;

    note(progress, "progress.history_ios");
    let mut apps = runtime().block_on(purchases::fetch_owned_apps_store(
        &client, &account, &signer, "34",
    ))?;
    note(progress, "progress.history_mac");
    apps.extend(runtime().block_on(purchases::fetch_owned_apps_store(
        &client, &account, &signer, "13",
    ))?);
    let apps = purchases::merge_apps(apps);
    save_cookies(&client);
    note(
        progress,
        &format!("progress.history_ready\x1f{}", apps.len()),
    );
    Ok(apps)
}

pub fn download_to(path: &Path, app_id: i64, external_version_id: Option<&str>) -> Result<String> {
    download(&DownloadRequest {
        app_id: Some(app_id),
        bundle_id: None,
        output: Some(path.display().to_string()),
        external_version_id: external_version_id.map(str::to_string),
        purchase: true,
        keychain_passphrase: None,
    })
}

#[cfg(test)]
mod tests {
    use super::normalize_auth_code;

    #[test]
    fn auth_code_strips_spaces() {
        let c = normalize_auth_code(Some("12 34 56")).unwrap().unwrap();
        assert_eq!(c, "123456");
    }

    #[test]
    fn auth_code_rejects_non_digits() {
        assert!(normalize_auth_code(Some("12ab56")).is_err());
    }

    #[test]
    fn auth_code_empty_is_none() {
        assert!(normalize_auth_code(None).unwrap().is_none());
        assert!(normalize_auth_code(Some("   ")).unwrap().is_none());
    }
}
