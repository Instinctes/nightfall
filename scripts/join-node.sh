#!/usr/bin/env bash
# Join NIGHTFALLCOIN network as miner (set SEED_NODE=host:17891)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${ROOT}/target/release/nightfalld"
DATA="${NF_DATA:-$HOME/nightfall-mainnet}"
NET="${NF_NETWORK:-mainnet}"
SEED_NODE="${SEED_NODE:?Set SEED_NODE=ip:17891 of first node}"

if [[ ! -x "$BIN" ]]; then
  echo "Building release nightfalld..."
  (cd "$ROOT" && cargo build --release -p nightfall-node)
fi

mkdir -p "$DATA"
"$BIN" --network "$NET" --datadir "$DATA" init
echo "Connecting to $SEED_NODE"
exec "$BIN" --network "$NET" --datadir "$DATA" run \
  --listen "${NF_LISTEN:-0.0.0.0:17891}" \
  --rpc-listen "${NF_RPC:-127.0.0.1:17881}" \
  --connect "$SEED_NODE" \
  --mine \
  --miner-seed miner.seed
