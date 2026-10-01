//! Native HTTP App Store client (stage 1: public iTunes Search; SAP ops NotImplemented).

use std::collections::BTreeMap;

use crate::client::{
    AppStoreClient, AuthInfo, DownloadRequest, LoginRequest, SearchResult, VersionInfo,
};
use crate::error::{IpatoolError, Result};
use crate::helpers::{build_query, parse_search_json};

/// Default client using the public iTunes Search/Lookup HTTP API.
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
    fn login(&self, _req: &LoginRequest) -> Result<AuthInfo> {
        Err(IpatoolError::NotImplemented { op: "auth login" })
    }

    fn auth_info(&self, _keychain_passphrase: Option<&str>) -> Result<AuthInfo> {
        Err(IpatoolError::NotImplemented { op: "auth info" })
    }

    fn revoke(&self) -> Result<()> {
        Err(IpatoolError::NotImplemented { op: "auth revoke" })
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
        _app_id: Option<i64>,
        _bundle_id: Option<&str>,
        _keychain_passphrase: Option<&str>,
    ) -> Result<()> {
        Err(IpatoolError::NotImplemented { op: "purchase" })
    }

    fn download(&self, _req: &DownloadRequest) -> Result<String> {
        Err(IpatoolError::NotImplemented { op: "download" })
    }

    fn list_versions(
        &self,
        _app_id: Option<i64>,
        _bundle_id: Option<&str>,
        _keychain_passphrase: Option<&str>,
    ) -> Result<Vec<String>> {
        Err(IpatoolError::NotImplemented {
            op: "list-versions",
        })
    }

    fn get_version_metadata(
        &self,
        _app_id: Option<i64>,
        _bundle_id: Option<&str>,
        _external_version_id: &str,
        _keychain_passphrase: Option<&str>,
    ) -> Result<VersionInfo> {
        Err(IpatoolError::NotImplemented {
            op: "get-version-metadata",
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sap_ops_are_not_implemented() {
        let c = HttpClient::new();
        assert!(matches!(
            c.login(&LoginRequest::default()),
            Err(IpatoolError::NotImplemented { .. })
        ));
        assert!(matches!(
            c.download(&DownloadRequest::default()),
            Err(IpatoolError::NotImplemented { .. })
        ));
    }
}
