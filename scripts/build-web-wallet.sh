#!/usr/bin/env bash
# Compile nightfall-web to wasm32.
#
# Schreibt standardmaessig nach target/web-wallet-build/ und NICHT in den
# veroeffentlichten Kanal. website/public/wallet/ liegt nicht in Git: ein
# versehentlicher Schreibvorgang dort ist unwiderruflich und ersetzt die
# Wallet, die Nutzer mit echtem Guthaben oeffnen.
#
#   scripts/build-web-wallet.sh              -> target/web-wallet-build/
#   scripts/build-web-wallet.sh --out DIR    -> DIR
#   scripts/build-web-wallet.sh --origin     -> website/public/wallet-origin/pkg/
#   scripts/build-web-wallet.sh --publish    -> website/public/wallet/pkg/
#
# --publish ist eine Veroeffentlichung. Es verlangt einen sauberen Kanal
# vorher und aktualisiert scripts/published-channel.sha256 NICHT von selbst.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

LIVE_DIR="website/public/wallet/pkg"
ORIGIN_DIR="website/public/wallet-origin/pkg"
OUT_DIR="target/web-wallet-build"
DEST="dev"
EXPLICIT_OUT=0

while [ $# -gt 0 ]; do
  case "$1" in
    # Two destination flags is not "the last one wins" — it is a command whose
    # author did not decide where the build should land, and guessing on their
    # behalf is guessing about the channel.
    # The frozen 0.9.5 channel. One exact directory, a typed confirmation.
    --publish) [ "$DEST" = "dev" ] || { echo "nur ein Ziel angeben" >&2; exit 2; }
               DEST="publish"; shift ;;
    # The 1.0 wallet's own host. Deployed, but not the channel anyone with a
    # real balance is using today, so no typed confirmation is asked for.
    --origin)  [ "$DEST" = "dev" ] || { echo "nur ein Ziel angeben" >&2; exit 2; }
               DEST="origin"; shift ;;
    --out)
      [ $# -ge 2 ] || { echo "--out braucht ein Verzeichnis" >&2; exit 2; }
      [ "$EXPLICIT_OUT" -eq 0 ] || { echo "--out darf nur einmal vorkommen" >&2; exit 2; }
      OUT_DIR="$2"; EXPLICIT_OUT=1; shift 2 ;;
    -h|--help) sed -n '2,14p' "$0"; exit 0 ;;
    *) echo "unbekanntes Argument: $1" >&2; exit 2 ;;
  esac
done

if [ "$DEST" != "dev" ]; then
  [ "$EXPLICIT_OUT" -eq 0 ] || { echo "--$DEST und --out sind nicht kombinierbar" >&2; exit 2; }
  [ "$DEST" = "publish" ] && OUT_DIR="$LIVE_DIR" || OUT_DIR="$ORIGIN_DIR"
fi
# Resolve existing symlinks AND missing parents before any build or write. The
# returned physical path is also the one used for installation, not the input.
OUT_DIR="$(node scripts/web-wallet-output.mjs resolve "$ROOT" "$OUT_DIR" "$DEST")"
node scripts/check-published-channel.mjs

if [ "$DEST" = "publish" ]; then
  echo "== Veroeffentlichung in $LIVE_DIR =="
  echo "Der Kanal liegt nicht in Git. Pruefe zuerst, dass er unveraendert ist."
  printf 'Den LIVE-Kanal wirklich ersetzen? Tippe genau: veroeffentlichen\n> '
  read -r answer
  [ "$answer" = "veroeffentlichen" ] || { echo "abgebrochen." >&2; exit 1; }
fi

if ! command -v wasm-bindgen >/dev/null; then
  echo "install wasm-bindgen-cli 0.2.100 (must match crates/nightfall-web)" >&2
  echo "  cargo install wasm-bindgen-cli --version 0.2.100 --locked" >&2
  exit 1
fi
if [ "$(wasm-bindgen --version)" != "wasm-bindgen 0.2.100" ]; then
  echo "This project requires wasm-bindgen 0.2.100." >&2
  exit 1
fi

# wasm-release: no LTO, opt-level s. See Cargo.toml.
cargo +1.98.0 build --locked --offline --profile wasm-release \
  --target-dir "$ROOT/target/web-wallet-build-target" \
  --target wasm32-unknown-unknown -p nightfall-web
WEB_BINDINGS="$(mktemp -d "$ROOT/target/web-bindings-XXXXXX")"
wasm-bindgen --target web \
  --out-dir "$WEB_BINDINGS" \
  "$ROOT/target/web-wallet-build-target/wasm32-unknown-unknown/wasm-release/nightfall_web.wasm"
# Check the live channel again after a potentially long build. Installation
# checks output aliases again and uses new files + rename, never cp-through-link.
node scripts/check-published-channel.mjs
node scripts/web-wallet-output.mjs install "$ROOT" "$OUT_DIR" "$DEST" "$WEB_BINDINGS"

echo "wrote $OUT_DIR/"
ls -lh "$OUT_DIR"

if [ "$DEST" = "publish" ]; then
  echo
  echo "Der Kanal wurde ersetzt. scripts/published-channel.sha256 ist jetzt VERALTET."
  echo "Neue Summen (in die Datei uebernehmen, mit der neuen Version im Kopf):"
  node scripts/check-published-channel.mjs --print
  exit 0
fi

echo
echo "Der veroeffentlichte Kanal wurde nicht angefasst."
node scripts/check-published-channel.mjs
