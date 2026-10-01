# ipatool-kit

Interactive terminal toolkit for browsing your App Store library and downloading IPA packages you already own — including apps removed from the store when your account still has a license.

Built in Rust. Menu-driven (arrow keys, multi-select, live search). Uses:

- **ipatool-cpp** (or a compatible local binary) for login / purchase / download  
- **[majd/ipatool](https://github.com/majd/ipatool)** (Go) for Apple purchase-history listing (`list-purchases`)  
- Optional curated **Apps_ID_List** for delisted app IDs that Apple’s history API omits  

> This project does **not** crack DRM or bypass Apple licensing. You can only download apps your Apple ID is entitled to.

## Features

- Search App Store / enter IDs / browse lists  
- Apple purchase history (all platforms) with cache  
- Separate menu for **delisted** IDs (Apps_ID_List − Apple history)  
- Multi-select with live search; `*` toggles all visible rows  
- Isolated data dir: `~/.ipatool/downloader/`  
- Optional install via `ideviceinstaller` when present  

## Requirements

| Tool | Role |
|------|------|
| Rust 1.75+ | Build this repo |
| `ipatool-cpp` (or equivalent) | Auth + download |
| Go [`ipatool`](https://github.com/majd/ipatool) (`brew install ipatool`) | Purchase history |
| Optional: patched `ipatool-hist` with higher page limit | Faster full-history fetch (~30–60s one shot) |

Place a high-limit helper at:

```text
~/.ipatool/downloader/bin/ipatool-hist
```

or set `IPATOOL_GO` / `IPATOOL_CPP` to absolute paths.

## Quick start

```bash
git clone https://github.com/ipatool-kit/ipatool-kit.git
cd ipatool-kit
cargo run -p ipa-downloader --release
```

Binary name: **`ipatool-kit`** (avoids clashing with Homebrew `ipatool`).

```bash
cargo build -p ipa-downloader --release
./target/release/ipatool-kit
```

Environment:

| Variable | Meaning |
|----------|---------|
| `IPATOOL_COUNTRY` | Storefront country (default `us`) |
| `IPATOOL_CPP` | Path to cpp backend |
| `IPATOOL_GO` | Path to Go `ipatool` / `ipatool-hist` |
| `IPA_DOWNLOADER_HOME` | Override data root (default `~/.ipatool/downloader`) |

## Library crate

`crates/ipatool` — helpers + iTunes search client (no SAP signing).

```bash
cargo test -p ipatool
cargo clippy -p ipa-downloader -- -D warnings
```

## Data layout

```text
~/.ipatool/downloader/
  Apps/           # downloaded IPAs
  Files/          # lists + Owned_Apps_Cache_*.json
  bin/            # optional ipatool-hist
```

Session cookies / account tokens live under `~/.ipatool/` (managed by backends). **Never commit those files.**

## Releases

Tagged versions (`v*`) publish platform binaries via GitHub Releases.

## Disclaimer

Use only with accounts you own and in accordance with Apple’s terms and local law. Delisted apps may still fail to download if Apple no longer serves the package.

## Credits

Inspired by community IPA downloaders and [majd/ipatool](https://github.com/majd/ipatool).

## Support

Tips: [DonationAlerts](https://www.donationalerts.com/r/s00d88)

## License

MIT — see [LICENSE](LICENSE).
