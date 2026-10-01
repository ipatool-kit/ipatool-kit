# ipatool-kit

Interactive terminal toolkit and Rust library for browsing your App Store library and downloading IPA packages you already own — including apps removed from the store when your account still has a license.

> Does **not** crack DRM or bypass Apple licensing. Only apps your Apple ID is entitled to.

<p align="center">
  <img src="https://raw.githubusercontent.com/ipatool-kit/ipatool-kit/main/assets/demo.gif" alt="ipatool-kit demo (sample data)" width="840">
</p>

<p align="center"><sub>Terminal demo (sample session).</sub></p>

[![crates.io](https://img.shields.io/crates/v/ipatool-kit.svg)](https://crates.io/crates/ipatool-kit)
[![docs.rs](https://img.shields.io/docsrs/ipatool-kit)](https://docs.rs/ipatool-kit)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## Install (TUI binary)

### From crates.io

```bash
cargo install ipatool-kit
ipatool-kit
```

### From GitHub Releases

1. Download the binary for your OS from [Releases](https://github.com/ipatool-kit/ipatool-kit/releases).
2. Make it executable and run:

```bash
chmod +x ipatool-kit-*
./ipatool-kit-*
```

### macOS Gatekeeper

If macOS blocks the binary (“damaged” / can’t be opened), clear quarantine:

```bash
xattr -cr ipatool-kit-*
```

Then run it again.

## Use as a library

```bash
cargo add ipatool-kit
```

```rust
use ipatool_kit::{AppStoreClient, HttpClient};

fn main() -> ipatool_kit::Result<()> {
    let client = HttpClient::new().with_country("us");
    let found = client.search("pages", 5, None)?;
    for app in found.results {
        println!("{}  {}  ({})", app.id, app.name, app.bundle_id);
    }
    Ok(())
}
```

Authenticated flows (login, download, purchase history) live under `ipatool_kit::store` and `ipatool_kit::purchases`. See [docs.rs/ipatool-kit](https://docs.rs/ipatool-kit).

Run the bundled example:

```bash
cargo run -p ipatool-kit --example search -- "vk"
```

### Crate layout

| Crate | Role |
|-------|------|
| [`ipatool-kit`](https://crates.io/crates/ipatool-kit) | Public library + `ipatool-kit` TUI binary |
| [`ipatool-kit-core`](https://crates.io/crates/ipatool-kit-core) | Low-level SAP/auth/download (usually transitive) |

## Features

- Search / enter IDs / browse lists
- Apple purchase history with cache
- Delisted IDs menu (`Apps_ID_List` − Apple history)
- Multi-select + live search; `*` = all visible
- Data under `~/.ipatool/downloader/`
- Built-in App Store auth / download (no extra CLI tools)
- Optional install via `ideviceinstaller`

## Environment

| Variable | Meaning |
|----------|---------|
| `IPATOOL_COUNTRY` | Storefront country (default `us`) |
| `IPA_DOWNLOADER_HOME` | Override data root (default `~/.ipatool/downloader`) |

## Data layout

```text
~/.ipatool/downloader/
  Apps/    # downloaded IPAs
  Files/   # lists + Owned_Apps_Cache_*.json
~/.ipatool/
  account.json   # session
  cookies.json
  cache/         # SAP runtime cache (first login)
```

**Never commit** account / cookie files.

## Build from source

```bash
cargo build -p ipatool-kit --release
./target/release/ipatool-kit
```

## Disclaimer

Use only with accounts you own and in accordance with Apple’s terms and local law. Delisted apps may still fail to download if Apple no longer serves the package.

## Credits

Inspired by community IPA downloaders and [majd/ipatool](https://github.com/majd/ipatool).

## Support

Tips: [DonationAlerts](https://www.donationalerts.com/r/s00d88)

## License

MIT — see [LICENSE](LICENSE).
