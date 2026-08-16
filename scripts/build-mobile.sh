#!/usr/bin/env bash
# Build the phone libraries and, when the SDKs exist, the apps.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> FFI bindings"
cargo build -p nightfall-mobile
LIB_HOST="$ROOT/target/debug/libnightfall_mobile.dylib"
cargo run -q -p nightfall-mobile --bin uniffi-bindgen -- generate \
  --library "$LIB_HOST" --language kotlin --out-dir mobile/android/app/src/main/java --no-format
cargo run -q -p nightfall-mobile --bin uniffi-bindgen -- generate \
  --library "$LIB_HOST" --language swift --out-dir mobile/ios/NightfallWallet/Generated --no-format

if command -v rustup >/dev/null; then
  rustup target add aarch64-apple-ios aarch64-apple-ios-sim >/dev/null 2>&1 || true
fi

echo "==> iOS static library (device)"
if rustup target list --installed | grep -q aarch64-apple-ios; then
  cargo build --release -p nightfall-mobile --target aarch64-apple-ios
  mkdir -p mobile/ios/Libs
  cp -f target/aarch64-apple-ios/release/libnightfall_mobile.a mobile/ios/Libs/
  echo "    libnightfall_mobile.a ready"
fi

if command -v xcodegen >/dev/null; then
  (cd mobile/ios && xcodegen generate)
fi

# Full Xcode (not just CLT). DEVELOPER_DIR avoids needing sudo xcode-select.
if [[ -d /Applications/Xcode.app/Contents/Developer ]]; then
  export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
fi
if xcrun --sdk iphoneos --show-sdk-path >/dev/null 2>&1 && command -v xcodegen >/dev/null; then
  echo "==> iOS IPA (unsigned, arm64)"
  (cd mobile/ios && xcodegen generate && xcodebuild \
    -project NightfallWallet.xcodeproj \
    -target NightfallWallet \
    -configuration Release \
    -sdk iphoneos \
    CODE_SIGNING_ALLOWED=NO \
    CODE_SIGNING_REQUIRED=NO \
    CODE_SIGN_IDENTITY=- \
    DEVELOPMENT_TEAM= \
    build)
  APP="$ROOT/mobile/ios/build/Release-iphoneos/NIGHT.app"
  if [[ -d "$APP" ]]; then
    STRIP="$(xcrun --find strip)"
    "$STRIP" -STx "$APP/NIGHT" || true
    STAGE="$(mktemp -d)"
    mkdir -p "$STAGE/Payload"
    cp -R "$APP" "$STAGE/Payload/NIGHT.app"
    mkdir -p "$ROOT/wallets"
    OUT="$ROOT/wallets/NIGHTFALLCOIN-0.7.0-ios-arm64.ipa"
    rm -f "$OUT"
    (cd "$STAGE" && zip -qr "$OUT" Payload)
    rm -rf "$STAGE"
    shasum -a 256 "$OUT" | awk '{print $1"  NIGHTFALLCOIN-0.7.0-ios-arm64.ipa"}' \
      | tee "$ROOT/wallets/SHA256SUMS-0.7.0-ios.txt"
    echo "    $OUT"
  fi
fi

if [[ -n "${ANDROID_NDK_HOME:-${ANDROID_NDK:-}}" ]] || [[ -d "${ANDROID_HOME:-$HOME/Library/Android/sdk}/ndk" ]]; then
  echo "==> Android arm64"
  rustup target add aarch64-linux-android >/dev/null 2>&1 || true
  if command -v cargo-ndk >/dev/null; then
    cargo ndk -t arm64-v8a -o mobile/android/app/src/main/jniLibs build --release -p nightfall-mobile
  else
    echo "    install cargo-ndk to produce libnightfall_mobile.so"
  fi
else
  echo "==> Android NDK not found — Kotlin sources are ready; install the NDK and rerun."
fi

echo "==> Done. Open:"
echo "    Android: mobile/android  (Android Studio)"
echo "    iOS:     wallets/NIGHTFALLCOIN-0.7.0-ios-arm64.ipa  (unsigned sideload)"
echo "             or mobile/ios/NightfallWallet.xcodeproj"
echo "    Node:    nightfalld --network mainnet run --mobile-listen 0.0.0.0:17888"
