#!/usr/bin/env bash
#
# Install nightfalld as an always-on mainnet seed node on Linux (systemd).
#
# A seed node solves one problem: a fresh install has no way to find the
# network. Without one, a new miner silently builds a private chain and loses
# everything the moment it finally connects to someone — which has already
# happened on this network, twice.
#
# A VPS is the right home for this rather than a machine at home or in an
# office. Both of those sit behind NAT, and the failure that produced two
# irreconcilable chains was exactly that: two miners who could not reach each
# other. A public IPv4 with no NAT removes the entire class of problem.
#
# What this sets up:
#   - a dedicated unprivileged user that owns nothing else
#   - a hardened systemd unit: restarts on failure, read-only filesystem,
#     no privilege escalation, no access to anything but its own data
#   - a genesis check before the service is ever enabled
#
# The node does NOT mine and holds no keys. A seed whose operator earns block
# rewards has an incentive to be selective about what it relays; this one has
# nothing to gain from lying, which is the only reason to trust it more than
# any other stranger.
#
# Usage, as root on a fresh Debian or Ubuntu box:
#
#   curl -fsSL https://raw.githubusercontent.com/Instinctes/nightfall/main/scripts/install-seed-node-linux.sh -o install.sh
#   less install.sh          # read it. never pipe an install script into a shell
#   bash install.sh
#
# Undo:
#   systemctl disable --now nightfall-seed
#   rm /etc/systemd/system/nightfall-seed.service
#   userdel -r nightfall

set -euo pipefail

SERVICE="nightfall-seed"
NF_USER="nightfall"
NETWORK="mainnet"
P2P_PORT=17891
RPC_PORT=17881
DATADIR="/var/lib/nightfall"
BINDIR="/usr/local/bin"
REPO="https://github.com/Instinctes/nightfall"
EXPECTED_GENESIS="c8614333c0f86a4824df212474632f4b9feecf9bf0593841199d894127f2f9a6"

die() { printf '\033[31merror:\033[0m %s\n' "$1" >&2; exit 1; }
info() { printf '\033[36m==>\033[0m %s\n' "$1"; }
warn() { printf '\033[33mnote:\033[0m %s\n' "$1"; }

[ "$(id -u)" -eq 0 ] || die "run this as root"
command -v systemctl >/dev/null || die "this script targets systemd; adapt it for other init systems"

# ---------------------------------------------------------------- toolchain --

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

# -------------------------------------------------------------------- build --

# Compiling this workspace needs more memory than the cheapest VPS tiers have.
# rustc holds a whole crate's IR in memory, and curve25519-dalek, bulletproofs
# and argon2 are not small. On a 1 GB box the linker gets killed by the OOM
# reaper partway through, which surfaces as a build that stops with no error
# worth reading.
#
# Two cheap fixes rather than making people buy a larger machine for a process
# that will idle at a few dozen megabytes once it is running: temporary swap,
# and one codegen job at a time.
TOTAL_MB=$(awk '/MemTotal/ {print int($2/1024)}' /proc/meminfo)
info "memory: ${TOTAL_MB} MB"

# Named for this script rather than the conventional /swapfile, so a swapfile
# the machine already had is never touched — and so a re-run always recognises
# its own and cleans it up. The first version used /swapfile and skipped
# creation when one existed, which meant a second run left the first run's file
# behind while reporting that it had removed it.
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
# Removed again afterwards: a node at rest does not need it, and leaving a
# swapfile behind on a small disk is a surprise for whoever looks next.
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
info "building with $JOBS job(s) — several minutes on a small VPS"
( cd "$BUILD/nightfall" && cargo build --release -j "$JOBS" -p nightfall-node ) \
    || die "build failed — if it was killed without an error, it ran out of memory"

install -m 0755 "$BUILD/nightfall/target/release/nightfalld" "$BINDIR/nightfalld"
info "installed $BINDIR/nightfalld"

# --------------------------------------------------------------------- user --

if ! id -u "$NF_USER" >/dev/null 2>&1; then
    # No login shell and no home worth having: this account exists to own one
    # directory and run one process.
    # No skel: a data directory should not accumulate .bashrc and .profile for
    # an account that cannot log in.
    useradd --system --shell /usr/sbin/nologin --home-dir "$DATADIR" \
            --no-create-home "$NF_USER"
    info "created user $NF_USER"
fi
install -d -o "$NF_USER" -g "$NF_USER" -m 0750 "$DATADIR"

# ------------------------------------------------------------------ genesis --

if [ ! -e "$DATADIR/chain-meta.json" ]; then
    info "initialising $NETWORK chain"
    sudo -u "$NF_USER" "$BINDIR/nightfalld" --network "$NETWORK" --datadir "$DATADIR" init >/dev/null
