#!/usr/bin/env bash
# Build the command-line binaries twice from the same commit, in two different
# directories, and compare them byte for byte.
#
# Roadmap 2027, milestone 8. Written 30 September 2026. Local only.
#
# This is the first question of reproducible builds: does the *same machine*
# produce the *same bytes* from the same source when only the checkout path
# differs? If not, nobody else can hope to match a published checksum. It does
# not answer the second question — does a *different* machine match — which
# needs someone outside the project (see docs/RELEASE-VERIFY.md).
#
#   scripts/release/compare-builds.sh [commit]      # default HEAD
#
# Offline: uses the local cargo registry cache. Writes
# output/release-compare/<timestamp>/report.md. Never uploads, signs or tags.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT=$(pwd)
REV=${1:-HEAD}
COMMIT=$(git rev-parse --verify --end-of-options "${REV}^{commit}")
TS=$(date -u +%Y-%m-%dT%H-%M-%SZ)
OUT="$ROOT/output/release-compare/$TS"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/nightfall-repro-XXXXXX")
mkdir -p "$OUT"
# Keep failure evidence; only remove successful disposable build trees.
finish() {
  local code=$?
  if [ "$code" -ne 0 ]; then
    printf "Build comparison failed (exit %s). No reproducibility result.\nTemporary evidence: %s\n" "$code" "$WORK" > "$OUT/FAILED.txt"
    echo "Build comparison failed; see $OUT/FAILED.txt" >&2
  fi
}
trap finish EXIT
BINS=(nightfalld nightfall-wallet nightfall-nodekit)
PKGS=(-p nightfall-node -p nightfall-wallet -p nightfall-nodekit)
TC="+1.98.0"

# Everything that commonly leaks the build environment into a Rust binary:
# the checkout path (panic messages, debug info), the cargo home, and time.
SOURCE_DATE_EPOCH=$(git log -1 --format=%ct "$COMMIT")
export SOURCE_DATE_EPOCH
export CARGO_INCREMENTAL=0

build() {
  local dir="$1"
  mkdir -p "$dir/src"
  git archive "$COMMIT" | tar -x -C "$dir/src"
  (
    cd "$dir/src"
    # Environment wrappers/encoded flags can override RUSTFLAGS or reuse a
    # shared target. Each invocation must actually compile its own archive.
    unset RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER CARGO_ENCODED_RUSTFLAGS
    unset RUSTC RUSTDOC RUSTDOCFLAGS CARGO_BUILD_TARGET CARGO_BUILD_RUSTFLAGS
    export CARGO_TARGET_DIR="$dir/target"
    export RUSTFLAGS="--remap-path-prefix=$dir/src=/build/nightfall --remap-path-prefix=$dir/target=/build/target --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo"
    cargo $TC build --release --locked --offline "${PKGS[@]}" > "$dir/build.log" 2>&1
  )
}

echo "Building $COMMIT twice (paths differ) …"
build "$WORK/a"
build "$WORK/b"

{
  echo "# Build comparison — $TS"
  echo
  echo "- commit: \`$COMMIT\`"
  echo "- rustc: $(rustc $TC --version)"
  echo "- host: $(uname -sm)"
  echo "- source: git archive of the resolved commit; uncommitted changes excluded"
  echo "- cargo target directories: separate for build A and B"
  echo "- Cargo.lock sha256: $(shasum -a 256 "$WORK/a/src/Cargo.lock" | cut -d' ' -f1)"
  echo "- SOURCE_DATE_EPOCH=$SOURCE_DATE_EPOCH, path prefixes remapped, CARGO_INCREMENTAL=0"
  echo
  echo "| Binary | Build A sha256 | Build B sha256 | Identical |"
  echo "|---|---|---|---|"
} > "$OUT/report.md"

same=0; diff=0
: > "$OUT/SHA256SUMS-a.txt"
: > "$OUT/SHA256SUMS-b.txt"
for b in "${BINS[@]}"; do
  A="$WORK/a/target/release/$b"; B="$WORK/b/target/release/$b"
  ha=$(shasum -a 256 "$A" | cut -d' ' -f1); hb=$(shasum -a 256 "$B" | cut -d' ' -f1)
  echo "$ha  $b" >> "$OUT/SHA256SUMS-a.txt"
  echo "$hb  $b" >> "$OUT/SHA256SUMS-b.txt"
  if [ "$ha" = "$hb" ]; then r=yes; same=$((same+1)); else r="**no**"; diff=$((diff+1)); fi
  echo "| $b | \`${ha:0:16}…\` | \`${hb:0:16}…\` | $r |" >> "$OUT/report.md"
done
{
  echo
  if [ "$diff" -eq 0 ]; then
    echo "All ${#BINS[@]} binaries are byte-identical across checkout paths on this machine."
  else
    echo "$diff of ${#BINS[@]} binaries differ. Inspect with \`cmp -l\` or diffoscope before claiming reproducibility."
  fi
  echo
  echo "Same machine, same toolchain. **Not** evidence that another machine reproduces these bytes."
  echo "Unsigned command-line binaries only; macOS bundle/signature reproducibility is outside this comparison."
} >> "$OUT/report.md"
cat "$OUT/report.md"
if [ "$diff" -eq 0 ]; then rm -rf "$WORK"; fi
[ "$diff" -eq 0 ]
