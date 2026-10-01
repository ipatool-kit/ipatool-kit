# ipatool-kit

Interactive terminal toolkit for browsing your App Store library and downloading IPA packages you already own — including apps removed from the store when your account still has a license.

> Does **not** crack DRM or bypass Apple licensing. Only apps your Apple ID is entitled to.

## Install

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

## Disclaimer

Use only with accounts you own and in accordance with Apple’s terms and local law. Delisted apps may still fail to download if Apple no longer serves the package.

## Credits

Inspired by community IPA downloaders and [majd/ipatool](https://github.com/majd/ipatool). Auth/SAP via [ipatool-core](https://crates.io/crates/ipatool-core).

## Support

Tips: [DonationAlerts](https://www.donationalerts.com/r/s00d88)

## License

MIT — see [LICENSE](LICENSE).
