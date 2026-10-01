//! Account session under `~/.ipatool/`.

use std::fs;
use std::path::PathBuf;

use ipatool_core::model::Account;
use serde::{Deserialize, Serialize};

use crate::error::{IpatoolError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AccountFile {
    email: String,
    #[serde(rename = "passwordToken")]
    password_token: String,
    #[serde(rename = "directoryServicesIdentifier", alias = "directoryServicesID")]
    directory_services_id: String,
    #[serde(default)]
    name: String,
    #[serde(rename = "storeFront", alias = "storefront")]
    store_front: String,
    #[serde(default)]
    pod: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    password: Option<String>,
}

impl From<&Account> for AccountFile {
    fn from(a: &Account) -> Self {
        Self {
            email: a.email.clone(),
            password_token: a.password_token.clone(),
            directory_services_id: a.directory_services_id.clone(),
            name: a.name.clone(),
            store_front: a.store_front.clone(),
            pod: a.pod.clone(),
            password: a.password.clone(),
        }
    }
}

impl From<AccountFile> for Account {
    fn from(a: AccountFile) -> Self {
        Account {
            email: a.email,
            password_token: a.password_token,
            directory_services_id: a.directory_services_id,
            name: a.name,
            store_front: a.store_front,
            pod: a.pod,
            password: a.password,
        }
    }
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

pub fn ipatool_dir() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".ipatool"))
}

pub fn account_path() -> Option<PathBuf> {
    ipatool_dir().map(|d| d.join("account.json"))
}

pub fn cookies_path() -> Option<PathBuf> {
    ipatool_dir().map(|d| d.join("cookies.json"))
}

pub fn cache_dir() -> Option<PathBuf> {
    ipatool_dir().map(|d| d.join("cache"))
}

pub fn ensure_dirs() -> Result<()> {
    let Some(root) = ipatool_dir() else {
        return Err(IpatoolError::msg("HOME not set"));
    };
    fs::create_dir_all(&root)?;
    if let Some(c) = cache_dir() {
        fs::create_dir_all(c)?;
    }
    Ok(())
}

pub fn save_account(account: &Account) -> Result<()> {
    ensure_dirs()?;
    let path = account_path().ok_or_else(|| IpatoolError::msg("HOME not set"))?;
    let file = AccountFile::from(account);
    let json = serde_json::to_string_pretty(&file)?;
    fs::write(&path, json)?;
    // Best-effort keyring mirror (ipatool-rs compatible).
    let _ = ipatool_core::credential::store_account(account);
    Ok(())
}

pub fn load_account() -> Result<Option<Account>> {
    if let Some(path) = account_path() {
        if path.is_file() {
            let raw = fs::read_to_string(&path)?;
            if let Ok(file) = serde_json::from_str::<AccountFile>(&raw) {
                return Ok(Some(file.into()));
            }
        }
    }
    match ipatool_core::credential::load_account() {
        Ok(acc) => Ok(acc),
        Err(_) => Ok(None),
    }
}

pub fn delete_account() -> Result<()> {
    if let Some(path) = account_path() {
        let _ = fs::remove_file(path);
    }
    let _ = ipatool_core::credential::delete_account();
    Ok(())
}
