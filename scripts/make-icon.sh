#!/usr/bin/env bash
# Generate assets/AppIcon.icns from a square PNG (1024x1024 recommended).
#
#   ./scripts/make-icon.sh path/to/icon.png
set -euo pipefail

SRC="${1:?usage: make-icon.sh <source.png>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SET="$ROOT/assets/AppIcon.iconset"
OUT="$ROOT/assets/AppIcon.icns"

mkdir -p "$SET"
for size in 16 32 128 256 512; do
    sips -z $size $size "$SRC" --out "$SET/icon_${size}x${size}.png" >/dev/null
    sips -z $((size * 2)) $((size * 2)) "$SRC" \
        --out "$SET/icon_${size}x${size}@2x.png" >/dev/null
done

iconutil -c icns "$SET" -o "$OUT"
echo "wrote $OUT"
