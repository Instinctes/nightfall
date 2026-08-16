# Nightfall phone wallets

Full wallets (seed on device, receive, send). They are **not** in the App
Store or Play Store. In the EU, iOS 17.4+ can install this build outside
Apple’s store. Android is sideloaded as an APK.

The phone **trusts a node** for what it displays. It cannot steal coins.
Default node: `http://seed.nightfallcoin.org:17888` (light API only —
no mining RPC).

## What is here

```
mobile/
  android/     Kotlin + Compose, Android Studio
  ios/         SwiftUI, Xcode, sideload (AltStore / EU web distro)
```

Shared logic is Rust (`crates/nightfall-mobile`) via UniFFI. Do not
reimplement key derivation in Swift or Kotlin.

## Build

```bash
./scripts/build-mobile.sh
```

**Android** — Android Studio → open `mobile/android`. Needs NDK. Then
Build → Build APK. Install with `adb install` or by tapping the APK.

**iOS** — Mac with the iOS SDK (full Xcode, not just CLT). Then:

```bash
brew install xcodegen
./scripts/build-mobile.sh
cd mobile/ios && xcodegen generate
open NightfallWallet.xcodeproj
```

Sign with your free Apple ID, plug in a phone, Run. In the EU you can
also export an unsigned/ad-hoc IPA and install via AltStore or an
alternative marketplace. No App Store listing is required.

## Seed node

```bash
nightfalld --network mainnet run --mobile-listen 0.0.0.0:17888 --proxy off
```

Open TCP **17888**. That port answers only `status`, `scan_feed`,
`submit_tx`, `get_utxo_root`, `banner`. `mine_one` is not on it.

Put Caddy or nginx in front for HTTPS when you have a cert. The apps
allow cleartext only to `*.nightfallcoin.org`.
