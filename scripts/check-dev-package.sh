#!/usr/bin/env bash
# Read-only profile checks; never builds, opens a wallet or publishes anything.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUILDER="$ROOT/scripts/build-macos-dev-app.sh"
config="$(env NIGHTFALL_DEV_MAINNET=1 NIGHTFALL_DEV_APP_NAME=wrong \
    NIGHTFALL_DEV_BUNDLE_ID=wrong "$BUILDER" arm64 --print-config)"
for expected in \
    'version=1.0.0-dev.2' \
    'app_version=1.0.0' \
    'app_name=NIGHTFALL Dev 1.0.0' \
    'bundle_id=cash.nightfall.wallet.dev100preview' \
    'profile=isolated-1.0.0-dev.2' \
    'network=devnet' \
    'data_namespace=wallet-1.0.0-dev.2' \
    'target=aarch64-apple-darwin'; do
    if ! printf '%s\n' "$config" | /usr/bin/grep -Fqx "$expected"; then
        printf 'Missing profile property: %s\n' "$expected" >&2; exit 1
    fi
done
intel="$("$BUILDER" intel --print-config)"
[[ "$intel" == *'target=x86_64-apple-darwin'* ]]
if "$BUILDER" arm64 intel --print-config >/dev/null 2>&1; then exit 1; fi
if "$BUILDER" --network mainnet --print-config >/dev/null 2>&1; then exit 1; fi
if "$BUILDER" --datadir /not-a-wallet --print-config >/dev/null 2>&1; then exit 1; fi
bash -n "$BUILDER"
printf 'Dev package profiles passed: isolated dev.2 identity, mainnet-env veto, architectures, rejected overrides.\n'
