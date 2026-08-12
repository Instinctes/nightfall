#!/usr/bin/env bash
# Start NIGHTFALLCOIN mainnet first node (miner + P2P + local RPC)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${ROOT}/target/release/nightfalld"
DATA="${NF_DATA:-$HOME/nightfall-mainnet}"
NET="${NF_NETWORK:-mainnet}"

if [[ ! -x "$BIN" ]]; then
  echo "Building release nightfalld..."
  (cd "$ROOT" && cargo build --release -p nightfall-node)
fi

mkdir -p "$DATA"
"$BIN" --network "$NET" --datadir "$DATA" init
echo "Starting first node — open TCP port for P2P (mainnet 17891)"
exec "$BIN" --network "$NET" --datadir "$DATA" run \
  --listen "${NF_LISTEN:-0.0.0.0:17891}" \
  --rpc-listen "${NF_RPC:-127.0.0.1:17881}" \
  --mine \
  --miner-seed miner.seed
