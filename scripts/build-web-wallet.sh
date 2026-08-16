#!/usr/bin/env bash
# Compile nightfall-web to wasm32 and emit website/public/wallet/pkg/.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if ! command -v wasm-bindgen >/dev/null; then
  echo "install wasm-bindgen-cli 0.2.100 (must match crates/nightfall-web)" >&2
  echo "  cargo install wasm-bindgen-cli --version 0.2.100 --locked" >&2
  exit 1
fi

# wasm-release: no LTO, opt-level s. See Cargo.toml.
cargo build --profile wasm-release --target wasm32-unknown-unknown -p nightfall-web
wasm-bindgen --target web \
  --out-dir website/public/wallet/pkg \
  target/wasm32-unknown-unknown/wasm-release/nightfall_web.wasm

echo "wrote website/public/wallet/pkg/"
ls -lh website/public/wallet/pkg
