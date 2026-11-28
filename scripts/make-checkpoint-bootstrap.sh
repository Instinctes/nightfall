#!/usr/bin/env bash
# OFFLINE MAINTAINER TOOL. Never run on a live datadir.
# Usage:
# scripts/make-checkpoint-bootstrap.sh --datadir OFFLINE_ARCHIVE --out OUTPUT

set -Eeuo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
DATADIR=""
OUT=""

while (( $# )); do
  case "$1" in
    --datadir)
      [[ $# -ge 2 ]] || exit 2
      DATADIR="$2"
      shift 2
      ;;
    --out)
      [[ $# -ge 2 ]] || exit 2
      OUT="$2"
      shift 2
      ;;
    *)
      printf 'Unknown parameter: %s\n' "$1" >&2
      exit 2
      ;;
  esac
done

[[ -n "$DATADIR" && -n "$OUT" ]] || {
  echo 'Require --datadir and --out' >&2
  exit 2
}

[[ -d "$DATADIR" && -f "$DATADIR/blocks.bin" ]] || {
  echo 'An offline full blocks.bin is required' >&2
  exit 1
}

DATADIR="$(cd "$DATADIR" && pwd -P)"

mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd -P)"

[[ "$OUT" != "$DATADIR" ]] || {
  echo 'Output must not equal the datadir' >&2
  exit 1
}

[[ "$OUT" != "$DATADIR"/* ]] || {
  echo 'Output must not be inside the datadir' >&2
  exit 1
}

# Preserve the existing archive producer's lock and pruning checks.
"$ROOT/scripts/make-chain-bootstrap.sh" \
  --datadir "$DATADIR" \
  --out "$OUT"

DATE="$(date -u +%Y-%m-%d)"
META="$DATADIR/chain-meta.json"

HEIGHT="$(python3 - "$META" <<'PYMETA'
import json
import sys

with open(sys.argv[1], encoding='utf-8') as f:
    value = json.load(f)

if value.get('pruned') is True:
    raise SystemExit('Pruned datadir is not eligible')

if value.get('network') not in ('Mainnet', 'mainnet'):
    raise SystemExit('Only mainnet is eligible')

print(value['block_count'])
PYMETA
)"

ARCHIVE_MANIFEST="$OUT/nightfall-chain-${HEIGHT}-${DATE}.json"

[[ -f "$ARCHIVE_MANIFEST" ]] || {
  echo 'Expected archive manifest absent' >&2
  exit 1
}

TMP="$OUT/.nightfall-checkpoint-$$.bin"

trap 'rm -f "$TMP"' EXIT

REPORT="$(cd "$ROOT" && cargo run --locked --release \
  -p nightfall-node -- \
  --network mainnet \
  --datadir "$DATADIR" \
  export-checkpoint-snapshot \
  --out "$TMP")"

printf '%s\n' "$REPORT"

PIN="$(printf '%s\n' "$REPORT" |
  sed -n 's/^checkpoint\.\.*[[:space:]]*//p' |
  tail -n 1)"

[[ "$PIN" =~ ^[0-9]+$ ]] || {
  echo 'Could not determine compiled checkpoint pin' >&2
  exit 1
}

SNAP="$OUT/nightfall-checkpoint-${PIN}-${DATE}.bin"

[[ ! -e "$SNAP" ]] || {
  echo 'Refusing to overwrite an existing checkpoint' >&2
  exit 1
}

mv "$TMP" "$SNAP"

python3 - "$ARCHIVE_MANIFEST" "$SNAP" "$OUT/bootstrap.json" "$PIN" <<'PYMAN'
import hashlib
import json
import os
import sys
from pathlib import Path

archive = Path(sys.argv[1])
snap = Path(sys.argv[2])
out = Path(sys.argv[3])
pin = int(sys.argv[4])

manifest = json.loads(archive.read_text(encoding='utf-8'))

digest = hashlib.sha256()

with snap.open('rb') as source:
    for chunk in iter(lambda: source.read(1024 * 1024), b''):
        digest.update(chunk)

size = snap.stat().st_size

if not 0 < size <= 512 * 1024 * 1024:
    raise SystemExit('Checkpoint size outside importer limit')

manifest['checkpoint'] = {
    'height': pin,
    'file': snap.name,
    'bytes': size,
    'sha256': digest.hexdigest(),
}

temporary = out.with_suffix('.tmp')

with temporary.open('w', encoding='utf-8') as target:
    json.dump(manifest, target, indent=2, sort_keys=True)
    target.write('\n')
    target.flush()
    os.fsync(target.fileno())

os.replace(temporary, out)

print('CHECKPOINT MANIFEST READY:', out)
PYMAN

(
  cd "$OUT"
  shasum -a 256 \
    "$(basename "$SNAP")" \
    bootstrap.json \
    > "SHA256SUMS-checkpoint-${DATE}.txt"
)

echo 'REVIEW REQUIRED: publish release assets and deploy bootstrap.json separately.'
