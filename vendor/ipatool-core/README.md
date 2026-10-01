# ipatool-kit-core

Low-level Rust client for Apple’s App Store **Configurator** wire protocol: store bag, SAP signing, Apple ID login (incl. 2FA), free license purchase, and IPA download/patch.

This is the engine behind [`ipatool-kit`](https://crates.io/crates/ipatool-kit). Use **`ipatool-kit`** for the TUI and a simpler sync façade. Depend on **this crate** when you need async control over bag / SAP / auth / download yourself.

| | |
|---|---|
| Crate | `ipatool-kit-core` |
| Rust lib name | `ipatool_core` |
| Edition | 2024 |
| License | MIT |

```toml
[dependencies]
ipatool-kit-core = "0.2"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

> Not a DRM cracker. It only talks to Apple with credentials for an account you control, for apps that account is entitled to.

## What it does

1. **`api::bag`** — `GET init.itunes.apple.com/bag.xml?guid=…`, parse `urlBag`, pick authenticate / SAP / redownload endpoints (same shape as [majd/ipatool](https://github.com/majd/ipatool)).
2. **`sap`** — download & cache Apple frameworks, run SAP handshake, produce `ActionSigner` for `X-Apple-ActionSignature`.
3. **`api::auth`** — XML-plist login over HTTP/1.1 (no keep-alive on the auth client), follow only auth `302`s the Go client follows, return `Account` (DSID, password token, storefront, …).
4. **`api::purchase`** — obtain a free license when needed (`STDQ` / `GAME` retries).
5. **`api::download`** — `volumeStoreDownloadProduct` (with bag fallbacks), stream IPA to disk (Range resume), then **`ipa::patch`** to inject metadata / SINF.

Also: `api::search`, `api::lookup`, `api::versions`, `api::reauth`, cookie jar, machine GUID from MAC.

## Minimal example: login → download → patch

Replace email/password/app id. First SAP run downloads large framework blobs into `cache_dir` (later runs reuse them).

```rust
use std::path::PathBuf;

use ipatool_core::api::{auth, bag, download, purchase};
use ipatool_core::client::AppleClient;
use ipatool_core::guid::generate_machine_identity;
use ipatool_core::ipa::patch::patch_ipa;
use ipatool_core::sap::{self, ActionSigner};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let email = std::env::var("APPLE_ID")?;
    let password = std::env::var("APPLE_PASSWORD")?;
    // Optional 6-digit 2FA code (concatenated onto the password on the wire).
    let auth_code = std::env::var("APPLE_2FA").ok();

    let app_id: i64 = std::env::var("APP_ID")?.parse()?;
    let out = PathBuf::from(format!("{app_id}.ipa"));

    let home = dirs_next_home(); // see helper below
    let cache = home.join(".ipatool/cache");
    let cookies = home.join(".ipatool/cookies.json");
    std::fs::create_dir_all(&cache)?;

    let machine = generate_machine_identity()?;
    let client = AppleClient::new(machine, Some(cookies.as_path()), &cache)?;

    // 1) Store bag → auth URL + SAP config
    let bag_cfg = bag::fetch_bag(&client).await?;

    // 2) SAP signer (bound to this machine's hardware id)
    let signer = sap::new_default_signer(&client, &bag_cfg.sap, client.hardware_id()).await?;

    // 3) Login
    let account = auth::login(
        &client,
        &email,
        &password,
        auth_code.as_deref(),
        &bag_cfg.auth_endpoint,
        Some(&signer as &dyn ActionSigner),
    )
    .await?;

    println!(
        "logged in as {} ({}) storefront={}",
        account.email, account.name, account.store_front
    );
    let _ = client.save_cookies(&cookies);

    // 4) Ensure free license (safe no-op / retry if already owned — handle errors as you like)
    match purchase::purchase(&client, app_id, &account).await {
        Ok(()) => println!("purchase/license ok"),
        Err(e) => eprintln!("purchase skipped/failed: {e}"),
    }

    // 5) Resolve CDN URL + SINF, download, patch IPA
    let item = download::get_download_info(&client, app_id, &account, None).await?;
    let tmp = PathBuf::from(format!("{app_id}.download.ipa"));
    download::download_file(&client, &item.url, &tmp, true).await?;
    patch_ipa(&tmp, &out, &item, &account.email)?;
    let _ = std::fs::remove_file(&tmp);

    println!("wrote {}", out.display());
    Ok(())
}

fn dirs_next_home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .expect("HOME not set")
}
```

### 2FA

Pass the six-digit code as `auth_code`. The client sends `password + code` in the plist body (Apple’s Configurator style). If Apple returns a 2FA challenge, retry the same `login` call with the code.

### Session reuse

Persist `cookies.json` via `AppleClient::save_cookies` and reload with `AppleClient::new(..., Some(cookie_path), ...)`. Persist `Account` yourself (JSON) if you want to skip interactive login on the next run; tokens expire — use `api::reauth` / login again when Apple rejects them.

## Public modules

| Module | Role |
|--------|------|
| `client::AppleClient` | HTTP clients (general + auth), cookies, GUID / hardware id, cache dir |
| `guid` | `generate_machine_identity()` — GUID + SAP hardware id from MAC |
| `api::bag` | Fetch & validate store bag |
| `sap` | Framework cache, emulator, `new_default_signer`, `ActionSigner` |
| `api::auth` | Apple ID login |
| `api::purchase` | Free license |
| `api::download` | Download info + streaming file |
| `api::search` / `lookup` / `versions` | Store metadata helpers |
| `api::reauth` | Refresh expired session |
| `ipa::patch` | Inject purchase metadata / SINF into the IPA |
| `model::Account` | Session fields Apple returns |
| `error` | `ClientError`, `StoreError`, … |

## Cache layout

On first SAP setup the crate downloads Apple frameworks into `cache_dir` (in `ipatool-kit` that is `~/.ipatool/cache/`). Expect hundreds of MB once; afterward startup is much faster if digests still match.

## Relation to `ipatool-kit`

- **`ipatool-kit`** — sync helpers (`store::login`, `store::download`, DAAP purchase history), interactive `ipatool-kit` binary.
- **`ipatool-kit-core`** — async primitives those helpers call.

If you only need “log in and download an IPA”, start with `ipatool-kit`. If you are embedding App Store flows in your own async service, use this crate.

## Safety / legal

Use only with Apple IDs you own and in line with Apple’s terms and local law. Do not commit cookies, password tokens, or account dumps.

## License

MIT — see [LICENSE](LICENSE).
