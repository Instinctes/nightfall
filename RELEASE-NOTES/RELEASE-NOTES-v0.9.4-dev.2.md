# NIGHTFALL Dev 0.9.4-dev.2

Development preview, not a stable or independently audited release.
Stable desktop remains **0.9.2**. Mainnet atomic swaps remain disabled.

## One wallet design, two layouts

Core desktop and the responsive web wallet share soft violet surfaces, rounded
cards, restrained shadows, pastel pink/lavender/cyan accents and readable dark
ink on bright balances and actions. Core has drawn navigation icons; the web
wallet uses a desktop sidebar and thumb-accessible mobile bottom navigation.
Forms, activity, contacts, receive addresses and settings share the same styling.
Core QR codes have a square white background and four-module quiet zone.
The web payment review shows the full address; incomplete or unaffordable
payments cannot open review. Node failures have readable connection messages.
HTML escaping now covers quotes in attribute values as well as text.

The web wallet remains a mainnet light wallet with its existing trust model.
It receives the responsive design, **not atomic swaps**. No third-party fonts,
analytics, image services or accounts were added.

## Core swaps and limitations

Includes the execution/recovery fixes described in
[0.9.4-dev.1](RELEASE-NOTES-v0.9.4-dev.1.md): serial background monitoring,
durable exact-byte retries, confirmed-chain observation, correct witness-secret
recovery, exposure-aware cancellation and Alice NIGHT recovery after Bob refund.
All prerelease Core binaries default to devnet; the macOS Dev wrapper pins devnet.
CLI node/wallet tools require an explicit `--network devnet` argument.

**Use test coins only.** There is no independent timed NIGHT refund. If Bob
never publishes his Bitcoin refund, Alice's NIGHT can remain locked permanently,
even after punishment. Keep both nodes and Core online until settlement or
recovery finishes. Back up per-swap `.secret` files as well as the wallet seed.
The seed alone cannot recover swap secrets. Independent cryptographic review
and long-duration public-network acceptance remain outstanding.

## Downloads and verification

- macOS Apple Silicon: `NIGHTFALL-Dev-0.9.4-dev.2-macOS-arm64.zip`
- macOS Intel: `NIGHTFALL-Dev-0.9.4-dev.2-macOS-intel.zip`
- Windows x64: `nightfall-core-0.9.4-dev.2-windows-x64.exe`
- Linux x64: `nightfall-core-0.9.4-dev.2-linux-x64`
- `nightfalld` and `nightfall-wallet` CLI tools accompany Windows/Linux builds
  and are included inside macOS app bundles.

Use `SHA256SUMS-0.9.4-dev.2.txt` for macOS, or the corresponding `-windows.txt`
and `-linux.txt` list. Verify downloaded bytes before running them. macOS bundles
are ad-hoc signed, not Developer-ID signed or Apple-notarized. Windows builds
have no publisher signing certificate. Keep the Dev app separate from stable.

## Validation

The release workflow runs workspace tests on Windows/Linux before building and
attaching binaries. Local gates include strict workspace Clippy, workspace tests,
Core eight-page layout tests, normal-text and gradient contrast regressions,
disabled-button tests and stable/development download-version checks.
Browser review uses a known public test phrase on localhost, no real funds,
at mobile and desktop widths. A static local server deliberately has no live
wallet API; offline states are therefore part of that review.

The preceding candidate also passed the isolated real-bitcoind/NIGHT Core-worker
settlement and cancel/refund/recovery test, reloading sessions between ticks, and
a 5,000-tick simulation. These are bounded tests, not a guarantee of safety or
full visual acceptance on every platform/device.
