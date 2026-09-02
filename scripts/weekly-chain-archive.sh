#!/usr/bin/env bash
# The whole weekly chore, one command:
#
#     ./scripts/weekly-chain-archive.sh
#
# Builds a fresh archive from the node's data folder, puts it on the GitHub
# release in place of last week's, points the website at it, deploys, and then
# checks the live site actually serves what it just published.
#
# Only the copy needs the wallet closed, and that takes seconds. The script
# waits for you to close it and tells you the moment you can start it again —
# the upload, which is the slow part, runs fine with the wallet back up.
#
# Options:
#   --check          run the preflight only and stop. Safe with the wallet
#                    open; use it to find a broken login before you close
#                    anything.
#   --keep N         how many archives stay on the release (default 2)
#   --datadir DIR    override the data folder
#   --service NAME   systemd unit to stop for the copy. For an unattended
#                    archive node; ignored on macOS, where you close the
#                    wallet yourself.
#   --no-deploy      publish to GitHub, skip the website
#
# On a server, run it against a node kept for this purpose, never a seed. A
# seed that goes down every week is a seed that is missing exactly when the
# network is smallest and needs it most.
set -euo pipefail

KEEP=2
DATADIR=""
SERVICE=""
CHECK_ONLY=0
SELF_TEST=0
DEPLOY=1
TAG="chain-bootstrap"
REPO="Instinctes/nightfall"

while [ $# -gt 0 ]; do
    case "$1" in
        --check)      CHECK_ONLY=1; shift ;;
        --self-test)  SELF_TEST=1; shift ;;
        --keep)       KEEP="$2"; shift 2 ;;
        --datadir)    DATADIR="$2"; shift 2 ;;
        --service)    SERVICE="$2"; shift 2 ;;
        --no-deploy)  DEPLOY=0; shift ;;
        --tag)        TAG="$2"; shift 2 ;;
        --repo)       REPO="$2"; shift 2 ;;
        -h|--help)    sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MIRROR="$ROOT/github"
PAGE_REL="website/public/chain/bootstrap.json"

say()  { printf '\n\033[1m%s\033[0m\n' "$*"; }
step() { printf '    %s\n' "$*"; }
die()  { printf '\n\033[31m%s\033[0m\n' "$*" >&2; exit 1; }

# Archive names newest first, so the tail of the list is what gets retired.
#
# Sorted on the height as a NUMBER. `sort -r` on the whole name looks like it
# works and does, right up to the moment the height gains a digit: as a string
# "nightfall-chain-99981" sorts above "nightfall-chain-100248", so the newest
# archive would land at the bottom of the list and be deleted — by the same
# run that had just uploaded it, leaving the website pointing at a file the
# script had thrown away. At 95,867 blocks that trap was about two weeks out.
#
# Field 3 of nightfall-chain-<height>-<date>.bin split on "-" is the height.
newest_first() { sort -t- -k3,3nr; }

# Proves the above rather than asserting it. Run with --self-test; costs
# nothing and needs no network, so CI can run it too.
self_test() {
    local got want
    got="$(printf '%s\n' \
        nightfall-chain-99981-2026-09-16.bin \
        nightfall-chain-100248-2026-09-23.bin \
        nightfall-chain-95867-2026-09-02.bin | newest_first | head -1)"
    want="nightfall-chain-100248-2026-09-23.bin"
    if [ "$got" != "$want" ]; then
        echo "self-test FAILED: newest is '$got', expected '$want'" >&2
        echo "The retirement step would delete the newest archive." >&2
        return 1
    fi
    echo "self-test ok — ordering survives the height gaining a digit"
}

if [ "$SELF_TEST" = "1" ]; then
    self_test
    exit $?
fi

if [ -z "$DATADIR" ]; then
    case "$(uname -s)" in
        Darwin) DATADIR="$HOME/Library/Application Support/nightfall/mainnet/n8" ;;
        *)      DATADIR="$HOME/.local/share/nightfall/mainnet/n8" ;;
    esac
fi

# ----------------------------------------------------------------- preflight ---
#
# Everything that can be wrong is checked before anything expensive happens.
# The alternative is finding out that a login expired after you have closed
# your wallet and copied 150 MB, which is how a five-minute chore becomes an
# evening.
say "==> preflight"

[ -d "$MIRROR/.git" ] || die "no git mirror at $MIRROR"
[ -f "$DATADIR/blocks.bin" ] || die "no blocks.bin in $DATADIR — wrong data folder?"
step "data folder... $DATADIR"

