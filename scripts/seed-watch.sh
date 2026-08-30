#!/usr/bin/env bash
# Watch both seeds and complain when something is wrong.
#
# Written after 26 August 2026, when the light API on one seed had been dead
# for hours and a *user* found it, not a monitor. Everything this checks is
# something that has actually gone wrong at least once:
#
#   * a seed stops answering            (Vultr, 20 and 23 Aug)
#   * a seed answers but too slowly     (8.2 s against a 6 s timeout)
#   * a seed stops following the chain  (stuck after a deep fork)
#   * the two seeds disagree on the tip (the thing nobody would notice)
#   * the mempool creeps toward its cap (the corpses problem, 0.8.2)
#
# Deliberately dumb: curl, one file of state, no dependencies. It runs from
# cron on the machine that has spare capacity, checks *both* seeds from the
# outside, and therefore also fails when its own host loses the network — which
# is the correct behaviour for a watchdog.
#
# Install on Contabo:
#   install -m 755 seed-watch.sh /usr/local/bin/seed-watch
#   ( crontab -l 2>/dev/null; echo '*/5 * * * * /usr/local/bin/seed-watch' ) | crontab -
#
# Alerts go to the log always, and to a webhook if one is configured:
#   echo 'https://discord.com/api/webhooks/…' > /etc/nightfall-watch-webhook
#   chmod 600 /etc/nightfall-watch-webhook
# Without the file it stays quiet in the log, which is still better than
# nothing and needs no secret to exist.

set -uo pipefail

LOG=/var/log/nightfall-watch.log
STATE=/var/lib/nightfall-watch.state
WEBHOOK_FILE=/etc/nightfall-watch-webhook

SEEDS=("seed.nightfallcoin.org" "seed1.nightfallcoin.org")

# A light request must beat the website Worker's own timeout, or phones fail
# over. Anything slower is broken even if it eventually answers.
MAX_SECS=4
# Two seeds are never in lockstep — a block lands on one first. More than this
# many apart for two runs in a row is a real divergence.
MAX_DRIFT=5
# The mempool cap is 10 000. At this point something is wrong upstream.
MAX_MEMPOOL=2000

now() { date -u +"%Y-%m-%dT%H:%M:%SZ"; }
log() { echo "$(now) $*" >> "$LOG"; }

alert() {
    local msg="$1"
    log "ALERT $msg"
    [ -r "$WEBHOOK_FILE" ] || return 0
    local url
    url="$(head -n1 "$WEBHOOK_FILE")"
    [ -n "$url" ] || return 0
    curl -s -m 10 -H 'content-type: application/json' \
        -d "$(printf '{"content":"NIGHTFALL watch: %s"}' "$(echo "$msg" | tr -d '"')")" \
        "$url" >/dev/null 2>&1
}

heights=()
problems=()

for host in "${SEEDS[@]}"; do
    body=$(mktemp)
    secs=$(curl -s -m "$((MAX_SECS + 4))" -o "$body" -w '%{time_total}' \
        -X POST -H 'content-type: application/json' \
        -d '{"method":"status","params":{},"id":1}' \
        "http://$host/" 2>/dev/null)
    rc=$?

    if [ $rc -ne 0 ] || [ ! -s "$body" ]; then
        problems+=("$host does not answer")
        heights+=("0")
        rm -f "$body"
        continue
    fi

    read -r h mem loading <<<"$(python3 - "$body" <<'PY'
import json, sys
try:
    d = json.load(open(sys.argv[1]))
    r = d.get("result", d)
    print(r.get("tip_height", 0), r.get("mempool", 0), str(r.get("loading", False)).lower())
except Exception:
    print(0, 0, "true")
PY
)"
    rm -f "$body"

    # A truncated or unreadable body parses as height 0. That is not
    # "the chain is at genesis" — writing 0 into the state file made the
    # next check scream "has not moved past block 0".
    if [ -z "$h" ] || [ "$h" -eq 0 ] 2>/dev/null; then
        problems+=("$host answered with no usable height")
        heights+=("0")
        continue
    fi
    heights+=("$h")

    # `1` sorts before `0.9`, so compare as numbers, not strings.
    if awk -v a="$secs" -v b="$MAX_SECS" 'BEGIN{exit !(a>b)}'; then
        problems+=("$host answered in ${secs}s, slower than ${MAX_SECS}s")
    fi
    [ "$loading" = "true" ] && problems+=("$host is still loading its chain")
    [ "$mem" -gt "$MAX_MEMPOOL" ] 2>/dev/null && problems+=("$host mempool at $mem")

    prev=$(grep "^$host " "$STATE" 2>/dev/null | awk '{print $2}')
    if [ -n "${prev:-}" ] && [ "$h" -le "$prev" ] 2>/dev/null; then
        problems+=("$host has not moved past block $h since the last check")
    fi
    grep -v "^$host " "$STATE" 2>/dev/null > "$STATE.tmp" || true
    echo "$host $h" >> "$STATE.tmp"
    mv "$STATE.tmp" "$STATE"
done

if [ "${heights[0]}" -gt 0 ] 2>/dev/null && [ "${heights[1]}" -gt 0 ] 2>/dev/null; then
    drift=$(( heights[0] - heights[1] ))
    [ "$drift" -lt 0 ] && drift=$(( -drift ))
    if [ "$drift" -gt "$MAX_DRIFT" ]; then
        problems+=("seeds $drift blocks apart (${heights[0]} vs ${heights[1]})")
    fi
fi

if [ ${#problems[@]} -eq 0 ]; then
    log "ok  ${SEEDS[0]}=${heights[0]}  ${SEEDS[1]}=${heights[1]}"
    exit 0
fi

for p in "${problems[@]}"; do alert "$p"; done
exit 1
