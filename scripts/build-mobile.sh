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
echo "    iOS:     mobile/ios/NightfallWallet.xcodeproj  (sideload in the EU)"
echo "    Node:    nightfalld --network mainnet run --mobile-listen 0.0.0.0:17888"
