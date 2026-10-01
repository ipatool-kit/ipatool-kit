use std::collections::HashMap;

use crate::client::AppleClient;
use crate::error::{ClientError, StoreError};
use crate::model::Account;

const MAX_PURCHASE_ATTEMPTS: u32 = 3;

pub async fn purchase(
    client: &AppleClient,
    app_id: i64,
    account: &Account,
) -> Result<(), ClientError> {
    for attempt in 0..MAX_PURCHASE_ATTEMPTS {
        let result = match try_purchase(client, app_id, account, "STDQ").await {
            Err(ClientError::Store(StoreError::TemporarilyUnavailable)) => {
                tracing::info!("STDQ unavailable, trying GAME pricing");
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                try_purchase(client, app_id, account, "GAME").await
            }
            other => other,
        };

        match result {
            Err(ClientError::Store(StoreError::TemporarilyUnavailable))
                if attempt + 1 < MAX_PURCHASE_ATTEMPTS =>
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

async fn try_purchase(
    client: &AppleClient,
    app_id: i64,
    account: &Account,
    pricing_parameters: &str,
) -> Result<(), ClientError> {
    let url = buy_url(account);

    let mut body = plist::Dictionary::new();
    body.insert("appExtVrsId".into(), plist::Value::String("0".into()));
    body.insert(
        "hasAskedToFulfillPreorder".into(),
        plist::Value::String("true".into()),
    );
    body.insert(
        "hasDoneAgeCheck".into(),
        plist::Value::String("true".into()),
    );
    body.insert(
        "buyWithoutAuthorization".into(),
        plist::Value::String("true".into()),
    );
    body.insert(
        "guid".into(),
        plist::Value::String(client.guid().to_string()),
    );
    body.insert("needDiv".into(), plist::Value::String("0".into()));
    body.insert(
        "origPage".into(),
        plist::Value::String(format!("Software-{app_id}")),
    );
    body.insert(
        "origPageLocation".into(),
        plist::Value::String("Buy".into()),
    );
    body.insert("price".into(), plist::Value::String("0".into()));
    body.insert(
        "pricingParameters".into(),
        plist::Value::String(pricing_parameters.into()),
    );
    body.insert("productType".into(), plist::Value::String("C".into()));
    body.insert(
        "salableAdamId".into(),
        plist::Value::String(app_id.to_string()),
    );

    let mut body_bytes = Vec::new();
    plist::to_writer_xml(&mut body_bytes, &body)
        .map_err(|e| ClientError::UnexpectedResponse(format!("plist serialize: {e}")))?;

    let resp = client
        .http()
        .post(&url)
        .header("Content-Type", "application/x-apple-plist")
        .header("iCloud-DSID", &account.directory_services_id)
        .header("X-Dsid", &account.directory_services_id)
        .header("X-Apple-Store-Front", &account.store_front)
        .header("X-Token", &account.password_token)
        .body(body_bytes)
        .send()
        .await?;

    let http_status = resp.status();
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let resp_body = resp.bytes().await?;
    tracing::debug!(
        len = resp_body.len(),
        preview = %String::from_utf8_lossy(&resp_body[..resp_body.len().min(500)]),
        pricing = pricing_parameters,
        %http_status,
        "purchase response body"
    );

    // Go parity: bare HTTP 500 ⇒ license already exists.
    if http_status.as_u16() == 500 && resp_body.is_empty() {
        tracing::info!("purchase HTTP 500 empty — treating as license already exists");
        return Ok(());
    }

    let dict: HashMap<String, plist::Value> = crate::client::plist_xml::parse_plist_http(
        "purchase",
        &url,
        Some(http_status.as_u16()),
        content_type.as_deref(),
        &resp_body,
    )?;

    if let Some(err) = StoreError::from_plist_dict(&dict) {
        match err {
            StoreError::LicenseAlreadyExists => {
                tracing::info!("license already exists");
                return Ok(());
            }
            other => return Err(ClientError::Store(other)),
        }
    }

    if http_status.as_u16() == 500 {
        tracing::info!("purchase HTTP 500 — treating as license already exists");
        return Ok(());
    }

    // Match majd/ipatool: success requires jingleDocType=purchaseSuccess and status=0.
    let jingle = dict
        .get("jingleDocType")
        .and_then(plist::Value::as_string)
        .unwrap_or("");
    let status = dict.get("status").and_then(|v| match v {
        plist::Value::Integer(i) => i.as_signed(),
        plist::Value::String(s) => s.parse().ok(),
        _ => None,
    });

    if jingle != "purchaseSuccess" || status != Some(0) {
        return Err(ClientError::Store(StoreError::PurchaseFailed));
    }

    Ok(())
}

fn buy_url(account: &Account) -> String {
    let host = match &account.pod {
        Some(pod) => format!("p{pod}-buy.itunes.apple.com"),
        None => "buy.itunes.apple.com".to_string(),
    };
    format!("https://{host}/WebObjects/MZFinance.woa/wa/buyProduct")
}
