//! Native App Store client (iTunes Search + authenticated ops via `store`).

use std::collections::BTreeMap;

use crate::client::{
    AppStoreClient, AuthInfo, DownloadRequest, LoginRequest, SearchResult, VersionInfo,
};
use crate::error::{IpatoolError, Result};
use crate::helpers::{build_query, parse_search_json};
use crate::store;

/// Default client using public iTunes Search plus native authenticated store ops.
#[derive(Debug, Default, Clone)]
pub struct HttpClient {
    country: String,
}

impl HttpClient {
    pub fn new() -> Self {
        Self {
            country: "us".into(),
        }
    }

    pub fn with_country(mut self, country: impl Into<String>) -> Self {
        self.country = country.into();
        self
    }
}

impl AppStoreClient for HttpClient {
    fn login(&self, req: &LoginRequest) -> Result<AuthInfo> {
        store::login(req)
    }

    fn auth_info(&self, _keychain_passphrase: Option<&str>) -> Result<AuthInfo> {
        store::auth_info()
    }

    fn revoke(&self) -> Result<()> {
        store::revoke()
    }

    fn search(
        &self,
        term: &str,
        limit: u32,
        _keychain_passphrase: Option<&str>,
    ) -> Result<SearchResult> {
        let mut params = BTreeMap::new();
        params.insert("term".into(), term.into());
        params.insert("entity".into(), "software,iPadSoftware".into());
        params.insert("limit".into(), limit.to_string());
        params.insert("country".into(), self.country.clone());
        let url = format!("https://itunes.apple.com/search?{}", build_query(&params));
        let body = http_get(&url)?;
        Ok(parse_search_json(&body))
    }

    fn purchase(
        &self,
        app_id: Option<i64>,
        _bundle_id: Option<&str>,
        _keychain_passphrase: Option<&str>,
    ) -> Result<()> {
        let id = app_id.ok_or_else(|| IpatoolError::msg("app id required"))?;
        store::purchase(id)
    }

    fn download(&self, req: &DownloadRequest) -> Result<String> {
        store::download(req)
    }

    fn list_versions(
        &self,
        app_id: Option<i64>,
        _bundle_id: Option<&str>,
        _keychain_passphrase: Option<&str>,
    ) -> Result<Vec<String>> {
        let id = app_id.ok_or_else(|| IpatoolError::msg("app id required"))?;
        store::list_versions(id)
    }

    fn get_version_metadata(
        &self,
        app_id: Option<i64>,
        _bundle_id: Option<&str>,
        external_version_id: &str,
        _keychain_passphrase: Option<&str>,
    ) -> Result<VersionInfo> {
        let id = app_id.ok_or_else(|| IpatoolError::msg("app id required"))?;
        store::get_version_metadata(id, external_version_id)
    }

    fn kbsync(
        &self,
        _refresh: bool,
        _keychain_passphrase: Option<&str>,
    ) -> Result<serde_json::Value> {
        Err(IpatoolError::NotImplemented { op: "kbsync" })
    }
}

fn http_get(url: &str) -> Result<String> {
    let resp = ureq::get(url)
        .call()
        .map_err(|e| IpatoolError::Http(e.to_string()))?;
    resp.into_string()
        .map_err(|e| IpatoolError::Http(e.to_string()))
}
