use std::collections::HashMap;
use std::time::Duration;
use url::Url;

use crate::client::AppleClient;
use crate::error::{ClientError, StoreError};
use crate::model::Account;
use crate::sap::ActionSigner;

const MAX_ATTEMPTS: u32 = 4;
const MAX_REDIRECTS: u32 = 5;
const AUTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Header carrying the SAP signature over the request body.
const ACTION_SIGNATURE_HEADER: &str = "X-Apple-ActionSignature";

pub async fn login(
    client: &AppleClient,
    email: &str,
    password: &str,
    auth_code: Option<&str>,
    auth_url: &Url,
    signer: Option<&dyn ActionSigner>,
) -> Result<Account, ClientError> {
    super::bag::validate_auth_endpoint(auth_url)?;

    let password_with_code = match auth_code {
        Some(code) => format!("{password}{code}"),
        None => password.to_string(),
    };

    let mut attempt = 1u32;
    let mut redirects = 0u32;
    let mut current_url = auth_url.clone();

    loop {
        let body = build_auth_plist(email, &password_with_code, client.guid(), attempt);
        let mut body_bytes = Vec::new();
        plist::to_writer_xml(&mut body_bytes, &body)
            .map_err(|e| ClientError::UnexpectedResponse(format!("plist serialize: {e}")))?;

        tracing::debug!(attempt, url = %current_url, signed = signer.is_some(), "sending auth request");

        let mut request = client
            .http_auth()
            .post(current_url.as_str())
            // Go majd/ipatool uses this Content-Type even though the body is XML plist.
            .header("Content-Type", "application/x-www-form-urlencoded")
            .timeout(AUTH_REQUEST_TIMEOUT);

        // Apple signs the exact bytes it receives, so this has to happen after
        // the body is serialized and before it is sent.
        if let Some(signer) = signer {
            let signature = signer.sign(&body_bytes)?;
            request = request.header(ACTION_SIGNATURE_HEADER, encode_base64(&signature));
        }

        let resp = match request.body(body_bytes).send().await {
            Ok(resp) => resp,
            Err(e) => {
                if http_status_retryable_from_reqwest(&e) && attempt < MAX_ATTEMPTS {
                    let delay = auth_backoff(attempt);
                    tracing::warn!(attempt, ?delay, error = %e, "auth transport error, retrying");
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                    continue;
                }
                return Err(e.into());
            }
        };

        let status = resp.status();
        tracing::debug!(%status, "auth response status");

        // Match Go retryableAuthenticationError: 204, 404, 429, 5xx.
        if is_retryable_auth_http(status.as_u16()) && attempt < MAX_ATTEMPTS {
            let delay = auth_retry_after(resp.headers()).unwrap_or_else(|| auth_backoff(attempt));
            tracing::warn!(
                attempt,
                status = status.as_u16(),
                ?delay,
                "auth HTTP retryable status, retrying"
            );
            // Drain body before retry.
            let _ = resp.bytes().await;
            tokio::time::sleep(delay).await;
            attempt += 1;
            continue;
        }

        // Go parseLoginResponse only follows HTTP 302 Found to the Store pod.
        // Other 3xx (e.g. edge 301 without Location) must not be treated as pod redirects.
        if status == reqwest::StatusCode::FOUND {
            let headers = format_headers(resp.headers());
            let location = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let _ = resp.bytes().await;

            let Some(location) = location.filter(|s| !s.trim().is_empty()) else {
                return Err(ClientError::UnexpectedResponse(format!(
                    "auth: HTTP 302 Found missing Location\nurl={current_url}\n{headers}"
                )));
            };

            redirects += 1;
            if redirects > MAX_REDIRECTS {
                return Err(ClientError::UnexpectedResponse(
                    "too many auth redirects".into(),
                ));
            }

            tracing::debug!(%location, "following auth pod redirect");
            let new_url = current_url.join(&location).map_err(|e| {
                ClientError::UnexpectedResponse(format!("redirect URL {location}: {e}"))
            })?;
            super::bag::validate_auth_endpoint(&new_url)?;
            // Pod redirect keeps the original attempt counter in the plist (Go).
            current_url = new_url;
            continue;
        }

        if status.is_redirection() {
            // Apple edge sometimes answers authenticate with HTML 301 and no
            // Location (majd/ipatool#520). Fresh TCP via http_auth + retry helps.
            if status.as_u16() == 301 && attempt < MAX_ATTEMPTS {
                let delay = auth_backoff(attempt);
                tracing::warn!(
                    attempt,
                    status = status.as_u16(),
                    ?delay,
                    "auth HTML 301 without usable Location, retrying on fresh connection"
                );
                let _ = resp.bytes().await;
                tokio::time::sleep(delay).await;
                attempt += 1;
                continue;
            }

            let headers = format_headers(resp.headers());
            let content_type = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let resp_body = resp.bytes().await?;
            return Err(ClientError::UnexpectedResponse(format!(
                "{}\n{}\nnote: Apple edge returned a non-302 redirect for authenticate; \
                 try again later or from another network/VPN (see majd/ipatool#520)",
                crate::client::plist_xml::describe_non_plist(
                    "auth",
                    current_url.as_str(),
                    Some(status.as_u16()),
                    content_type.as_deref(),
                    &resp_body,
                ),
                headers
            )));
        }

        let store_front = resp
            .headers()
            .get("x-set-apple-store-front")
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        let pod = resp
            .headers()
            .get("pod")
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);

        let headers = format_headers(resp.headers());
        let resp_body = resp.bytes().await?;

        tracing::debug!(
            len = resp_body.len(),
            preview = %String::from_utf8_lossy(&resp_body[..resp_body.len().min(500)]),
            "auth response body"
        );

        if resp_body.is_empty() {
            // Apple's application tier drops unsigned sign-in requests with a
            // zero-length 403. Saying so beats reporting an empty response.
            if signer.is_none() && status == reqwest::StatusCode::FORBIDDEN {
                return Err(ClientError::SapSignatureRequired {
                    status: status.as_u16(),
                });
            }

            return Err(ClientError::UnexpectedResponse(format!(
                "auth: empty response (HTTP {status}) url={current_url}\n{headers}"
            )));
        }

        let dict: HashMap<String, plist::Value> = crate::client::plist_xml::parse_plist_http(
            "auth",
            current_url.as_str(),
            Some(status.as_u16()),
            content_type.as_deref(),
            &resp_body,
        )
        .map_err(|e| match e {
            ClientError::UnexpectedResponse(msg) => {
                ClientError::UnexpectedResponse(format!("{msg}\n{headers}"))
            }
            other => other,
        })?;

        if let Some(err) = StoreError::from_plist_dict(&dict) {
            if err.is_retryable() && attempt < MAX_ATTEMPTS {
                attempt += 1;
                tracing::warn!("retryable error, attempt {attempt}");
                continue;
            }
            return Err(ClientError::Store(err));
        }

        let password_token = dict
            .get("passwordToken")
            .and_then(|v| v.as_string())
            .ok_or_else(|| ClientError::UnexpectedResponse("missing passwordToken".into()))?
            .to_string();

        let ds_person_id = dict
            .get("dsPersonId")
            .map(|v| match v {
                plist::Value::String(s) => s.clone(),
                plist::Value::Integer(i) => {
                    i.as_signed().map_or_else(String::new, |n| n.to_string())
                }
                _ => String::new(),
            })
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ClientError::UnexpectedResponse("missing dsPersonId".into()))?;

        let name = dict
            .get("accountInfo")
            .and_then(|v| v.as_dictionary())
            .and_then(|d| d.get("address"))
            .and_then(|v| v.as_dictionary())
            .and_then(|d| {
                let first = d.get("firstName")?.as_string()?;
                let last = d.get("lastName")?.as_string()?;
                Some(format!("{first} {last}"))
            })
            .unwrap_or_default();

        let sf = store_front.unwrap_or_default();

        return Ok(Account {
            email: email.to_string(),
            password_token,
            directory_services_id: ds_person_id,
            name,
            store_front: sf,
            pod,
            password: None,
        });
    }
}