command -v gh >/dev/null || die "gh is not installed"
gh auth status >/dev/null 2>&1 || die "gh is not logged in — run: gh auth login"
step "github....... logged in"

if [ "$DEPLOY" = "1" ]; then
    # wrangler keeps its own OAuth login in ~/.wrangler. An earlier version of
    # the deploy step passed CLOUDFLARE_API_TOKEN read from a file that does
    # not exist on this machine: the variable came out empty, wrangler fell
    # back to that stored login, and the deploy worked for a reason entirely
    # unlike the one written down. So it is checked for what it actually uses.
    if ! ( cd "$MIRROR/website" && npx wrangler whoami >/dev/null 2>&1 ); then
        die "wrangler is not logged in — run: cd github/website && npx wrangler login"
    fi
    step "cloudflare... logged in"
fi

# A stray edit to this file in the mirror would be swept into the commit
# below. Only bootstrap.json is committed, but if someone left an unrelated
# change in exactly that file, better to say so now.
if ! git -C "$MIRROR" diff --quiet -- "$PAGE_REL" 2>/dev/null; then
    die "$PAGE_REL already has uncommitted changes in the mirror — resolve that first"
fi
step "mirror....... clean"

if [ "$CHECK_ONLY" = "1" ]; then
    say "preflight passed. Nothing was changed."
    exit 0
fi

# ------------------------------------------------------------------- the copy ---
#
# The archive has to be copied while nothing is writing to blocks.bin. The
# lock is held by the operating system, so this is a real answer rather than
# a guess about whether the wallet is running.
node_is_running() {
    [ -f "$DATADIR/.nightfall-lock" ] || return 1
    ! python3 - "$DATADIR/.nightfall-lock" <<'PY' 2>/dev/null
import fcntl, sys
try:
    fcntl.flock(open(sys.argv[1], "r+"), fcntl.LOCK_EX | fcntl.LOCK_NB)
except OSError:
    sys.exit(1)
sys.exit(0)
PY
}

started_it=0
if [ -n "$SERVICE" ] && command -v systemctl >/dev/null 2>&1; then
    if systemctl is-active --quiet "$SERVICE"; then
        say "==> stopping $SERVICE"
        systemctl stop "$SERVICE"
        started_it=1
    fi
fi

if node_is_running; then
    say "==> waiting for the wallet to close"
    step "The copy needs nothing writing to blocks.bin."
    step "Close NIGHTFALLCOIN Core now — this waits up to 5 minutes."
    for _ in $(seq 1 150); do
        node_is_running || break
        sleep 2
    done
    node_is_running && die "still running after 5 minutes. Nothing was changed."
    step "closed."
fi

