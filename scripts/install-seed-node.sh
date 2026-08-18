#!/usr/bin/env bash
#
# Install nightfalld as an always-on mainnet seed node on macOS.
#
# A seed node solves exactly one problem: a fresh install has no way to find
# the network. Without one, every new miner silently builds a private fork and
# loses everything the moment they finally connect to someone.
#
# What this sets up:
#   - a launchd agent that starts at login and restarts on crash
#   - a dedicated data directory, separate from any wallet on this machine
#   - log rotation, because an unattended node writing forever will fill a disk
#
# The node does NOT mine and holds no keys. A seed node that mines is a seed
# node whose operator has an incentive to be dishonest about what it relays.
#
# Usage:
#   ./scripts/install-seed-node.sh /path/to/nightfalld
#   ./scripts/install-seed-node.sh              # uses ./target/release/nightfalld
#
# Undo:
#   launchctl bootout gui/$(id -u)/org.nightfallcoin.seed
#   rm ~/Library/LaunchAgents/org.nightfallcoin.seed.plist

set -euo pipefail

LABEL="org.nightfallcoin.seed"
NETWORK="mainnet"
P2P_PORT=17891
RPC_PORT=17881

BIN="${1:-$(cd "$(dirname "$0")/.." && pwd)/target/release/nightfalld}"
DATADIR="$HOME/Library/Application Support/NightfallSeed"
LOGDIR="$HOME/Library/Logs/NightfallSeed"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"

die() { printf '\033[31merror:\033[0m %s\n' "$1" >&2; exit 1; }
info() { printf '\033[36m==>\033[0m %s\n' "$1"; }

[ -x "$BIN" ] || die "nightfalld not found or not executable: $BIN
Build it first:  cargo build --release -p nightfall-node"

# Resolve to an absolute path — launchd has no working directory to speak of.
BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"

info "binary:  $BIN"
info "datadir: $DATADIR"

mkdir -p "$DATADIR" "$LOGDIR" "$(dirname "$PLIST")"

# Initialise the chain if this is a first run. Idempotent.
if [ ! -e "$DATADIR/chain.meta" ] && [ ! -e "$DATADIR/blocks.dat" ]; then
    info "initialising $NETWORK chain"
    "$BIN" --network "$NETWORK" --datadir "$DATADIR" init
fi

# Verify the genesis matches the published one before advertising this node to
# the world. A seed serving a different genesis peers with nobody and is worse
# than no seed at all, because it looks like it is working.
EXPECTED_GENESIS="061a052d49607ff8f4b306c75d622ebd230cff4ec3a45a6dffc2f7738d4b20de"
# `init` labels this line "genesis_hash", `status` labels it "genesis". Matching
# on the start of the line covers both; the previous pattern looked for
# "genesis_hash" in `status` output, never matched, and left ACTUAL empty —
# which the check below then skipped over. A verification that quietly does
# nothing is worse than none, because it reports success.
ACTUAL="$("$BIN" --network "$NETWORK" --datadir "$DATADIR" status 2>/dev/null \
    | awk '/^genesis/ {print $NF; exit}')"
[ -n "$ACTUAL" ] || die "could not read the genesis hash back — the node did not start"
if [ "$ACTUAL" != "$EXPECTED_GENESIS" ]; then
    die "genesis mismatch — this node is on a different chain
  expected: $EXPECTED_GENESIS
  got:      $ACTUAL
Delete $DATADIR and re-run to start clean."
fi

info "genesis verified"

cat > "$PLIST" <<PLIST_EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>$LABEL</string>

    <key>ProgramArguments</key>
    <array>
        <string>$BIN</string>
        <string>--network</string>
        <string>$NETWORK</string>
        <string>--datadir</string>
        <string>$DATADIR</string>
        <string>run</string>
        <string>--listen</string>
        <string>0.0.0.0:$P2P_PORT</string>
        <!-- RPC stays on loopback. It is an administrative interface with no
             authentication; exposing it is how a node gets taken over. -->
        <string>--rpc-listen</string>
        <string>127.0.0.1:$RPC_PORT</string>
    </array>

    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <!-- Do not respawn faster than this. A node that crashes on a corrupt
         block file would otherwise spin the CPU restarting forever. -->
    <key>ThrottleInterval</key>
    <integer>30</integer>

    <key>StandardOutPath</key>
    <string>$LOGDIR/seed.log</string>
    <key>StandardErrorPath</key>
    <string>$LOGDIR/seed.err.log</string>

    <key>ProcessType</key>
    <string>Background</string>
    <key>LowPriorityIO</key>
    <true/>
</dict>
</plist>
PLIST_EOF

info "wrote $PLIST"

# Rotate logs, so an unattended node cannot fill the disk.
sudo tee "/etc/newsyslog.d/nightfall-seed.conf" >/dev/null <<ROTATE_EOF || \
    info "skipped log rotation (needs sudo) — set it up later"
# logfilename                         [owner:group]  mode count size when  flags
$LOGDIR/seed.log                       $USER:staff    644  7     10240 *     GJ
$LOGDIR/seed.err.log                   $USER:staff    644  7     10240 *     GJ
ROTATE_EOF

launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
launchctl bootstrap "gui/$(id -u)" "$PLIST"
launchctl enable "gui/$(id -u)/$LABEL"

sleep 3

if launchctl print "gui/$(id -u)/$LABEL" >/dev/null 2>&1; then
    info "running"
else
    die "launchd did not start it — check $LOGDIR/seed.err.log"
fi

cat <<NEXT

  Seed node installed.

  Logs      tail -f "$LOGDIR/seed.log"
  Status    "$BIN" --network $NETWORK --datadir "$DATADIR" status
  Stop      launchctl bootout gui/$(id -u)/$LABEL

  Two things this script cannot do for you:

    1. Forward TCP port $P2P_PORT to this machine in your router, and give it a
       static local IP or a DHCP reservation. Without this the node can dial
       out but nobody can dial in — which is the entire job of a seed.

    2. Point seed.nightfallcoin.org at your public address. Check it with:

           curl -s https://api.ipify.org

       If your ISP changes that address periodically, use a dynamic DNS
       updater rather than a fixed A record.

  Verify from a different network — not from this machine, which will always
  reach itself:

      nc -vz seed.nightfallcoin.org $P2P_PORT

NEXT