fi

# `init` labels this line "genesis_hash", `status` labels it "genesis".
# Anchoring at the start of the line matches whichever is being read — the
# earlier pattern looked for "genesis_hash" in `status` output and therefore
# never matched, which surfaced as "the node did not start" when the node had
# started perfectly well.
ACTUAL="$(sudo -u "$NF_USER" "$BINDIR/nightfalld" --network "$NETWORK" --datadir "$DATADIR" status 2>/dev/null \
    | awk '/^genesis/ {print $NF; exit}')"
[ -n "$ACTUAL" ] || die "could not read the genesis hash back — the node did not start"
if [ "$ACTUAL" != "$EXPECTED_GENESIS" ]; then
    die "genesis mismatch — this node would serve a different chain
  expected: $EXPECTED_GENESIS
  got:      $ACTUAL
A seed on the wrong chain peers with nobody while appearing to work.
Delete $DATADIR and re-run, or update EXPECTED_GENESIS if the protocol moved."
fi
info "genesis verified"

# ------------------------------------------------------------------ service --

cat > "/etc/systemd/system/$SERVICE.service" <<UNIT
[Unit]
Description=NIGHTFALLCOIN seed node ($NETWORK)
Documentation=$REPO/blob/main/docs/MAINNET.md
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=$NF_USER
Group=$NF_USER

# The RPC is an administrative interface with no authentication. Binding it
# anywhere but loopback is how a node gets taken over; mine_one alone is a
# denial-of-service button. Mobile clients get a separate, narrow HTTPS
# surface in front of this — see docs/MOBILE.md.
ExecStart=$BINDIR/nightfalld \\
    --network $NETWORK \\
    --datadir $DATADIR \\
    run \\
    --listen 0.0.0.0:$P2P_PORT \\
    --rpc-listen 127.0.0.1:$RPC_PORT

Restart=always
# Do not respawn faster than this. A node that crashes on a corrupt block file
# would otherwise spin the CPU restarting forever.
RestartSec=30

# It parses untrusted bytes from strangers all day, so give it as little as it
# can work with.
NoNewPrivileges=true
PrivateTmp=true
PrivateDevices=true
ProtectSystem=strict
ProtectHome=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
ProtectClock=true
ProtectHostname=true
RestrictSUIDSGID=true
RestrictRealtime=true
RestrictNamespaces=true
LockPersonality=true
MemoryDenyWriteExecute=true
RestrictAddressFamilies=AF_INET AF_INET6
SystemCallArchitectures=native
SystemCallFilter=@system-service
SystemCallErrorNumber=EPERM
ReadWritePaths=$DATADIR

# journald handles rotation, so an unattended node cannot fill the disk.
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
UNIT

systemctl daemon-reload
systemctl enable --now "$SERVICE"
info "service enabled"

# ----------------------------------------------------------------- firewall --

if command -v ufw >/dev/null && ufw status 2>/dev/null | grep -q "Status: active"; then
    ufw allow "$P2P_PORT/tcp" >/dev/null && info "opened $P2P_PORT/tcp in ufw"
elif command -v firewall-cmd >/dev/null && firewall-cmd --state >/dev/null 2>&1; then
    firewall-cmd --permanent --add-port="$P2P_PORT/tcp" >/dev/null
    firewall-cmd --reload >/dev/null
    info "opened $P2P_PORT/tcp in firewalld"
else
    warn "no active firewall detected — make sure $P2P_PORT/tcp is reachable, \
including in your provider's own firewall, which is separate from the one on this host"
fi

sleep 5
systemctl is-active --quiet "$SERVICE" \
    || die "service did not stay up — journalctl -u $SERVICE -n 50"

PUBLIC_IP="$(curl -fsS --max-time 10 https://api.ipify.org 2>/dev/null || echo '<this-machine>')"

cat <<NEXT

  Seed node running.

  Status    systemctl status $SERVICE
  Logs      journalctl -u $SERVICE -f
  Chain     sudo -u $NF_USER $BINDIR/nightfalld --network $NETWORK --datadir $DATADIR status

  One step left, and it cannot be done from here: point the DNS name at this
  machine.

      seed.nightfallcoin.org.   A   $PUBLIC_IP

  Then verify from somewhere else — this box will always reach itself:

      nc -vz seed.nightfallcoin.org $P2P_PORT

  A seed that does not answer is not harmless. Nodes log the failed dial and
  carry on, but new installs then find nobody, mine alone, and lose the work
  when they eventually connect. Check it now and then.

NEXT
