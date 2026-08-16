# Phone wallets — 0.7.0

## Android (built)

`NIGHTFALLCOIN-0.7.0-android-arm64.apk` — 64-bit Android, sideload.

On the phone: allow installs from this source, tap the APK.
Or from a computer: `adb install NIGHTFALLCOIN-0.7.0-android-arm64.apk`

Checksum: `SHA256SUMS-0.7.0-android.txt`

The app talks to `http://seed.nightfallcoin.org:17888` (light API only).

iPhone: `NIGHTFALLCOIN-0.7.0-ios-arm64.ipa` (unsigned). Sideload with
AltStore, Sideloadly or Xcode. Or https://nightfallcoin.org/wallet/ —
add to the Home Screen. Same 24 words as Core.

## iOS (not in this folder)

This Mac has only Apple Command Line Tools — **no Xcode, no iPhone SDK**.
A real `.ipa` cannot be produced here. The SwiftUI project is
`mobile/ios/` in the repo.

On a Mac **with full Xcode**:

```
brew install xcodegen
./scripts/build-mobile.sh
cd mobile/ios && xcodegen generate
open NightfallWallet.xcodeproj
```

Sign with your Apple ID, plug in the iPhone, Run. In the EU you can
sideload that build (AltStore / alternative marketplace). No App Store
listing is required.
