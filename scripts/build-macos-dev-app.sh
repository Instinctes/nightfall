#!/usr/bin/env bash
# Build an isolated DEV Core bundle and a versioned zip (devnet, swaps enabled).
# Installation is separate: do not replace a running wallet or delete its bundle.
#
#   ./scripts/build-macos-dev-app.sh
#
# Does not touch the production NIGHTFALLCOIN Core.app. Mainnet stays gated
# in the binary; this wrapper just starts on --network devnet.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | sed 's/.*"\(.*\)".*/\1/')"
APP_VERSION="${VERSION%%-*}"
APP_NAME="NIGHTFALL Dev"
BUNDLE_ID="cash.nightfall.dev"
ARCH="${1:-arm64}"
case "$ARCH" in
    arm64) TRIPLE=aarch64-apple-darwin; MIN_MACOS=11.0 ;;
    intel) TRIPLE=x86_64-apple-darwin; MIN_MACOS=10.15 ;;
    *) echo "Usage: $0 [arm64|intel]" >&2; exit 2 ;;
esac
ICON="$ROOT/assets/AppIcon.icns"
DEST="/Applications/${APP_NAME}.app"
mkdir -p "$ROOT/target"
WORK="$(mktemp -d "$ROOT/target/macos-dev-XXXXXX")"

if [[ ! -f "$ICON" ]]; then
    echo "!! Missing $ICON" >&2
    exit 1
fi

echo "==> ${APP_NAME} ${VERSION} (devnet, $ARCH)"

MACOSX_DEPLOYMENT_TARGET="$MIN_MACOS" cargo build --offline --locked --release --target "$TRIPLE" \
    -p nightfall-core -p nightfall-node -p nightfall-wallet

BIN="$ROOT/target/$TRIPLE/release"
APP="$WORK/${APP_NAME}.app"

mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$BIN/nightfall-core" "$APP/Contents/MacOS/nightfall-core.bin"
cp "$BIN/nightfalld" "$APP/Contents/MacOS/nightfalld"
cp "$BIN/nightfall-wallet" "$APP/Contents/MacOS/nightfall-wallet"
chmod +x "$APP/Contents/MacOS/"*

# Finder starts an app with no arguments. Pin this bundle to devnet so a
# double-click cannot open mainnet by accident. The first --network wins.
cat > "$APP/Contents/MacOS/nightfall-core" <<'LAUNCH'
#!/bin/bash
DIR="$(cd "$(dirname "$0")" && pwd)"
exec "$DIR/nightfall-core.bin" --network devnet "$@"
LAUNCH
chmod +x "$APP/Contents/MacOS/nightfall-core"

cp "$ICON" "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$APP_NAME</string>
    <key>CFBundleDisplayName</key><string>$APP_NAME</string>
    <key>CFBundleExecutable</key><string>nightfall-core</string>
    <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
    <key>CFBundleVersion</key><string>$APP_VERSION</string>
    <key>CFBundleShortVersionString</key><string>$APP_VERSION</string>
    <key>NIGHTFALLDevelopmentVersion</key><string>$VERSION</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>LSMinimumSystemVersion</key><string>$MIN_MACOS</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSHumanReadableCopyright</key><string>NIGHTFALLCOIN — fair launch, no premine</string>
</dict>
</plist>
PLIST

printf 'APPL????' > "$APP/Contents/PkgInfo"

# Ad-hoc signing verifies bundle integrity locally; this is not notarization.
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"
plutil -lint "$APP/Contents/Info.plist"
ARCHIVE="$WORK/NIGHTFALL-Dev-$VERSION-macOS-$ARCH.zip"
ditto -c -k --sequesterRsrc --keepParent "$APP" "$ARCHIVE"
(cd "$WORK" && shasum -a 256 "NIGHTFALL-Dev-$VERSION-macOS-$ARCH.zip" > SHA256SUMS.txt)
echo "==> Dev artifact: $APP"
echo "==> Dev archive: $ARCHIVE"
echo "    No installed app or wallet data was changed."
