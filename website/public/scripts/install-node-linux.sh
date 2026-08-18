#!/usr/bin/env bash
#
# Install nightfalld as an always-on mainnet node on Linux (systemd).
#
# This is the headless node, not the seed. It syncs, can listen, and can
# mine if you pass --mine. A seed (no keys, public doorbell) is
# scripts/install-seed-node-linux.sh.
#
# Usage, as root on Debian or Ubuntu:
#
#   curl -fsSL https://nightfallcoin.org/scripts/install-node-linux.sh -o install-node.sh
#   less install-node.sh          # read it. never pipe an installer into a shell
#   bash install-node.sh
#   bash install-node.sh --mine   # also mines; holds a key
#
# Same file on GitHub:
#   https://raw.githubusercontent.com/Instinctes/nightfall/main/scripts/install-node-linux.sh
#
# Undo:
#   systemctl disable --now nightfall-node
#   rm /etc/systemd/system/nightfall-node.service

set -euo pipefail

SERVICE="nightfall-node"
NF_USER="nightfall"
NETWORK="mainnet"
P2P_PORT=17891
RPC_PORT=17881
DATADIR="/var/lib/nightfall"
BINDIR="/usr/local/bin"
REPO="https://github.com/Instinctes/nightfall"
EXPECTED_GENESIS="061a052d49607ff8f4b306c75d622ebd230cff4ec3a45a6dffc2f7738d4b20de"
MINE=0

die() { printf '\033[31merror:\033[0m %s\n' "$1" >&2; exit 1; }
info() { printf '\033[36m==>\033[0m %s\n' "$1"; }
warn() { printf '\033[33mnote:\033[0m %s\n' "$1"; }

for arg in "$@"; do
    case "$arg" in
        --mine) MINE=1 ;;
        -h|--help)
            sed -n '2,20p' "$0"
            exit 0
            ;;
        *) die "unknown argument: $arg" ;;
    esac
done

[ "$(id -u)" -eq 0 ] || die "run this as root"
command -v systemctl >/dev/null || die "this script targets systemd"

if ! command -v cargo >/dev/null; then
    info "installing build dependencies"
    if command -v apt-get >/dev/null; then
        export DEBIAN_FRONTEND=noninteractive
        apt-get update -qq
        apt-get install -y -qq build-essential pkg-config libssl-dev git curl ca-certificates
    elif command -v dnf >/dev/null; then
        dnf install -y -q gcc gcc-c++ make pkgconf-pkg-config openssl-devel git curl
    else
        die "unsupported package manager — install a C toolchain, git and rustup by hand"
    fi
    info "installing rust"
    curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y --no-modify-path
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
fi
command -v cargo >/dev/null || die "cargo still not on PATH"

TOTAL_MB=$(awk '/MemTotal/ {print int($2/1024)}' /proc/meminfo)
info "memory: ${TOTAL_MB} MB"
SWAPFILE=""
SWAP_PATH=/var/tmp/nightfall-build.swap
if [ "$TOTAL_MB" -lt 2048 ]; then
    swapoff "$SWAP_PATH" 2>/dev/null || true
    rm -f "$SWAP_PATH"
    info "adding 2 GB of temporary swap for the build"
    if fallocate -l 2G "$SWAP_PATH" 2>/dev/null \
        || dd if=/dev/zero of="$SWAP_PATH" bs=1M count=2048 status=none; then
        chmod 600 "$SWAP_PATH"
        mkswap "$SWAP_PATH" >/dev/null 2>&1
        if swapon "$SWAP_PATH" 2>/dev/null; then
            SWAPFILE="$SWAP_PATH"
        else
            rm -f "$SWAP_PATH"
            warn "could not enable swap — the build may run out of memory"
        fi
    fi
fi
cleanup() {
    if [ -n "$SWAPFILE" ]; then
        swapoff "$SWAPFILE" 2>/dev/null || true
        rm -f "$SWAPFILE"
    fi
    rm -rf "${BUILD:-}"
}
trap cleanup EXIT

BUILD="$(mktemp -d)"
info "fetching source"
git clone --depth 1 "$REPO" "$BUILD/nightfall" >/dev/null 2>&1 \
    || die "clone failed — check network access to github.com"
