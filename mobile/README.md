# Nightfall phone wallets

Full wallets (seed on device, receive, send). They are **not** in the App
Store or Play Store. In the EU, iOS 17.4+ can install this build outside
Apple’s store. Android is sideloaded as an APK.

The phone **trusts a node** for what it displays. It cannot steal coins.
Default node: `http://seed.nightfallcoin.org:17888` (light API only —
no mining RPC).

iPhone: download the IPA from https://nightfallcoin.org and sideload it
(AltStore / Sideloadly / Xcode + your Apple ID). Or open
https://nightfallcoin.org/wallet/ and add it to the Home Screen. Same
24 words as Core. The phone trusts the seed node for what it shows.

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

**iOS** — Mac with full Xcode (not just the Command Line Tools):

```bash
sudo xcode-select -s /Applications/Xcode.app/Contents/Developer
brew install xcodegen
./scripts/build-mobile.sh
```

That writes `wallets/NIGHTFALLCOIN-0.7.0-ios-arm64.ipa` (unsigned).
Sideload with AltStore, Sideloadly, or open
`mobile/ios/NightfallWallet.xcodeproj`, sign with your Apple ID, plug
in a phone, Run. No App Store listing.

## Seed node

```bash
nightfalld --network mainnet run --mobile-listen 0.0.0.0:17888 --proxy off
```

Open TCP **17888**. That port answers only `status`, `scan_feed`,
`submit_tx`, `get_utxo_root`, `banner`. `mine_one` is not on it.

Put Caddy or nginx in front for HTTPS when you have a cert. The apps
allow cleartext only to `*.nightfallcoin.org`.
