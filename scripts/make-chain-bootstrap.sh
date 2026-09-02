#!/usr/bin/env bash
# Pack the chain so a newcomer does not have to fetch it block by block.
#
#   ./scripts/make-chain-bootstrap.sh [--datadir DIR] [--out DIR]
#
# Produces, under --out (default ./bootstrap):
#   nightfall-chain-<height>-<date>.bin    the chain file, as-is
#   nightfall-chain-<height>-<date>.json   height, tip, size, checksum
#   SHA256SUMS-chain-<date>.txt
#
# Not compressed, on purpose. Measured on the live chain: zstd -3 takes 7 %
# off, zstd -19 takes 8 % — the file is mostly hashes and range proofs, and
# those do not compress. Charging every downloader an unpacking step and a
# tool they may not have, to save eight percent, is a bad trade. The web
# server can still gzip it in transit for whoever benefits.
#
# The win here was never the byte count. It is that 95k blocks arrive as one
# HTTP download instead of a block-by-block negotiation with a handful of
# peers.
#
# What this deliberately does NOT include, and why the script would be
# dangerous without that rule:
#
#   chain-meta.json   carries the validation record. A node that finds one
#                     matching its own installation id skips proof-of-work
#                     checking for the whole chain. Shipping it would mean
#                     every downloader trusts whatever we put in the file
#                     instead of checking it. Measured before the id was
#                     added: copying blocks.bin alone logged "re-verifying
#                     proof of work for the whole chain"; copying it together
#                     with the metadata loaded in silence.
#   install-id        the thing that makes a validation record ours. Publishing
#                     it re-opens the same hole from the other side.
#   *.seed, wallet*   keys. Never.
#
# So the archive is one file: the blocks. The receiving node re-derives every
# hash and re-checks every proof of work, exactly as if the blocks had arrived
# over the network. This saves the download and the peer round-trips. It does
# not save the verification, and it is not supposed to.
set -euo pipefail

DATADIR=""
OUT=""
while [ $# -gt 0 ]; do
    case "$1" in
        --datadir) DATADIR="$2"; shift 2 ;;
        --out)     OUT="$2";     shift 2 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${OUT:-$ROOT/bootstrap}"

# Default to the mainnet datadir this platform uses.
if [ -z "$DATADIR" ]; then
    case "$(uname -s)" in
        Darwin) DATADIR="$HOME/Library/Application Support/nightfall/mainnet/n8" ;;
        *)      DATADIR="$HOME/.local/share/nightfall/mainnet/n8" ;;
    esac
fi

BLOCKS="$DATADIR/blocks.bin"
META="$DATADIR/chain-meta.json"

[ -f "$BLOCKS" ] || { echo "no blocks.bin in $DATADIR" >&2; exit 1; }
[ -f "$META" ]   || { echo "no chain-meta.json in $DATADIR — cannot read the height" >&2; exit 1; }

# A node holding the directory may be mid-write. Copying a half-written
# blocks.bin produces an archive that fails to load for everyone who
# downloads it, and the failure looks like corruption rather than a race.
#
# Checked through python3's fcntl rather than the `flock` command: that
# command is util-linux and does not exist on macOS, so the first version of
# this guard was skipped entirely there — `command -v flock` failed, the test
# was silently not run, and the script happily archived a live datadir. A
# guard that quietly does nothing on half the platforms is worse than none,
# because it reads like protection.
if [ -f "$DATADIR/.nightfall-lock" ]; then
    if ! python3 - "$DATADIR/.nightfall-lock" <<'PYLOCK'
import fcntl, sys
try:
    fh = open(sys.argv[1], "r+")
    fcntl.flock(fh, fcntl.LOCK_EX | fcntl.LOCK_NB)
except OSError:
    sys.exit(1)
sys.exit(0)
PYLOCK
    then
        echo "a node is running on $DATADIR — stop it first, or the archive" >&2
        echo "may capture a partially written chain file." >&2
        exit 1
    fi
fi

read_json() { grep -o "\"$1\"[[:space:]]*:[[:space:]]*[^,}]*" "$META" | head -1 | sed 's/.*:[[:space:]]*//; s/"//g'; }

HEIGHT="$(read_json block_count)"
TIP="$(read_json validated_tip)"
GENESIS="$(read_json genesis_hash)"
NETWORK="$(read_json network)"
PRUNED="$(read_json pruned)"

[ -n "$HEIGHT" ] || { echo "could not read block_count from $META" >&2; exit 1; }

if [ "$PRUNED" = "true" ]; then
    echo "this datadir is pruned — the old bodies are gone and the archive" >&2
    echo "would be useless to anyone syncing from genesis." >&2
    exit 1
fi

DATE="$(date -u +%Y-%m-%d)"
BASE="nightfall-chain-${HEIGHT}-${DATE}"
mkdir -p "$OUT"

echo "==> chain bootstrap"
echo "    network...... $NETWORK"
echo "    height....... $HEIGHT"
echo "    tip.......... $TIP"
echo "    raw.......... $(du -h "$BLOCKS" | cut -f1)"

# Copy rather than hardlink: the source may keep growing under a node that
# starts up again while this runs, and a hardlink would follow it.
cp "$BLOCKS" "$OUT/$BASE.bin"

RAW_BYTES="$(wc -c < "$OUT/$BASE.bin" | tr -d ' ')"
RAW_SHA="$(shasum -a 256 "$OUT/$BASE.bin" | cut -d' ' -f1)"

cat > "$OUT/$BASE.json" <<JSON
{
  "network": "$NETWORK",
  "height": $HEIGHT,
  "tip": "$TIP",
  "genesis": "$GENESIS",
  "taken_utc": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "archive": "$BASE.bin",
  "archive_bytes": $RAW_BYTES,
  "blocks_bin_sha256": "$RAW_SHA",
  "verified_on_import": true,
  "note": "Contains blocks only. Your node re-checks every proof of work; this archive replaces the download, not the verification."
}
JSON

( cd "$OUT" && shasum -a 256 "$BASE.bin" "$BASE.json" > "SHA256SUMS-chain-${DATE}.txt" )

# Belt and braces: prove the rule at the top of this file actually held.
for forbidden in chain-meta.json install-id core.seed miner.seed wallet.json; do
    if [ -e "$OUT/$forbidden" ]; then
        echo "REFUSING: $forbidden ended up in the output directory" >&2
        exit 1
    fi
done

echo "    archive...... $BASE.bin  ($(du -h "$OUT/$BASE.bin" | cut -f1))"
echo "    manifest..... $BASE.json"
echo "    checksums.... SHA256SUMS-chain-${DATE}.txt"
echo "==> done. Output in $OUT"
