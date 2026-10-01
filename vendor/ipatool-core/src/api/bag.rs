use std::collections::HashMap;
use std::time::Duration;

use url::Url;

use crate::client::AppleClient;
use crate::error::ClientError;
use crate::sap::SapConfig;

const BAG_HOST_PATH: &str = "https://init.itunes.apple.com/bag.xml";
const BAG_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// The only sign-in action Apple's bag advertises.
pub const AUTH_PATH: &str = "/WebObjects/MZFinance.woa/wa/authenticate";

const AUTH_HOST: &str = "buy.itunes.apple.com";
/// Account-specific pods, e.g. `p14-buy.itunes.apple.com`.
const AUTH_POD_SUFFIX: &str = "-buy.itunes.apple.com";

#[derive(Debug, Clone)]
pub struct BagConfig {
    pub auth_endpoint: Url,
    pub sap: SapConfig,
    /// `redownloadProduct` from the bag (download fallback).
    pub redownload_endpoint: Option<String>,
    /// `updateProduct` from the bag (download fallback after redownload).
    pub update_endpoint: Option<String>,
}

pub async fn fetch_bag(client: &AppleClient) -> Result<BagConfig, ClientError> {
    fetch_bag_with_guid(client, client.guid()).await
}

/// Same wire shape as majd/ipatool `appstore_bag.go`:
/// `GET …/bag.xml?guid=` with Configurator UA → top-level `urlBag` dict.
pub async fn fetch_bag_with_guid(
    client: &AppleClient,
    guid: &str,
) -> Result<BagConfig, ClientError> {
    let url = format!("{BAG_HOST_PATH}?guid={guid}");
    let resp = client
        .http()
        .get(&url)
        .header("Accept", "application/xml")
        .timeout(BAG_REQUEST_TIMEOUT)
        .send()
        .await?;
    let status = resp.status();
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body = resp.bytes().await?;

    let outer: HashMap<String, plist::Value> = crate::client::plist_xml::parse_plist_http(
        "bag",
        &url,
        Some(status.as_u16()),
        content_type.as_deref(),
        &body,
    )?;

    let url_bag = outer
        .get("urlBag")
        .and_then(plist::Value::as_dictionary)
        .ok_or_else(|| ClientError::UnexpectedResponse("bag: missing urlBag".into()))?;

    let inner: HashMap<String, plist::Value> = url_bag
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    parse_bag(&inner)
}

fn parse_bag(inner: &HashMap<String, plist::Value>) -> Result<BagConfig, ClientError> {
    let auth_endpoint = bag_url(inner, "authenticateAccount")?;
    validate_auth_endpoint(&auth_endpoint)?;

    // Apple publishes the version as a string.
    let version_str = inner
        .get("sign-sap-version")
        .and_then(plist::Value::as_string)
        .ok_or_else(|| ClientError::UnexpectedResponse("bag: missing sign-sap-version".into()))?;

    let version = version_str.parse::<u32>().map_err(|e| {
        ClientError::UnexpectedResponse(format!(
            "bag: invalid sign-sap-version {version_str:?}: {e}"
        ))
    })?;

    let sap = SapConfig {
        setup_url: bag_url(inner, "sign-sap-setup")?,
        cert_url: bag_url(inner, "sign-sap-setup-cert")?,
        version,
    };
    sap.validate()?;

    let redownload_endpoint = inner
        .get("redownloadProduct")
        .and_then(plist::Value::as_string)
        .map(str::to_string);
    let update_endpoint = inner
        .get("updateProduct")
        .and_then(plist::Value::as_string)
        .map(str::to_string);

    Ok(BagConfig {
        auth_endpoint,
        sap,
        redownload_endpoint,
        update_endpoint,
    })
}

fn bag_url(inner: &HashMap<String, plist::Value>, key: &str) -> Result<Url, ClientError> {
    let raw = inner
        .get(key)
        .and_then(plist::Value::as_string)
        .ok_or_else(|| ClientError::UnexpectedResponse(format!("bag: missing {key}")))?;

    Url::parse(raw)
        .map_err(|e| ClientError::UnexpectedResponse(format!("bag: invalid {key} URL: {e}")))
}

