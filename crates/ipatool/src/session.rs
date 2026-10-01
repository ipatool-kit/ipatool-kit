//! Account session under `~/.ipatool/`.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use ipatool_core::model::Account;
use serde::{Deserialize, Serialize};

use crate::error::{IpatoolError, Result};

#[derive(Clone, Serialize, Deserialize)]
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
    /// Never written to disk (always None in AccountFile::from). Kept for
    /// reading legacy account.json that still contained a password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    password: Option<String>,
}

impl std::fmt::Debug for AccountFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountFile")
            .field("email", &self.email)
            .field("password_token", &"***")
            .field("directory_services_id", &self.directory_services_id)
            .field("name", &self.name)
            .field("store_front", &self.store_front)
            .field("pod", &self.pod)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .finish()
    }
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
            // Never persist Apple ID password in account.json.
            password: None,
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
            password: None,
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

fn chmod_private(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
}

/// Harden a secret file after an external writer (e.g. cookie jar) created it.
pub fn chmod_secret_file(path: &std::path::Path) {
    chmod_private(path);
}

fn chmod_private_dir(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
    }
}

pub fn ensure_dirs() -> Result<()> {
    let Some(root) = ipatool_dir() else {
        return Err(IpatoolError::msg("HOME not set"));
    };
    fs::create_dir_all(&root)?;
    chmod_private_dir(&root);
    if let Some(c) = cache_dir() {
        fs::create_dir_all(&c)?;
        chmod_private_dir(&c);
    }
    Ok(())
}

fn write_private(path: &std::path::Path, data: &[u8]) -> Result<()> {
    let mut f = fs::File::create(path)?;
    f.write_all(data)?;
    f.sync_all()?;
    chmod_private(path);
    Ok(())
}

pub fn save_account(account: &Account) -> Result<()> {
    ensure_dirs()?;
    let path = account_path().ok_or_else(|| IpatoolError::msg("HOME not set"))?;
    let file = AccountFile::from(account);
    let json = serde_json::to_string_pretty(&file)?;
    write_private(&path, json.as_bytes())?;
    // Keyring keeps full Account including password for silent reauth.
    let _ = ipatool_core::credential::store_account(account);
    Ok(())
}

pub fn load_account() -> Result<Option<Account>> {
    if let Some(path) = account_path() {
        if path.is_file() {
            let raw = fs::read_to_string(&path)?;
            if let Ok(file) = serde_json::from_str::<AccountFile>(&raw) {
                let mut acc: Account = file.into();
                // Overlay password from keyring (never from JSON).
                if let Ok(Some(kr)) = ipatool_core::credential::load_account() {
                    if kr.email == acc.email {
                        acc.password = kr.password;
                    }
                }
                // Scrub legacy plaintext password from disk on next opportunity.
                return Ok(Some(acc));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_file_never_serializes_password() {
        let acc = Account {
            email: "a@b.c".into(),
            password_token: "tok".into(),
            directory_services_id: "1".into(),
            name: "N".into(),
            store_front: "143441".into(),
            pod: None,
            password: Some("secret".into()),
        };
        let file = AccountFile::from(&acc);
        let json = serde_json::to_string(&file).unwrap();
        assert!(!json.contains("secret"));
        assert!(!json.contains("\"password\""));
    }
}


#[cfg(all(test, unix))]
mod unix_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn write_private_sets_mode_600() {
        let dir = std::env::temp_dir().join(format!("ipatool-session-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("secret.json");
        write_private(&path, b"{}").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = fs::remove_dir_all(&dir);
    }
}
