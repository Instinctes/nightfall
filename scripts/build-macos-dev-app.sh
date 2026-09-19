#!/usr/bin/env bash
# Build an isolated local-only DEV Core bundle. No production release changes.
# Installation is separate: do not replace a running wallet or delete its bundle.
#
#   ./scripts/build-macos-dev-app.sh arm64
#   ./scripts/build-macos-dev-app.sh --print-config
#   NIGHTFALL_DEV_MAINNET=1 ./scripts/build-macos-dev-app.sh --legacy-1.0
#
# The default 1.0.0 preview cannot inherit the old operator-only mainnet opt-in.
# The former 1.0 development profile is available only with --legacy-1.0.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ARCH=arm64
ARCH_SET=false
LEGACY=false
PRINT_CONFIG=false
for arg in "$@"; do
    case "$arg" in
        arm64|intel)
            if $ARCH_SET; then echo "!! Specify one architecture only." >&2; exit 2; fi
            ARCH="$arg"; ARCH_SET=true ;;
        --legacy-1.0)
            if $LEGACY; then echo "!! Duplicate legacy profile." >&2; exit 2; fi
            LEGACY=true ;;
        --print-config) PRINT_CONFIG=true ;;
        *) echo "Usage: $0 [arm64|intel] [--legacy-1.0] [--print-config]" >&2; exit 2 ;;
    esac
done
case "$ARCH" in
    arm64) TRIPLE=aarch64-apple-darwin; MIN_MACOS=11.0 ;;
    intel) TRIPLE=x86_64-apple-darwin; MIN_MACOS=10.15 ;;
esac

if $LEGACY; then
    VERSION="1.0.0-dev.1"
    APP_NAME="${NIGHTFALL_DEV_APP_NAME:-NIGHTFALL 1.0 Dev}"
    BUNDLE_ID="${NIGHTFALL_DEV_BUNDLE_ID:-cash.nightfall.wallet.v1dev}"
    PROFILE=legacy-1.0
    DATA_NAMESPACE=wallet-1.0-dev
    GUIDE="$ROOT/docs/DEV-WALLET-1.0.0.md"
    DEV_TARGET="$ROOT/target/dev-build"
    NETWORK_ARG=devnet
    if [[ -n "${NIGHTFALL_DEV_MAINNET:-}" ]]; then
        echo "!! MAINNET development build — opens the real wallet directory." >&2
        echo "!! Once it saves, released 0.9.5 can no longer read that wallet." >&2
        NETWORK_ARG=mainnet
        DATA_NAMESPACE="real-mainnet-directory"
    fi
else
    VERSION="1.0.0-dev.2"
    APP_NAME="NIGHTFALL Dev 1.0.0"
    BUNDLE_ID="cash.nightfall.wallet.dev100preview"
    PROFILE=isolated-1.0.0-dev.2
    DATA_NAMESPACE=wallet-1.0.0-dev.2
    GUIDE="$ROOT/docs/DEV-WALLET-1.0.0-dev.2.md"
    DEV_TARGET="$ROOT/target/dev-build"
    NETWORK_ARG=devnet
    if [[ -n "${NIGHTFALL_DEV_MAINNET:-}" ]]; then
        echo "==> Ignoring inherited NIGHTFALL_DEV_MAINNET: 1.0.0-dev.2 is devnet-only." >&2
    fi
fi
APP_VERSION="${VERSION%%-*}"

# Names end up in paths, a shell launcher and XML; custom legacy names must
# remain plain labels. The isolated profile deliberately has fixed identity.
if [[ ! "$APP_NAME" =~ ^[A-Za-z0-9][A-Za-z0-9\ ._-]*$ ]] ||
   [[ ! "$BUNDLE_ID" =~ ^[A-Za-z0-9]+([.-][A-Za-z0-9]+)+$ ]]; then
    echo "!! Invalid application name or bundle identifier." >&2
    exit 2
fi

if $PRINT_CONFIG; then
    printf 'version=%s\napp_version=%s\napp_name=%s\nbundle_id=%s\nprofile=%s\nnetwork=%s\ndata_namespace=%s\ntarget=%s\nguide=%s\n' \
        "$VERSION" "$APP_VERSION" "$APP_NAME" "$BUNDLE_ID" "$PROFILE" \
        "$NETWORK_ARG" "$DATA_NAMESPACE" "$TRIPLE" "$GUIDE"
    exit 0
fi

ICON="$ROOT/assets/AppIcon.icns"
for source in "$ICON" "$GUIDE"; do
    if [[ ! -f "$source" ]]; then echo "!! Missing $source" >&2; exit 1; fi
done
# Never follow an output/cache root onto an installed bundle or public assets.
for path in "$ROOT/target" "$DEV_TARGET" "$ROOT/dev-builds"; do
    if [[ -L "$path" ]] || [[ -e "$path" && ! -d "$path" ]]; then
        echo "!! Refusing non-directory or symlink output: $path" >&2
        exit 1
    fi
