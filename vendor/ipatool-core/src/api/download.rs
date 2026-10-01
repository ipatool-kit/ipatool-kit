use std::collections::HashMap;
use std::path::Path;

use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;
use tokio::io::AsyncWriteExt;

use crate::client::AppleClient;
use crate::error::{ClientError, StoreError};
use crate::model::Account;

#[derive(Debug, Deserialize)]
pub struct DownloadItem {
    #[serde(rename = "URL")]
    pub url: String,
    pub sinfs: Vec<Sinf>,
    #[serde(default)]
    pub metadata: HashMap<String, plist::Value>,
}

impl DownloadItem {
    pub fn latest_external_version_id(&self) -> Option<String> {
        metadata_string(
            &self.metadata,
            &[
                "softwareVersionExternalIdentifier",
                "externalVersionIdentifier",
            ],
        )
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Sinf {
    pub id: i64,
    #[serde(with = "serde_bytes")]
    pub sinf: Vec<u8>,
}

mod serde_bytes {
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let val = plist::Value::deserialize(deserializer)?;
        match val {
            plist::Value::Data(d) => Ok(d),
            _ => Err(serde::de::Error::custom("expected data")),
        }
    }
}

const MAX_DOWNLOAD_ATTEMPTS: u32 = 3;

pub async fn get_download_info(
    client: &AppleClient,
    app_id: i64,
    account: &Account,
    external_version_id: Option<&str>,
) -> Result<DownloadItem, ClientError> {
    for attempt in 0..MAX_DOWNLOAD_ATTEMPTS {
        match try_get_download_info(client, app_id, account, external_version_id).await {
            Err(ClientError::Store(StoreError::TemporarilyUnavailable))
                if attempt + 1 < MAX_DOWNLOAD_ATTEMPTS =>
            {
                let delay = 5 * (attempt as u64 + 1);
                tracing::warn!(
                    attempt = attempt + 1,
                    delay,
                    "temporarily unavailable, retrying"
                );
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            }
            other => return other,
        }
    }
    Err(ClientError::Store(StoreError::TemporarilyUnavailable))
}

async fn try_get_download_info(
    client: &AppleClient,
    app_id: i64,
    account: &Account,
    external_version_id: Option<&str>,
) -> Result<DownloadItem, ClientError> {
    // 1) volumeStoreDownloadProduct (primary, matching Go).
    let volume = post_download_product(
        client,
        account,
        &volume_store_url(account, client.guid()),
        app_id,
        client.guid(),
        external_version_id,
        VersionKey::ExternalVersionId,
    )
    .await?;

    if !is_empty_or_unavailable(&volume) {
        return download_item_from_response_dict(&volume);
    }

    tracing::info!("volumeStore empty/unavailable — trying redownload/updateProduct fallback");

    // 2) Bag endpoints for redownload / updateProduct.
    let bag = crate::api::bag::fetch_bag(client).await?;
    let Some(redownload_base) = bag.redownload_endpoint.as_deref() else {
        return download_item_from_response_dict(&volume);
    };

    let mut version_id = external_version_id.map(str::to_string);
    // Unpinned redownload can return the wrong platform; pin latest iOS build.
    if version_id.is_none()
        && let Ok(list) = crate::api::versions::list_versions(client, app_id, account).await
    {
        version_id = list.latest_external_version_id.or_else(|| {
            list.versions
                .last()
                .map(|v| v.external_version_id.clone())
        });
    }

    let redownload_url = format!("{redownload_base}?guid={}", client.guid());
    let redownload = match post_download_product(
        client,
        account,
        &redownload_url,
        app_id,
        client.guid(),
        version_id.as_deref(),
        VersionKey::AppExtVrsId,
    )
    .await
    {
        Ok(dict) => dict,
        Err(e) => {
            // Empty HTTP 500 from redownload → try updateProduct when we have a version.
            if let (Some(update_base), Some(vid)) =
                (bag.update_endpoint.as_deref(), version_id.as_deref())
            {
                tracing::warn!(error = %e, "redownload failed - trying updateProduct");
                return post_and_parse_update(
                    client,
                    account,
                    update_base,
                    app_id,
                    client.guid(),
                    vid,
                )
                .await;
            }
            return Err(e);
        }
    };

    if !is_empty_or_unavailable(&redownload) {
        return download_item_from_response_dict(&redownload);
    }

    // 3) updateProduct when redownload is still empty/unavailable and version is known.
    if let (Some(update_base), Some(vid)) = (bag.update_endpoint.as_deref(), version_id.as_deref())
    {
        tracing::info!("redownload unavailable — trying updateProduct");
        return post_and_parse_update(client, account, update_base, app_id, client.guid(), vid)
            .await;
    }

    download_item_from_response_dict(&redownload)
}

#[derive(Clone, Copy)]
enum VersionKey {
    ExternalVersionId,
    AppExtVrsId,
}

impl VersionKey {
    fn as_str(self) -> &'static str {
        match self {
            Self::ExternalVersionId => "externalVersionId",
            Self::AppExtVrsId => "appExtVrsId",
        }
    }
}

