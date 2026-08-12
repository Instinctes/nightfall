#!/usr/bin/env bash
# Start NIGHTFALLCOIN Core Wallet (GUI: mine + send + receive)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${ROOT}/target/release/nightfall-core"
NET="${1:-mainnet}"

if [[ ! -x "$BIN" ]]; then
  echo "Building Core Wallet (release)…"
  (cd "$ROOT" && cargo build --release -p nightfall-core)
fi

export NF_DATA="${NF_DATA:-$HOME/nightfall-${NET}}"
# Optional: SEED_NODE=ip:17891 to connect to another miner
echo "Network: $NET"
echo "Data:    $NF_DATA"
echo "Starting Core Wallet…"
exec "$BIN" --network "$NET"
