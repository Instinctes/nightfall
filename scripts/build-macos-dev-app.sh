#!/usr/bin/env bash
# Build an isolated local-only DEV Core bundle. No production release changes.
# Installation is separate: do not replace a running wallet or delete its bundle.
#
#   ./scripts/build-macos-dev-app.sh
#
# Does not touch the production NIGHTFALLCOIN Core.app. Mainnet stays gated
# in the binary; this wrapper just starts on --network devnet.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="1.0.0-dev.1"
APP_VERSION="${VERSION%%-*}"
# Overridable so a second bundle can be installed beside the first — for
# looking at the interface while another copy is already running, which macOS
# resolves by bundle identifier and would otherwise point at the wrong window.
APP_NAME="${NIGHTFALL_DEV_APP_NAME:-NIGHTFALL 1.0 Dev}"
BUNDLE_ID="${NIGHTFALL_DEV_BUNDLE_ID:-cash.nightfall.wallet.v1dev}"
ARCH="${1:-arm64}"
case "$ARCH" in
    arm64) TRIPLE=aarch64-apple-darwin; MIN_MACOS=11.0 ;;
    intel) TRIPLE=x86_64-apple-darwin; MIN_MACOS=10.15 ;;
    *) echo "Usage: $0 [arm64|intel]" >&2; exit 2 ;;
esac
ICON="$ROOT/assets/AppIcon.icns"
DEV_TARGET="$ROOT/target/dev-build"
mkdir -p "$ROOT/target"
WORK="$(mktemp -d "$ROOT/target/macos-dev-XXXXXX")"

if [[ ! -f "$ICON" ]]; then
    echo "!! Missing $ICON" >&2
    exit 1
fi


# Mainnet is off unless asked for at build time. Set NIGHTFALL_DEV_MAINNET=1 to
# produce a build that opens the real wallet directory — see the warning the
# binary prints and the banner it shows. Do not hand such a build to anyone.
if [[ -n "${NIGHTFALL_DEV_MAINNET:-}" ]]; then
    echo "!! MAINNET development build — opens the real wallet directory." >&2
    echo "!! Once it saves, released 0.9.5 can no longer read that wallet." >&2
    NETWORK_ARG="mainnet"
else
    NETWORK_ARG="devnet"
fi

echo "==> ${APP_NAME} ${VERSION} (${NETWORK_ARG}, $ARCH)"

cd "$ROOT"
# `export`, not an inline assignment: `${VAR:+NAME=$VAR}` expands to a word
# bash then tries to execute, which is how the first mainnet build silently
# produced a devnet binary with a mainnet launcher in front of it.
[[ -n "${NIGHTFALL_DEV_MAINNET:-}" ]] && export NIGHTFALL_DEV_MAINNET
NIGHTFALL_DEV_VERSION="$VERSION" MACOSX_DEPLOYMENT_TARGET="$MIN_MACOS" \
    cargo +1.98.0 build --offline --locked --release --jobs 2 --target "$TRIPLE" \
    --target-dir "$DEV_TARGET" -p nightfall-core

BIN="$DEV_TARGET/$TRIPLE/release"
APP="$WORK/${APP_NAME}.app"

mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$BIN/nightfall-core" "$APP/Contents/MacOS/nightfall-core.bin"
chmod +x "$APP/Contents/MacOS/"*

# Finder starts an app with no arguments, so the network is pinned here. An
# ordinary build is pinned to devnet AND refused mainnet by the binary itself,
# and uses a separate wallet-1.0-dev subdirectory; a build made with
# NIGHTFALL_DEV_MAINNET is pinned to mainnet and uses the real one. No CLI
# binaries with production defaults are bundled either way.
cat > "$APP/Contents/MacOS/nightfall-core" <<LAUNCH
#!/bin/bash
DIR="\$(cd "\$(dirname "\$0")" && pwd)"
exec "\$DIR/nightfall-core.bin" --network $NETWORK_ARG "\$@"
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
cp "$ROOT/docs/DEV-WALLET-1.0.0.md" "$WORK/READ-ME-FIRST.md"
cp "$ROOT/docs/DEV-WALLET-1.0.0.md" "$APP/Contents/Resources/READ-ME-FIRST.md"
# Resources are part of the signature; sign again after including the guide.
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"
ARCHIVE="$WORK/NIGHTFALL-Dev-$VERSION-macOS-$ARCH.zip"
ditto -c -k --sequesterRsrc --keepParent "$APP" "$ARCHIVE"
(cd "$WORK" && shasum -a 256 "NIGHTFALL-Dev-$VERSION-macOS-$ARCH.zip" > SHA256SUMS.txt)
echo "==> Dev artifact: $APP"
echo "==> Dev archive: $ARCHIVE"
echo "    No installed app or wallet data was changed."
