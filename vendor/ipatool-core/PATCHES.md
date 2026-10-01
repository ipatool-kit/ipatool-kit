# Vendored `ipatool-core` 0.1.8 (patched)

Source: crates.io `ipatool-core` 0.1.8 (Kosthi/ipatool-rs). Kept in-tree so we can match
[majd/ipatool](https://github.com/majd/ipatool) wire behaviour without waiting on upstream.

## Patches vs crates.io

| Gap | Fix |
|---|---|
| Bag fetch | Match Go: `GET …/bag.xml?guid=` + Configurator UA → parse `urlBag` |
| Plist/XML responses | Match Go: Document/plist/dict normalize + reject HTML before Serde |
| Auth HTTP client | Isolated client: `pool_max_idle_per_host(0)` + HTTP/1.1 (Go `DisableKeepAlives`) |
| Auth redirects | Follow only HTTP 302; retry HTML 301 without Location on fresh TCP |
| Missing bag download endpoints | Parse `redownloadProduct` / `updateProduct` |
| No `serialNumber` on product POSTs | Always send `serialNumber=0` |
| No download fallback | volumeStore → redownload → updateProduct |
| Purchase payload | Drop non-Go `hasBeenAuthedForBuy`; match majd buyProduct fields |
| Download+purchase | Go order: download first; buyProduct only on license-not-found |
| failureType 2040 | Treat as temporarily unavailable (retry GAME pricing) |
| Auth HTTP flakes | Retry 204 / 404 / 429 / 5xx with backoff + `Retry-After` |
| Login `Content-Type` | `application/x-www-form-urlencoded` (Go parity; body still XML plist) |

## Not ported

macOS `.pkg` decrypt / XAR sidecars — this toolkit is IPA-focused; Mac packages error out clearly.