fn volume_store_url(account: &Account, guid: &str) -> String {
    download_url(account, guid)
}

fn is_empty_or_unavailable(dict: &HashMap<String, plist::Value>) -> bool {
    if StoreError::from_plist_dict(dict).is_some() {
        return false;
    }
    let items_empty = dict
        .get("songList")
        .and_then(plist::Value::as_array)
        .map(|a| a.is_empty())
        .unwrap_or(true);
    if !items_empty {
        return false;
    }
    let msg = dict
        .get("customerMessage")
        .and_then(plist::Value::as_string)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    msg.is_empty()
        || msg == "no longer available"
        || msg.ends_with(" no longer available")
}

async fn post_download_product(
    client: &AppleClient,
    account: &Account,
    url: &str,
    app_id: i64,
    guid: &str,
    external_version_id: Option<&str>,
    version_key: VersionKey,
) -> Result<HashMap<String, plist::Value>, ClientError> {
    let mut body = plist::Dictionary::new();
    body.insert(
        "salableAdamId".into(),
        plist::Value::String(app_id.to_string()),
    );
    body.insert("guid".into(), plist::Value::String(guid.to_string()));
    body.insert("creditDisplay".into(), plist::Value::String(String::new()));
    // Match majd/ipatool wire format.
    body.insert("serialNumber".into(), plist::Value::String("0".into()));

    if let Some(vid) = external_version_id {
        body.insert(
            version_key.as_str().into(),
            plist::Value::String(vid.to_string()),
        );
    }

    let mut body_bytes = Vec::new();
    plist::to_writer_xml(&mut body_bytes, &body)
        .map_err(|e| ClientError::UnexpectedResponse(format!("plist serialize: {e}")))?;

    tracing::debug!(
        %url,
        dsid = %account.directory_services_id,
        store_front = %account.store_front,
        version_key = version_key.as_str(),
        "download product request"
    );

    let resp = client
        .http()
        .post(url)
        .header("Content-Type", "application/x-apple-plist")
        .header("iCloud-DSID", &account.directory_services_id)
        .header("X-Dsid", &account.directory_services_id)
        .body(body_bytes)
        .send()
        .await?;

    let status = resp.status();
    tracing::debug!(%status, "download product response status");

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let resp_body = resp.bytes().await?;
    crate::client::plist_xml::parse_plist_http(
        "download",
        url,
        Some(status.as_u16()),
        content_type.as_deref(),
        &resp_body,
    )
}

async fn post_and_parse_update(
    client: &AppleClient,
    account: &Account,
    update_base: &str,
    app_id: i64,
    guid: &str,
    external_version_id: &str,
) -> Result<DownloadItem, ClientError> {
    let url = format!("{update_base}?guid={guid}");
    let dict = post_download_product(
        client,
        account,
        &url,
        app_id,
        guid,
        Some(external_version_id),
        VersionKey::AppExtVrsId,
    )
    .await?;
    download_item_from_response_dict(&dict)
}

