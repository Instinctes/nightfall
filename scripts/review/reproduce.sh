#!/usr/bin/env bash
# Reproduce every claim in docs/review/EXTERNAL-REVIEW-PACKAGE.md §4.
#
# Offline. Never contacts mainnet, never deploys, never needs root. Each step's
# exit code and the tail of its output go into output/review/<timestamp>/,
# and summary.md lists pass/fail per step. A failing step does not stop the
# run: a reviewer wants the whole picture, not the first red line.
#
#   scripts/review/reproduce.sh            # everything (≈10 min on an M-series Mac)
#   QUICK=1 scripts/review/reproduce.sh    # skip the workspace test run and the labs
set -u
cd "$(dirname "$0")/../.."
ROOT=$(pwd)
TS=$(date -u +%Y-%m-%dT%H-%M-%SZ)
OUT="$ROOT/output/review/$TS"
mkdir -p "$OUT"
TC="+1.98.0"

{
  echo "# Reproduction run $TS"
  echo
  echo "- commit: $(git rev-parse HEAD 2>/dev/null || echo unknown)$( [ -n "$(git status --porcelain --untracked-files=no 2>/dev/null)" ] && echo ' (uncommitted changes present)')"
  echo "- rustc: $(rustc $TC --version 2>/dev/null || echo 'toolchain 1.98.0 missing')"
  echo "- node: $(node --version 2>/dev/null || echo missing)"
  echo "- host: $(uname -sm)"
  echo
  echo "| Step | Result | Seconds | Log |"
  echo "|---|---|---|---|"
} > "$OUT/summary.md"

pass=0; fail=0; skipped=0
step() {
  local name="$1"; shift
  local log="$OUT/$(echo "$name" | tr ' /' '__').log"
  local t0=$(date +%s)
  printf '  %-34s ' "$name"
  if "$@" > "$log" 2>&1; then
    r=pass; pass=$((pass+1)); echo ok
  else
    r="**FAIL**"; fail=$((fail+1)); echo FAIL
  fi
  echo "| $name | $r | $(( $(date +%s) - t0 )) | $(basename "$log") |" >> "$OUT/summary.md"
  [ "$r" = pass ]
}

echo "Reproducing review claims → $OUT"
step "rustfmt"                cargo $TC fmt --all -- --check
step "clippy -D warnings"     cargo $TC clippy --workspace --all-targets --offline --locked -- -D warnings
step "exploit regressions"    cargo $TC test -p nightfall-ledger --test exploit_regression --offline --locked
step "chain rules"            cargo $TC test -p nightfall-consensus --test chain_rules --offline --locked
step "reorg persistence"      cargo $TC test -p nightfall-storage --offline --locked
step "light API allow-list"   cargo $TC test -p nightfall-node --lib mobile --offline --locked
step "till settlement"        cargo $TC test -p nightfall-wallet --lib counter --offline --locked
step "node kit"               cargo $TC test -p nightfall-nodekit --offline --locked
if [ -z "${QUICK:-}" ]; then
  step "workspace tests"      cargo $TC test --workspace --offline --locked
fi
step "release dependency policy" python3 scripts/test_release_lock_policy.py
step "release provenance policy" python3 scripts/test_release_provenance.py
step "installer swap ownership" python3 scripts/test_installer_swap_safety.py
step "installer source policy" python3 scripts/test_node_kit_source_policy.py
step "host preflight" python3 scripts/test_node_kit_preflight.py
step "independent build comparison" python3 scripts/test_compare_independent_builds.py
# Rebuild private bindings first; no deployed browser asset is overwritten.
if step "private WASM build" bash scripts/build-web-wallet.sh --out target/roadmap-110-web; then
  export NIGHTFALL_WASM_DIR="$ROOT/target/roadmap-110-web"
else
  echo 'Private WASM build failed; refusing stale binding evidence.' >> "$OUT/summary.md"
  exit 1
fi
step "emission"               node scripts/check-emission.mjs
step "web wallet (WASM)"      node scripts/check-web-wallet-v104.mjs
step "pay link transport" node scripts/check-pay-link.mjs
step "pay links (WASM)"       node scripts/check-pay-link-wasm.mjs
step "build runner isolation" python3 scripts/check-build-comparison.py
step "checkout HTTP boundary" node scripts/check-checkout-service.mjs
step "backup rehearsal"       node scripts/check-backup-rehearsal.mjs
step "scan loop"              node scripts/check-wallet-sync.mjs
step "my node fallback"       node scripts/check-wallet-my-node.mjs
step "scan proxy"             node scripts/check-wallet-scan-proxy.mjs
step "website security"       node scripts/check-website-security.mjs
if [ -z "${QUICK:-}" ]; then
  if step "build devnet binaries" cargo $TC build --offline --locked --bins -p nightfall-node -p nightfall-nodekit -p nightfall-wallet; then
    step "light HTTP resource boundary" python3 scripts/test-light-api-boundary.py
    step "reliability lab" node scripts/lab/run-lab.mjs --out "output/review/$TS/lab"
    step "checkout devnet" node scripts/lab/checkout-devnet.mjs --out "output/review/$TS/lab"
  else
    skipped=$((skipped+2))
    echo '| Process labs | skipped: binary build failed; stale binaries are not evidence | — | — |' >> "$OUT/summary.md"
  fi
fi

{
  echo
  echo "**$pass passed, $fail failed, $skipped skipped.** Logs beside this file."
  echo
  echo "This run reproduces the project's own tests. It is not a review."
} >> "$OUT/summary.md"
echo
echo "$pass passed, $fail failed — $OUT/summary.md"
[ "$fail" -eq 0 ]