WORK="$ROOT/bootstrap"
ok=0
cleanup() {
    if [ "$started_it" = "1" ]; then
        echo "==> starting $SERVICE"
        systemctl start "$SERVICE" || true
    fi
    if [ "$ok" = "1" ]; then
        rm -rf "$WORK"
    elif ls "$WORK"/nightfall-chain-*.bin >/dev/null 2>&1; then
        # Worth keeping: building it means taking the wallet down and copying
        # the whole chain, too expensive to throw away over a failed upload.
        printf '\n\033[31mRun failed. The archive is kept at %s — rerun and it\n
will be rebuilt, or upload it by hand.\033[0m\n' "$WORK" >&2
    else
        # Failed before there was anything to keep. Saying otherwise sends
        # someone looking for a file that was never written.
        rm -rf "$WORK" 2>/dev/null || true
        printf '\n\033[31mRun failed before the archive was built. Nothing was published.\033[0m\n' >&2
    fi
}
trap cleanup EXIT

rm -rf "$WORK"
"$ROOT/scripts/make-chain-bootstrap.sh" --datadir "$DATADIR" --out "$WORK"

BIN="$(ls "$WORK"/nightfall-chain-*.bin)"
MANIFEST="$(ls "$WORK"/nightfall-chain-*.json)"
SUMS="$(ls "$WORK"/SHA256SUMS-chain-*.txt)"

if [ "$started_it" = "1" ]; then
    systemctl start "$SERVICE"; started_it=0
fi
say "==> the copy is done — you can start the wallet again now"
step "Everything below runs without it."

# -------------------------------------------------------------------- publish ---
if ! gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
    gh release create "$TAG" --repo "$REPO" \
        --title "Chain bootstrap" \
        --notes "The chain as one file, rebuilt weekly. Your node still re-checks every block; see /chain/#bootstrap."
fi

# The archive goes up alone and its arrival is confirmed before anything that
# describes it follows. The first run uploaded all three at once, the 158 MB
# archive did not make it, and the release was left holding a checksum for a
# file that did not exist — a download page reading that manifest would have
# offered a dead link.
say "==> uploading the archive ($(du -h "$BIN" | cut -f1))"
gh release upload "$TAG" "$BIN" --clobber --repo "$REPO"

WANT="$(wc -c < "$BIN" | tr -d ' ')"
GOT="$(gh release view "$TAG" --repo "$REPO" --json assets \
    -q ".assets[] | select(.name == \"$(basename "$BIN")\") | \"\(.state) \(.size)\"")"
[ "$GOT" = "uploaded $WANT" ] || die "archive did not land: wanted \"uploaded $WANT\", release says \"${GOT:-nothing}\""
step "confirmed: $WANT bytes"

gh release upload "$TAG" "$MANIFEST" "$SUMS" --clobber --repo "$REPO"
step "manifest and checksums up"

# Last week's archive is dropped only after this week's is confirmed, and one
# older copy is left behind on purpose: someone may be halfway through
# downloading it, and the page has to link something real at every instant.
say "==> retiring old archives (keeping $KEEP)"
OLD="$(gh release view "$TAG" --repo "$REPO" --json assets -q '.assets[].name' \
    | grep '^nightfall-chain-.*\.bin$' | newest_first | tail -n +$((KEEP + 1)) || true)"
if [ -z "$OLD" ]; then
    step "nothing to retire"
else
    while IFS= read -r a; do
        [ -n "$a" ] || continue
        gh release delete-asset "$TAG" "$a" --yes --repo "$REPO" && step "removed $a"
    done <<< "$OLD"
fi

# ------------------------------------------------------------------- the page ---
#
# Written only now. If anything above failed the site keeps pointing at last
# week's archive, which is behind but real, rather than at one that does not
# exist.
URL="https://github.com/$REPO/releases/download/$TAG/$(basename "$BIN")"
python3 - "$MANIFEST" "$URL" > "$ROOT/$PAGE_REL" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
m["url"] = sys.argv[2]
json.dump(m, sys.stdout, indent=2)
print()
PY
cp -p "$ROOT/$PAGE_REL" "$MIRROR/$PAGE_REL"

HEIGHT="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["height"])' "$MANIFEST")"

say "==> updating the mirror"
# Only this one file. `git add -A` here would sweep in whatever else happens
# to be sitting in the mirror.
git -C "$MIRROR" add "$PAGE_REL"
if git -C "$MIRROR" diff --cached --quiet; then
    step "nothing changed — same archive as last time"
else
    git -C "$MIRROR" commit -q -m "Chain archive: height $HEIGHT

Weekly rebuild. The page reads this manifest, so the height, size and
checksum it shows come from the archive itself."
    git -C "$MIRROR" push -q origin main
    step "pushed $(git -C "$MIRROR" rev-parse --short HEAD)"
fi

if [ "$DEPLOY" = "1" ]; then
    say "==> deploying the website"
    ( cd "$MIRROR/website" && npx wrangler deploy 2>&1 | tail -3 | sed 's/^/    /' )
fi

# -------------------------------------------------------------------- verify ---
#
# Publishing and checking are not the same act. Everything above could report
# success and still leave the site serving last week's manifest from a cache,
# or a link to an asset that was pruned a moment too early. So the last step
# reads the live site the way a stranger would.
if [ "$DEPLOY" = "1" ]; then
    say "==> checking the live site"
    sleep 3
    LIVE="$(curl -fsS --max-time 30 "https://nightfallcoin.org/chain/bootstrap.json")" \
        || die "the site does not serve /chain/bootstrap.json"
    LIVE_H="$(printf '%s' "$LIVE" | python3 -c 'import json,sys; print(json.load(sys.stdin)["height"])')"
    LIVE_U="$(printf '%s' "$LIVE" | python3 -c 'import json,sys; print(json.load(sys.stdin)["url"])')"
    [ "$LIVE_H" = "$HEIGHT" ] || die "the site says height $LIVE_H, this run published $HEIGHT"
    step "manifest..... height $LIVE_H"

    CODE="$(curl -fsSL -o /dev/null -w '%{http_code}' --max-time 60 -r 0-0 "$LIVE_U" || echo 000)"
    [ "$CODE" = "206" ] || [ "$CODE" = "200" ] || die "the link on the site answers HTTP $CODE"
    step "download..... reachable"
fi

ok=1
say "==> done"
step "height $HEIGHT is live at nightfallcoin.org/chain"
