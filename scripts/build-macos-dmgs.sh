#!/usr/bin/env bash
# Build NIGHTFALLCOIN Core .app bundles and .dmg installers for macOS.
#
#   ./scripts/build-macos-dmgs.sh
#
# Produces, under wallets/:
#   NIGHTFALLCOIN-Core-<version>-macOS-arm64.dmg   Apple Silicon
#   NIGHTFALLCOIN-Core-<version>-macOS-intel.dmg   Intel
#
# The version is in the filename on purpose. Without it a new release
# overwrites the previous file at the same URL, and every cache between here
# and the user — CDN edge, browser, corporate proxy — may keep handing out the
# old bytes under the new build's published checksum. A user who verifies the
# checksum then sees a mismatch and cannot tell whether they are looking at a
# stale cache or a tampered download. Distinct names make that impossible:
# a URL always refers to exactly one file.
#
# Both target macOS 12.5 or newer. Neither is code-signed — see wallets/README.md.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/wallets"
WORK="$ROOT/target/macos-bundle"
export MACOSX_DEPLOYMENT_TARGET=12.5

VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | sed 's/.*"\(.*\)".*/\1/')"
BUNDLE_ID="cash.nightfall.core"
APP_NAME="NIGHTFALLCOIN Core"

echo "==> NIGHTFALLCOIN Core ${VERSION} — macOS bundles"

# ---------------------------------------------------------------- icon ------
ICON="$ROOT/assets/AppIcon.icns"
if [[ ! -f "$ICON" ]]; then
    echo "!! Missing $ICON"
    echo "   Generate it from a 1024x1024 PNG with:"
    echo "   ./scripts/make-icon.sh path/to/icon.png"
    exit 1
fi

# ------------------------------------------------------------- binaries -----
build_target() {
    local TRIPLE="$1"
    echo "==> Building $TRIPLE"
    (cd "$ROOT" && cargo build --release --target "$TRIPLE" \
        -p nightfall-core -p nightfall-node -p nightfall-wallet)
}

# ------------------------------------------------------------- bundle -------
make_app() {
    local LABEL="$1" TRIPLE="$2"
    local APP="$WORK/$LABEL/$APP_NAME.app"
    local BIN="$ROOT/target/$TRIPLE/release"

    rm -rf "$WORK/$LABEL"
    mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

    # The GUI is the bundle's executable; the CLI tools ride along so a user who
    # wants them does not need a second download.
    cp "$BIN/nightfall-core" "$APP/Contents/MacOS/nightfall-core.bin"
    cp "$BIN/nightfalld" "$APP/Contents/MacOS/nightfalld"
    cp "$BIN/nightfall-wallet" "$APP/Contents/MacOS/nightfall-wallet"
    chmod +x "$APP/Contents/MacOS/"*

    # Launcher: Finder starts an app with no arguments, so default to mainnet
    # while still allowing `--network devnet` from a terminal.
    cat > "$APP/Contents/MacOS/nightfall-core" <<'LAUNCH'
#!/bin/bash
DIR="$(cd "$(dirname "$0")" && pwd)"
if [[ $# -eq 0 ]]; then
    exec "$DIR/nightfall-core.bin" --network "${NF_NETWORK:-mainnet}"
else
    exec "$DIR/nightfall-core.bin" "$@"
fi
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
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>LSMinimumSystemVersion</key><string>12.5</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSHumanReadableCopyright</key><string>NIGHTFALLCOIN — fair launch, no premine</string>
</dict>
</plist>
PLIST

    printf 'APPL????' > "$APP/Contents/PkgInfo"
    echo "    bundled $LABEL"
}

# ---------------------------------------------------------------- dmg -------
make_dmg() {
    local LABEL="$1" OUTNAME="$2"
    local STAGE="$WORK/$LABEL/stage"
    local DMG="$OUT/$OUTNAME"

    rm -rf "$STAGE"
    mkdir -p "$STAGE"
    cp -R "$WORK/$LABEL/$APP_NAME.app" "$STAGE/"
    ln -s /Applications "$STAGE/Applications"

    cat > "$STAGE/READ ME FIRST.txt" <<'TXT'
NIGHTFALLCOIN Core
==================

1. Drag "NIGHTFALLCOIN Core" onto the Applications folder.
2. First launch: right-click the app -> Open -> Open.
   macOS blocks unsigned apps on a normal double-click. This app is not
   code-signed, so that warning is expected.
3. Go to Settings -> Backup and write your recovery seed on paper.
   Lose it and the coins are gone. There is no reset and no support desk.

Mining runs on every CPU core but one. Mined coins are shown as "immature"
for 1,440 blocks (~6 hours) before they can be spent.

This is pre-launch software that has not been audited by anyone outside the
project. Do not put value on it you cannot afford to lose.
TXT

    rm -f "$DMG"
    hdiutil create -volname "$APP_NAME" -srcfolder "$STAGE" \
        -ov -format UDZO "$DMG" >/dev/null
    echo "    $(basename "$DMG")  $(du -h "$DMG" | cut -f1)"
}

mkdir -p "$OUT"

build_target aarch64-apple-darwin
build_target x86_64-apple-darwin

make_app arm64 aarch64-apple-darwin
make_app intel x86_64-apple-darwin

echo "==> Packaging"
make_dmg arm64 "NIGHTFALLCOIN-Core-${VERSION}-macOS-arm64.dmg"
make_dmg intel "NIGHTFALLCOIN-Core-${VERSION}-macOS-intel.dmg"

# Checksums, next to the files, so publishing a release does not depend on
# anyone remembering to run shasum by hand.
(cd "$OUT" && shasum -a 256 NIGHTFALLCOIN-Core-${VERSION}-macOS-*.dmg > "SHA256SUMS-${VERSION}.txt")
echo "    SHA256SUMS-${VERSION}.txt"

echo "==> Done. Output in $OUT"
