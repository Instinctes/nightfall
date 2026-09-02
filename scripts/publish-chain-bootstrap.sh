#!/usr/bin/env bash
# Weekly: rebuild the chain archive and publish it.
#
#   ./scripts/publish-chain-bootstrap.sh --service nightfall-archive --datadir DIR
#
# Meant to be run by a timer on a server that already has a synced node.
#
# It does NOT use a seed node's datadir. The archive has to be copied while
# nothing is writing to it, which means stopping whatever holds it — and a
# seed that goes down every week is a seed that is missing exactly when the
# network is smallest and needs it most. So this expects a second, ordinary
# node kept for this purpose: it syncs like any other peer, nobody depends on
# it, and stopping it for the length of a file copy costs nothing.
#
# Publishing goes to a GitHub release rather than the website. Cloudflare
# Workers serve static assets with a per-file ceiling well under the size of
# this chain, and a release is what the download page already links to for
# every binary.
set -euo pipefail

SERVICE=""
DATADIR=""
TAG="chain-bootstrap"
REPO="Instinctes/nightfall"
KEEP=2

while [ $# -gt 0 ]; do
    case "$1" in
        --service) SERVICE="$2"; shift 2 ;;
        --datadir) DATADIR="$2"; shift 2 ;;
        --tag)     TAG="$2";     shift 2 ;;
        --repo)    REPO="$2";    shift 2 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

[ -n "$DATADIR" ] || { echo "--datadir is required" >&2; exit 2; }
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

started_it=0
stop_node() {
    [ -n "$SERVICE" ] || return 0
    if systemctl is-active --quiet "$SERVICE"; then
        echo "==> stopping $SERVICE"
        systemctl stop "$SERVICE"
        started_it=1
    fi
}
start_node() {
    [ "$started_it" = "1" ] || return 0
    echo "==> starting $SERVICE"
    systemctl start "$SERVICE"
}
# Whatever happens below, the node comes back. A failed archive run must not
# leave a node down until somebody notices.
#
# The work directory is kept when the run fails. Building the archive means
# taking the node down and copying the whole chain; throwing that away because
# the upload timed out costs another outage to redo. On success there is
# nothing left worth keeping, so it goes.
ok=0
cleanup() {
    start_node
    if [ "$ok" = "1" ]; then
        rm -rf "$WORK"
    else
        echo "==> run failed; the archive is kept at $WORK" >&2
    fi
}
trap cleanup EXIT

stop_node
# The lock is released when the process dies, but systemd returns before the
# process is necessarily gone.
for _ in $(seq 1 30); do
    [ -e "$DATADIR/.nightfall-lock" ] && command -v flock >/dev/null 2>&1 || break
    flock -n "$DATADIR/.nightfall-lock" true 2>/dev/null && break
    sleep 1
done

"$ROOT/scripts/make-chain-bootstrap.sh" --datadir "$DATADIR" --out "$WORK"
start_node   # back up before the upload, which is the slow part

BIN="$(ls "$WORK"/nightfall-chain-*.bin)"
MANIFEST="$(ls "$WORK"/nightfall-chain-*.json)"
SUMS="$(ls "$WORK"/SHA256SUMS-chain-*.txt)"

# The release is a moving target on purpose: one tag, replaced weekly. A tag
# per week would leave dozens of 160 MB artefacts behind, and nobody wants
# last month's chain.
if ! gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
    gh release create "$TAG" --repo "$REPO" \
        --title "Chain bootstrap" \
        --notes "The chain as one file, rebuilt weekly. Your node still re-checks every block; see /chain/#bootstrap."
fi

# Order matters. The manifest and the checksum file describe the archive, so
# they must never reach the release without it: a download page that reads the
# manifest would then offer a link to a file that does not exist. The archive
# goes up alone, its arrival is confirmed against the size on disk, and only
# then do the two descriptions follow.
#
# This is not hypothetical. The first run uploaded all three in one command,
# the 158 MB archive did not make it, and the release sat there with a
# checksum for a missing file.
echo "==> uploading the archive ($(du -h "$BIN" | cut -f1))"
gh release upload "$TAG" "$BIN" --clobber --repo "$REPO"

WANT="$(wc -c < "$BIN" | tr -d ' ')"
GOT="$(gh release view "$TAG" --repo "$REPO" --json assets \
    -q ".assets[] | select(.name == \"$(basename "$BIN")\") | \"\(.state) \(.size)\"")"
if [ "$GOT" != "uploaded $WANT" ]; then
    echo "archive did not land: expected \"uploaded $WANT\", release says \"${GOT:-nothing}\"" >&2
    exit 1
fi

echo "==> uploading the manifest"
gh release upload "$TAG" "$MANIFEST" "$SUMS" --clobber --repo "$REPO"

# Drop archives older than the last few, or the release grows without bound.
mapfile -t OLD < <(gh release view "$TAG" --repo "$REPO" --json assets \
    -q '.assets[].name' | grep '^nightfall-chain-.*\.bin$' | sort -r | tail -n +$((KEEP + 1)))
for a in "${OLD[@]:-}"; do
    [ -n "$a" ] || continue
    echo "    removing $a"
    gh release delete-asset "$TAG" "$a" --yes --repo "$REPO" || true
done

# The page reads this to describe the archive. Written last: if anything above
# failed, the site keeps pointing at the previous archive, which is old but
# real, rather than at one that does not exist.
URL="https://github.com/$REPO/releases/download/$TAG/$(basename "$BIN")"
PAGE="$ROOT/website/public/chain/bootstrap.json"
python3 - "$MANIFEST" "$URL" > "$WORK/bootstrap.json" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
m["url"] = sys.argv[2]
json.dump(m, sys.stdout, indent=2)
PY

# Written into the repo rather than printed for someone to copy. A weekly job
# whose last step is "now paste this by hand" is a weekly job that stops
# happening in week three.
if [ -d "$(dirname "$PAGE")" ]; then
    mv "$WORK/bootstrap.json" "$PAGE"
    echo "==> wrote $PAGE"
    echo "    commit it and deploy the website."
else
    echo "==> manifest for the website:"
    cat "$WORK/bootstrap.json"
fi

ok=1
