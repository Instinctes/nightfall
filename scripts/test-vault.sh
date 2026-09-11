#!/usr/bin/env bash
# Pre-release Vault/Recovery checks. Uses unfunded fixtures, never wallet datadirs.
# Does not build into website/public, modify release channels, deploy or use Git.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
VAULT_TOOLCHAIN=1.98.0
VAULT_TARGET="$ROOT/target"
command -v wasm-bindgen >/dev/null
command -v node >/dev/null
if [[ "$(wasm-bindgen --version)" != "wasm-bindgen 0.2.100" ]]; then
    printf '%s\n' 'This project requires wasm-bindgen 0.2.100.' >&2
    exit 1
fi

cargo +"$VAULT_TOOLCHAIN" test --target-dir "$VAULT_TARGET" -p nightfall-wallet -p nightfall-crypto \
    --lib --release --locked --offline -- --test-threads=2
cargo +"$VAULT_TOOLCHAIN" test --target-dir "$VAULT_TARGET" -p nightfall-core \
    --bin nightfall-core --locked --offline -- --test-threads=2
cargo +"$VAULT_TOOLCHAIN" build --target-dir "$VAULT_TARGET" --profile wasm-release \
    --target wasm32-unknown-unknown -p nightfall-web --locked --offline

mkdir -p "$VAULT_TARGET/vault-tests"
VAULT_RUN_DIR="$(mktemp -d "$VAULT_TARGET/vault-tests/run-XXXXXX")"
wasm-bindgen --target nodejs --out-dir "$VAULT_RUN_DIR/bindings" \
    "$VAULT_TARGET/wasm32-unknown-unknown/wasm-release/nightfall_web.wasm"
cargo +"$VAULT_TOOLCHAIN" run --target-dir "$VAULT_TARGET" -p nightfall-wallet --example vault_interop \
    --release --locked --offline -- create "$VAULT_RUN_DIR/native.vault"
# A second fixture that actually holds coins. The browser side cannot mint its
# own — a light wallet receives by scanning real output cryptography — so the
# coins are minted natively, in blocks that exist only in that process, and
# travel across inside the sealed vault. Public zero-entropy key, never funded.
cargo +"$VAULT_TOOLCHAIN" run --target-dir "$VAULT_TARGET" -p nightfall-wallet --example vault_interop \
    --release --locked --offline -- create-funded "$VAULT_RUN_DIR/funded.vault"
node scripts/check-vault-wasm.cjs "$VAULT_RUN_DIR/bindings/nightfall_web.js" \
    "$VAULT_RUN_DIR/native.vault" "$VAULT_RUN_DIR/wasm.vault" "$VAULT_RUN_DIR/funded.vault"
cargo +"$VAULT_TOOLCHAIN" run --target-dir "$VAULT_TARGET" -p nightfall-wallet --example vault_interop \
    --release --locked --offline -- check "$VAULT_RUN_DIR/wasm.vault"
printf 'Vault/Recovery checks passed. Public test fixtures only: %s\n' "$VAULT_RUN_DIR"