done
mkdir -p "$ROOT/target" "$DEV_TARGET" "$ROOT/dev-builds"
# Every invocation receives a fresh directory. No former app, archive or
# checksum is replaced, including when two packaging runs overlap.
WORK="$(mktemp -d "$ROOT/dev-builds/macos-dev-${APP_VERSION}-${ARCH}-XXXXXX")"
echo "==> ${APP_NAME} ${VERSION} (${NETWORK_ARG}, $ARCH)"
echo "==> New output directory: $WORK"

cd "$ROOT"
if $LEGACY; then
    # An inherited isolated profile must not make the explicit legacy build
    # compile a different gate from the network its launcher is pinned to.
    env -u NIGHTFALL_DEV_PROFILE NIGHTFALL_DEV_VERSION="$VERSION" \
        MACOSX_DEPLOYMENT_TARGET="$MIN_MACOS" \
        cargo +1.98.0 build --offline --locked --release --jobs 2 --target "$TRIPLE" \
        --target-dir "$DEV_TARGET" -p nightfall-core
else
    # Unset, rather than set to an empty string: option_env! sees empty as Some.
    # The binary also has an independent isolated-profile mainnet veto.
    env -u NIGHTFALL_DEV_MAINNET NIGHTFALL_DEV_PROFILE="$PROFILE" \
        NIGHTFALL_DEV_VERSION="$VERSION" MACOSX_DEPLOYMENT_TARGET="$MIN_MACOS" \
        cargo +1.98.0 build --offline --locked --release --jobs 2 --target "$TRIPLE" \
        --target-dir "$DEV_TARGET" -p nightfall-core
fi

BIN="$DEV_TARGET/$TRIPLE/release"
APP="$WORK/${APP_NAME}.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN/nightfall-core" "$APP/Contents/MacOS/nightfall-core.bin"
chmod +x "$APP/Contents/MacOS/nightfall-core.bin"
EXPECTED_ARCH=arm64
[[ "$ARCH" == intel ]] && EXPECTED_ARCH=x86_64
if [[ "$(lipo -archs "$APP/Contents/MacOS/nightfall-core.bin")" != "$EXPECTED_ARCH" ]]; then
    echo "!! Built binary architecture does not match $ARCH." >&2
    exit 1
fi

# Finder starts with no arguments (or an old macOS process serial number).
# The dev.2 launcher accepts no data/network overrides; testing a second fresh
# directory is an explicit advanced invocation of the inner binary instead.
if $LEGACY; then
cat > "$APP/Contents/MacOS/nightfall-core" <<LAUNCH
#!/bin/bash
DIR="\$(cd "\$(dirname "\$0")" && pwd)"
exec "\$DIR/nightfall-core.bin" --network $NETWORK_ARG "\$@"
LAUNCH
else
cat > "$APP/Contents/MacOS/nightfall-core" <<'LAUNCH'
#!/bin/bash
set -euo pipefail
for arg in "$@"; do
    case "$arg" in
        -psn_*) ;;
        *) echo "NIGHTFALL Dev 1.0.0 starts only in its isolated devnet directory; launcher overrides are disabled." >&2; exit 2 ;;
    esac
done
DIR="$(cd "$(dirname "$0")" && pwd)"
exec "$DIR/nightfall-core.bin" --network devnet
LAUNCH
fi
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
    <key>NIGHTFALLDevelopmentProfile</key><string>$PROFILE</string>
    <key>NIGHTFALLNetwork</key><string>$NETWORK_ARG</string>
    <key>NIGHTFALLDataNamespace</key><string>$DATA_NAMESPACE</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>LSMinimumSystemVersion</key><string>$MIN_MACOS</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSHumanReadableCopyright</key><string>NIGHTFALLCOIN — fair launch, no premine</string>
</dict>
</plist>
PLIST

printf 'APPL????' > "$APP/Contents/PkgInfo"

cp "$GUIDE" "$WORK/READ-ME-FIRST.md"
cp "$GUIDE" "$APP/Contents/Resources/READ-ME-FIRST.md"
cat > "$APP/Contents/Resources/BUILD-INFO.txt" <<INFO
version=$VERSION
bundle_version=$APP_VERSION
bundle_identifier=$BUNDLE_ID
profile=$PROFILE
network=$NETWORK_ARG
data_namespace=$DATA_NAMESPACE
architecture=$EXPECTED_ARCH
target=$TRIPLE
signing=local-ad-hoc; not notarized
distribution=local-testing-only; not a production release
INFO
# Ad-hoc signing verifies bundle integrity locally; this is not notarization.
plutil -lint "$APP/Contents/Info.plist"
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"
ARCHIVE="$WORK/NIGHTFALL-Dev-$VERSION-macOS-$ARCH.zip"
ditto -c -k --sequesterRsrc --keepParent "$APP" "$ARCHIVE"
(cd "$WORK" && shasum -a 256 "NIGHTFALL-Dev-$VERSION-macOS-$ARCH.zip" > SHA256SUMS.txt)
echo "==> Dev artifact: $APP"
echo "==> Dev archive: $ARCHIVE"
echo "    No installed app or wallet data was changed."