fn download_item_from_response_dict(
    dict: &HashMap<String, plist::Value>,
) -> Result<DownloadItem, ClientError> {
    match StoreError::from_plist_dict(dict) {
        Some(StoreError::LicenseAlreadyExists) => {
            if let Some(item) = download_item_from_dict(dict)? {
                Ok(item)
            } else {
                Err(ClientError::Store(StoreError::PasswordTokenExpired))
            }
        }
        Some(err) => Err(ClientError::Store(err)),
        None => {
            if let Some(item) = download_item_from_dict(dict)? {
                // Reject obvious macOS pkg URLs for the IPA-focused toolkit path.
                if item.url.contains(".pkg") || item.url.contains("/mac/") {
                    return Err(ClientError::UnexpectedResponse(
                        "macOS package download is not supported in this build (IPA only); pin an iOS version"
                            .into(),
                    ));
                }
                Ok(item)
            } else {
                Err(ClientError::UnexpectedResponse("missing songList".into()))
            }
        }
    }
}

fn download_item_from_dict(
    dict: &HashMap<String, plist::Value>,
) -> Result<Option<DownloadItem>, ClientError> {
    let Some(song_list) = dict.get("songList") else {
        return Ok(None);
    };

    let song_list = song_list
        .as_array()
        .ok_or_else(|| ClientError::UnexpectedResponse("invalid songList".into()))?;

    let first = song_list
        .first()
        .ok_or_else(|| ClientError::UnexpectedResponse("empty songList".into()))?;

    plist::from_value(first)
        .map(Some)
        .map_err(ClientError::PlistDe)
}

fn metadata_string(metadata: &HashMap<String, plist::Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| metadata.get(*key).and_then(metadata_value_to_string))
}

fn metadata_value_to_string(value: &plist::Value) -> Option<String> {
    match value {
        plist::Value::String(s) => Some(s.clone()),
        plist::Value::Integer(i) => i.as_signed().map(|n| n.to_string()),
        _ => None,
    }
}

pub async fn download_file(
    client: &AppleClient,
    url: &str,
    dest: &Path,
    show_progress: bool,
) -> Result<(), ClientError> {
    let existing_size = if dest.exists() {
        tokio::fs::metadata(dest)
            .await
            .map(|m| m.len())
            .unwrap_or(0)
    } else {
        0
    };

    let mut req = client.http().get(url);
    if existing_size > 0 {
        req = req.header("Range", format!("bytes={existing_size}-"));
        tracing::info!(existing_size, "resuming download");
    }

    let resp = req.send().await?;

    if resp.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        tracing::info!("file already fully downloaded");
        return Ok(());
    }

    let total_size = resp.content_length().map(|cl| cl + existing_size);

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dest)
        .await
        .map_err(|e| ClientError::UnexpectedResponse(format!("open file: {e}")))?;

    let pb = if show_progress {
        let pb = match total_size {
            Some(total) => {
                let pb = ProgressBar::new(total);
                pb.set_style(
                    ProgressStyle::default_bar()
                        .template("{msg} [{bar:40}] {bytes}/{total_bytes} ({eta})")
                        .unwrap()
                        .progress_chars("=> "),
                );
                pb.set_position(existing_size);
                pb
            }
            None => {
                let pb = ProgressBar::new_spinner();
                pb.set_style(
                    ProgressStyle::default_spinner()
                        .template("{msg} {bytes} {elapsed}")
                        .unwrap(),
                );
                pb
            }
        };
        pb.set_message("Downloading");
        Some(pb)
    } else {
        None
    };

    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk)
            .await
            .map_err(|e| ClientError::UnexpectedResponse(format!("write: {e}")))?;
        if let Some(ref pb) = pb {
            pb.inc(chunk.len() as u64);
        }
    }

    file.flush()
        .await
        .map_err(|e| ClientError::UnexpectedResponse(format!("flush: {e}")))?;

    if let Some(pb) = pb {
        pb.finish_with_message("Download complete");
    }

    Ok(())
}

