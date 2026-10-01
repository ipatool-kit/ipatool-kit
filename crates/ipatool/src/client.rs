//! App Store client trait and shared types.

use serde::{Deserialize, Serialize};

use crate::error::Result;

pub use crate::helpers::{App, SearchResult};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthInfo {
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub storefront: String,
    #[serde(default)]
    pub success: bool,
}

#[derive(Clone, Default)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
    pub auth_code: Option<String>,
    pub keychain_passphrase: Option<String>,
}

impl std::fmt::Debug for LoginRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginRequest")
            .field("email", &self.email)
            .field("password", &"***")
            .field("auth_code", &self.auth_code.as_ref().map(|_| "***"))
            .field(
                "keychain_passphrase",
                &self.keychain_passphrase.as_ref().map(|_| "***"),
            )
            .finish()
    }
}

#[derive(Clone, Default)]
pub struct DownloadRequest {
    pub app_id: Option<i64>,
    pub bundle_id: Option<String>,
    pub output: Option<String>,
    pub external_version_id: Option<String>,
    pub purchase: bool,
    pub keychain_passphrase: Option<String>,
}

impl std::fmt::Debug for DownloadRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DownloadRequest")
            .field("app_id", &self.app_id)
            .field("bundle_id", &self.bundle_id)
            .field("output", &self.output)
            .field("external_version_id", &self.external_version_id)
            .field("purchase", &self.purchase)
            .field(
                "keychain_passphrase",
                &self.keychain_passphrase.as_ref().map(|_| "***"),
            )
            .finish()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct VersionInfo {
    #[serde(default)]
    pub external_id: String,
    #[serde(default)]
    pub display_version: String,
    #[serde(default)]
    pub release_date: String,
}

/// Abstraction over App Store operations.
pub trait AppStoreClient {
    fn login(&self, req: &LoginRequest) -> Result<AuthInfo>;
    fn auth_info(&self, keychain_passphrase: Option<&str>) -> Result<AuthInfo>;
    fn revoke(&self) -> Result<()>;
    fn search(
        &self,
        term: &str,
        limit: u32,
        keychain_passphrase: Option<&str>,
    ) -> Result<SearchResult>;
    fn purchase(
        &self,
        app_id: Option<i64>,
        bundle_id: Option<&str>,
        keychain_passphrase: Option<&str>,
    ) -> Result<()>;
    fn download(&self, req: &DownloadRequest) -> Result<String>;
    fn list_versions(
        &self,
        app_id: Option<i64>,
        bundle_id: Option<&str>,
        keychain_passphrase: Option<&str>,
    ) -> Result<Vec<String>>;
    fn get_version_metadata(
        &self,
        app_id: Option<i64>,
        bundle_id: Option<&str>,
        external_version_id: &str,
        keychain_passphrase: Option<&str>,
    ) -> Result<VersionInfo>;
    fn list_purchases(&self, keychain_passphrase: Option<&str>) -> Result<Vec<crate::OwnedApp>>;
}