fn encode_base64(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn format_headers(headers: &reqwest::header::HeaderMap) -> String {
    let mut lines: Vec<String> = headers
        .iter()
        .map(|(k, v)| format!("{}: {}", k, String::from_utf8_lossy(v.as_bytes())))
        .collect();
    lines.sort();
    if lines.is_empty() {
        "response-headers: (none)".into()
    } else {
        format!("response-headers:\n{}", lines.join("\n"))
    }
}

fn is_retryable_auth_http(status: u16) -> bool {
    matches!(status, 204 | 404 | 429) || (500..600).contains(&status)
}

fn http_status_retryable_from_reqwest(err: &reqwest::Error) -> bool {
    err.status()
        .map(|s| is_retryable_auth_http(s.as_u16()))
        .unwrap_or(false)
}

fn auth_backoff(attempt: u32) -> Duration {
    let secs = (1u64 << (attempt.saturating_sub(1).min(4))).min(30);
    Duration::from_secs(secs)
}

fn auth_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let raw = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    if let Ok(secs) = raw.parse::<u64>() {
        return Some(Duration::from_secs(secs.clamp(1, 60)));
    }
    None
}

fn build_auth_plist(email: &str, password: &str, guid: &str, attempt: u32) -> plist::Dictionary {
    let mut dict = plist::Dictionary::new();
    dict.insert("appleId".into(), plist::Value::String(email.into()));
    dict.insert("attempt".into(), plist::Value::String(attempt.to_string()));
    dict.insert("guid".into(), plist::Value::String(guid.into()));
    dict.insert("password".into(), plist::Value::String(password.into()));
    dict.insert("rmp".into(), plist::Value::String("0".into()));
    dict.insert("why".into(), plist::Value::String("signIn".into()));
    dict
}