fn download_url(account: &Account, guid: &str) -> String {
    let host = match &account.pod {
        Some(pod) => format!("p{pod}-buy.itunes.apple.com"),
        None => "buy.itunes.apple.com".to_string(),
    };
    format!("https://{host}/WebObjects/MZFinance.woa/wa/volumeStoreDownloadProduct?guid={guid}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_download_item() -> plist::Value {
        let mut sinf = plist::Dictionary::new();
        sinf.insert("id".into(), plist::Value::Integer(1.into()));
        sinf.insert("sinf".into(), plist::Value::Data(vec![1, 2, 3]));

        let mut item = plist::Dictionary::new();
        item.insert(
            "URL".into(),
            plist::Value::String("https://example.invalid/app.ipa".into()),
        );
        item.insert(
            "sinfs".into(),
            plist::Value::Array(vec![plist::Value::Dictionary(sinf)]),
        );

        plist::Value::Dictionary(item)
    }

    #[test]
    fn download_item_wins_over_license_already_exists_marker() {
        let mut dict = HashMap::new();
        dict.insert("failureType".into(), plist::Value::String("5002".into()));
        dict.insert(
            "customerMessage".into(),
            plist::Value::String("license already exists".into()),
        );
        dict.insert(
            "songList".into(),
            plist::Value::Array(vec![sample_download_item()]),
        );

        let item = download_item_from_response_dict(&dict).unwrap();

        assert_eq!(item.url, "https://example.invalid/app.ipa");
        assert_eq!(item.sinfs[0].id, 1);
        assert_eq!(item.sinfs[0].sinf, vec![1, 2, 3]);
    }

    #[test]
    fn bare_license_already_exists_requires_reauth_for_download() {
        let mut dict = HashMap::new();
        dict.insert("failureType".into(), plist::Value::String("5002".into()));
        dict.insert(
            "customerMessage".into(),
            plist::Value::String("license already exists".into()),
        );

        let err = download_item_from_response_dict(&dict).unwrap_err();

        assert!(err.is_token_expired());
    }

    #[test]
    fn download_item_reads_current_latest_version_id() {
        let mut item = plist::Dictionary::new();
        item.insert(
            "URL".into(),
            plist::Value::String("https://example.invalid/app.ipa".into()),
        );
        item.insert("sinfs".into(), plist::Value::Array(Vec::new()));
        let mut metadata = plist::Dictionary::new();
        metadata.insert(
            "softwareVersionExternalIdentifier".into(),
            plist::Value::Integer(887118713.into()),
        );
        item.insert("metadata".into(), plist::Value::Dictionary(metadata));

        let item: DownloadItem = plist::from_value(&plist::Value::Dictionary(item)).unwrap();

        assert_eq!(
            item.latest_external_version_id().as_deref(),
            Some("887118713")
        );
    }

    #[test]
    fn download_item_reads_legacy_latest_version_id() {
        let mut item = plist::Dictionary::new();
        item.insert(
            "URL".into(),
            plist::Value::String("https://example.invalid/app.ipa".into()),
        );
        item.insert("sinfs".into(), plist::Value::Array(Vec::new()));
        let mut metadata = plist::Dictionary::new();
        metadata.insert(
            "externalVersionIdentifier".into(),
            plist::Value::String("12345678".into()),
        );
        item.insert("metadata".into(), plist::Value::Dictionary(metadata));

        let item: DownloadItem = plist::from_value(&plist::Value::Dictionary(item)).unwrap();

        assert_eq!(
            item.latest_external_version_id().as_deref(),
            Some("12345678")
        );
    }

    #[test]
    fn download_item_does_not_guess_latest_version_id_from_array() {
        let mut item = plist::Dictionary::new();
        item.insert(
            "URL".into(),
            plist::Value::String("https://example.invalid/app.ipa".into()),
        );
        item.insert("sinfs".into(), plist::Value::Array(Vec::new()));
        let mut metadata = plist::Dictionary::new();
        metadata.insert(
            "softwareVersionExternalIdentifiers".into(),
            plist::Value::Array(vec![plist::Value::Integer(887118713.into())]),
        );
        item.insert("metadata".into(), plist::Value::Dictionary(metadata));

        let item: DownloadItem = plist::from_value(&plist::Value::Dictionary(item)).unwrap();

        assert_eq!(item.latest_external_version_id(), None);
    }

    #[test]
    fn empty_song_list_without_message_is_empty() {
        let dict = HashMap::new();
        assert!(is_empty_or_unavailable(&dict));
    }

    #[test]
    fn no_longer_available_message_is_unavailable() {
        let mut dict = HashMap::new();
        dict.insert(
            "customerMessage".into(),
            plist::Value::String("No Longer Available".into()),
        );
        assert!(is_empty_or_unavailable(&dict));
    }

    #[test]
    fn song_list_present_is_not_empty() {
        let mut dict = HashMap::new();
        dict.insert(
            "songList".into(),
            plist::Value::Array(vec![sample_download_item()]),
        );
        assert!(!is_empty_or_unavailable(&dict));
    }
}