JOBS=1
[ "$TOTAL_MB" -ge 4096 ] && JOBS=$(nproc)
info "building with $JOBS job(s)"
( cd "$BUILD/nightfall" && cargo build --release -j "$JOBS" -p nightfall-node ) \
    || die "build failed — if it was killed without an error, it ran out of memory"
install -m 0755 "$BUILD/nightfall/target/release/nightfalld" "$BINDIR/nightfalld"
info "installed $BINDIR/nightfalld"

if ! id -u "$NF_USER" >/dev/null 2>&1; then
    useradd --system --shell /usr/sbin/nologin --home-dir "$DATADIR" \
            --no-create-home "$NF_USER"
    info "created user $NF_USER"
fi
install -d -o "$NF_USER" -g "$NF_USER" -m 0750 "$DATADIR"

if [ ! -e "$DATADIR/chain-meta.json" ]; then
    info "initialising $NETWORK chain"
    sudo -u "$NF_USER" "$BINDIR/nightfalld" --network "$NETWORK" --datadir "$DATADIR" init >/dev/null
fi

ACTUAL="$(sudo -u "$NF_USER" "$BINDIR/nightfalld" --network "$NETWORK" --datadir "$DATADIR" status 2>/dev/null \
    | awk '/^genesis/ {print $NF; exit}')"
[ -n "$ACTUAL" ] || die "could not read the genesis hash back — the node did not start"
if [ "$ACTUAL" != "$EXPECTED_GENESIS" ]; then
    die "genesis mismatch — this node would sit on a different chain
  expected: $EXPECTED_GENESIS
  got:      $ACTUAL
Delete $DATADIR and re-run, or update EXPECTED_GENESIS if the protocol moved."
fi
info "genesis verified"

if [ "$MINE" -eq 1 ]; then
    warn "mining is on. This process now holds a key. A seed should not do that."
    EXEC_EXTRA=" \\
    --mine --miner-seed miner.seed"
else
    EXEC_EXTRA=""
fi

cat > "/etc/systemd/system/$SERVICE.service" <<UNIT
[Unit]
Description=NIGHTFALLCOIN node ($NETWORK)
Documentation=$REPO/blob/main/docs/MAINNET.md
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=$NF_USER
Group=$NF_USER
ExecStart=$BINDIR/nightfalld \\
    --network $NETWORK \\
    --datadir $DATADIR \\
    run \\
    --listen 0.0.0.0:$P2P_PORT \\
    --rpc-listen 127.0.0.1:$RPC_PORT$EXEC_EXTRA
Restart=always
RestartSec=30
NoNewPrivileges=true
PrivateTmp=true
PrivateDevices=true
ProtectSystem=strict
ProtectHome=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictSUIDSGID=true
RestrictRealtime=true
RestrictNamespaces=true
LockPersonality=true
MemoryDenyWriteExecute=true
RestrictAddressFamilies=AF_INET AF_INET6
ReadWritePaths=$DATADIR
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
UNIT

systemctl daemon-reload
systemctl enable --now "$SERVICE"
info "service enabled"

if command -v ufw >/dev/null && ufw status 2>/dev/null | grep -q "Status: active"; then
    ufw allow "$P2P_PORT/tcp" >/dev/null && info "opened $P2P_PORT/tcp in ufw"
fi

sleep 5
systemctl is-active --quiet "$SERVICE" \
    || die "service did not stay up — journalctl -u $SERVICE -n 50"

cat <<NEXT

  Node running.

  Status    systemctl status $SERVICE
  Logs      journalctl -u $SERVICE -f
  Chain     sudo -u $NF_USER $BINDIR/nightfalld --network $NETWORK --datadir $DATADIR status

  Outbound to a seed is enough to stay on the tip. Forward TCP $P2P_PORT
  only if you want others to dial you. RPC stays on 127.0.0.1:$RPC_PORT.

  Snapshot (optional, still verifies PoW on import):

      nightfalld --network $NETWORK --datadir $DATADIR export-snapshot --out /tmp/nf-snap
      nightfalld --network $NETWORK --datadir /var/lib/nightfall-new import-snapshot --from /tmp/nf-snap

NEXT
