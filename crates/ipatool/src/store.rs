//! Sync facade over `ipatool-core` (SAP auth + download) and native DAAP purchases.

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
        let _ = client.save_cookies(&path);
    }
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
    session::ensure_dirs()?;
    let mut client = new_client()?;
    let account = runtime().block_on(async {
        let bag_cfg = bag::fetch_bag(&client).await.map_err(map_err)?;
        let signer = sap::new_default_signer(&client, &bag_cfg.sap, client.hardware_id())
            .await
            .map_err(map_err)?;
        auth::login(
            &client,
            &req.email,
            &req.password,
            req.auth_code.as_deref(),
            &bag_cfg.auth_endpoint,
            Some(&signer as &dyn ActionSigner),
        )
        .await
        .map_err(|e| {
            let s = e.to_string();
            if s.contains("auth code") || s.contains("2FA") || s.contains("verification") {
                IpatoolError::msg("auth code required")
            } else {
                map_err(e)
            }
        })
    })?;

    // Persist password for reauth on token expiry.
    let mut account = account;
    account.password = Some(req.password.clone());
    client.set_account(account.clone());
    session::save_account(&account)?;
    save_cookies(&client);

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
    let account = require_account()?;
    let client = new_client()?;
    runtime()
        .block_on(core_purchase::purchase(&client, app_id, &account))
        .map_err(map_err)?;
    save_cookies(&client);
    Ok(())
}

pub fn list_versions(app_id: i64) -> Result<Vec<String>> {
    let account = require_account()?;
    let client = new_client()?;
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
    let app_id = req
        .app_id
        .ok_or_else(|| IpatoolError::msg("app id required"))?;
    let mut account = require_account()?;
    let client = new_client()?;

    if req.purchase {
        let _ = runtime().block_on(core_purchase::purchase(&client, app_id, &account));
    }

    let item = runtime()
        .block_on(async {
            match core_download::get_download_info(
                &client,
                app_id,
                &account,
                req.external_version_id.as_deref(),
            )
            .await
            {
                Ok(item) => Ok(item),
                Err(e) => {
                    // Reauth once on token expiry if we still have a password.
                    let msg = e.to_string().to_lowercase();
                    if (msg.contains("token") || msg.contains("sign") || msg.contains("2034"))
                        && account.password.is_some()
                    {
                        let bag_cfg = bag::fetch_bag(&client).await.map_err(map_err)?;
                        let signer =
                            sap::new_default_signer(&client, &bag_cfg.sap, client.hardware_id())
                                .await
                                .map_err(map_err)?;
                        let pw = account.password.clone().unwrap();
                        account = auth::login(
                            &client,
                            &account.email,
                            &pw,
                            None,
                            &bag_cfg.auth_endpoint,
                            Some(&signer as &dyn ActionSigner),
                        )
                        .await
                        .map_err(map_err)?;
                        account.password = Some(pw);
                        session::save_account(&account)?;
                        core_download::get_download_info(
                            &client,
                            app_id,
                            &account,
                            req.external_version_id.as_deref(),
                        )
                        .await
                        .map_err(map_err)
                    } else {
                        Err(map_err(e))
                    }
                }
            }
        })?;

    let out = req
        .output
        .clone()
        .unwrap_or_else(|| format!("{app_id}.ipa"));
    let out_path = PathBuf::from(&out);
    let tmp = out_path.with_extension("ipa.partial");

    runtime()
        .block_on(core_download::download_file(
            &client,
            &item.url,
            &tmp,
            false,
        ))
        .map_err(map_err)?;

    patch_ipa(&tmp, &out_path, &item, &account.email).map_err(map_err)?;
    let _ = std::fs::remove_file(&tmp);
    save_cookies(&client);
    Ok(out_path.display().to_string())
}

pub fn list_purchases() -> Result<Vec<OwnedApp>> {
    let account = require_account()?;
    let client = new_client()?;
    let apps = runtime().block_on(async {
        let bag = bag::fetch_bag(&client).await.map_err(map_err)?;
        let signer = sap::new_default_signer(&client, &bag.sap, client.hardware_id())
            .await
            .map_err(map_err)?;
        purchases::fetch_owned_apps(&client, &account, &signer).await
    })?;
    save_cookies(&client);
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