/// Credentials are POSTed to this URL, and Apple redirects sign-in to
/// account-specific pods, so every candidate — from the bag and from a redirect
/// alike — is checked against the shape Apple actually uses.
pub fn validate_auth_endpoint(url: &Url) -> Result<(), ClientError> {
    if url.scheme() != "https" {
        return Err(ClientError::UnexpectedResponse(format!(
            "unsupported authentication endpoint {url}: not HTTPS"
        )));
    }

    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if host != AUTH_HOST && !host.ends_with(AUTH_POD_SUFFIX) {
        return Err(ClientError::UnexpectedResponse(format!(
            "unsupported authentication endpoint {url}: unexpected host"
        )));
    }

    if url.path().trim_end_matches('/') != AUTH_PATH {
        return Err(ClientError::UnexpectedResponse(format!(
            "unsupported authentication endpoint {url}: unexpected path"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    /// Mirrors Configurator-UA `urlBag` from init.itunes (majd/ipatool shape).
    fn current_bag() -> HashMap<String, plist::Value> {
        HashMap::from([
            (
                "authenticateAccount".to_string(),
                plist::Value::String(
                    "https://buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/authenticate".into(),
                ),
            ),
            (
                "sign-sap-setup".to_string(),
                plist::Value::String(
                    "https://fpinit.itunes.apple.com/v1/signSapSetup/legacy".into(),
                ),
            ),
            (
                "sign-sap-setup-cert".to_string(),
                plist::Value::String("https://s.mzstatic.com/sap/setupCert.plist".into()),
            ),
            (
                "sign-sap-version".to_string(),
                plist::Value::String("200".into()),
            ),
            (
                "redownloadProduct".to_string(),
                plist::Value::String("https://downloaddispatch.itunes.apple.com/r/redownload".into()),
            ),
            (
                "updateProduct".to_string(),
                plist::Value::String(
                    "https://downloaddispatch.itunes.apple.com/up/updateProduct".into(),
                ),
            ),
        ])
    }

    #[test]
    fn parses_apples_current_bag() {
        let config = parse_bag(&current_bag()).unwrap();

        assert_eq!(
            config.auth_endpoint.as_str(),
            "https://buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/authenticate"
        );
        assert_eq!(
            config.sap.setup_url.as_str(),
            "https://fpinit.itunes.apple.com/v1/signSapSetup/legacy"
        );
        assert_eq!(config.sap.version, 200);
        assert!(config
            .redownload_endpoint
            .as_deref()
            .unwrap()
            .contains("redownload"));
        assert!(config
            .update_endpoint
            .as_deref()
            .unwrap()
            .contains("updateProduct"));
    }

    #[test]
    fn rejects_native_fast_authenticate_account() {
        let mut bag = current_bag();
        bag.insert(
            "authenticateAccount".into(),
            plist::Value::String("https://auth.itunes.apple.com/auth/v1/native/fast".into()),
        );

        assert!(parse_bag(&bag).is_err());
    }

    #[test]
    fn fails_when_sap_keys_are_absent() {
        let mut bag = current_bag();
        bag.remove("sign-sap-setup");

        assert!(parse_bag(&bag).is_err());
    }

    #[test]
    fn accepts_pod_hosts() {
        assert!(validate_auth_endpoint(&url(
            "https://p14-buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/authenticate"
        ))
        .is_ok());
    }

    #[test]
    fn accepts_trailing_slash() {
        assert!(validate_auth_endpoint(&url(
            "https://buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/authenticate/"
        ))
        .is_ok());
    }

    #[test]
    fn rejects_native_fast_endpoint() {
        assert!(
            validate_auth_endpoint(&url("https://auth.itunes.apple.com/auth/v1/native/fast/"))
                .is_err()
        );
    }

    #[test]
    fn rejects_lookalike_host() {
        assert!(validate_auth_endpoint(&url(
            "https://buy.itunes.apple.com.evil.test/WebObjects/MZFinance.woa/wa/authenticate"
        ))
        .is_err());
    }

    #[test]
    fn rejects_other_action_on_valid_host() {
        assert!(validate_auth_endpoint(&url(
            "https://buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/buyProduct"
        ))
        .is_err());
    }

    #[test]
    fn rejects_plaintext() {
        assert!(validate_auth_endpoint(&url(
            "http://buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/authenticate"
        ))
        .is_err());
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn live_fetch_bag_urlbag_shape() {
        let client = crate::client::AppleClient::for_tests();
        let cfg = fetch_bag(&client).await.expect("fetch bag");
        assert!(
            cfg.auth_endpoint
                .as_str()
                .contains("/WebObjects/MZFinance.woa/wa/authenticate"),
            "auth={}",
            cfg.auth_endpoint
        );
        assert_eq!(cfg.sap.version, 200);
        eprintln!(
            "live bag ok auth={} sap={}",
            cfg.auth_endpoint, cfg.sap.setup_url
        );
    }
}
