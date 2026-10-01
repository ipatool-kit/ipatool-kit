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

## Also needed

| Tool | Role |
|------|------|
| `ipatool-cpp` (or set `IPATOOL_CPP`) | Auth + download |
| Go [`ipatool`](https://github.com/majd/ipatool) (`brew install ipatool`) | Purchase history (`list-purchases`) |

Optional faster history helper:

```text
~/.ipatool/downloader/bin/ipatool-hist
```

or set `IPATOOL_GO` to an absolute path.

## Features

- Search / enter IDs / browse lists
- Apple purchase history with cache
- Delisted IDs menu (`Apps_ID_List` − Apple history)
- Multi-select + live search; `*` = all visible
- Data under `~/.ipatool/downloader/`
- Optional install via `ideviceinstaller`

## Environment

| Variable | Meaning |
|----------|---------|
| `IPATOOL_COUNTRY` | Storefront country (default `us`) |
| `IPATOOL_CPP` | Path to cpp backend |
| `IPATOOL_GO` | Path to Go `ipatool` / `ipatool-hist` |
| `IPA_DOWNLOADER_HOME` | Override data root (default `~/.ipatool/downloader`) |

## Data layout

```text
~/.ipatool/downloader/
  Apps/    # downloaded IPAs
  Files/   # lists + Owned_Apps_Cache_*.json
  bin/     # optional ipatool-hist
```

Session cookies / tokens live under `~/.ipatool/` (backends). **Never commit those.**

## Disclaimer

Use only with accounts you own and in accordance with Apple’s terms and local law. Delisted apps may still fail to download if Apple no longer serves the package.

## Credits

Inspired by community IPA downloaders and [majd/ipatool](https://github.com/majd/ipatool).

## Support

Tips: [DonationAlerts](https://www.donationalerts.com/r/s00d88)

## License

MIT — see [LICENSE](LICENSE).
