# NIGHTFALLCOIN Mainnet — Operator Guide

**Network:** `mainnet`  
**Coin:** NIGHTFALLCOIN (`NIGHT`)  
**Max supply:** 90,000,000 (hard cap, no tail)  
**Premine:** 0  
**Fee:** 100% burn  
**P2P default:** `0.0.0.0:17891`  
**RPC default:** `127.0.0.1:17881` (local wallet only)  
**Protocol:** v7 (Nightproof) · **Wire:** v4
**Genesis:** `c8614333c0f86a4824df212474632f4b9feecf9bf0593841199d894127f2f9a6`

> ⚠ **Every chain before v7 is dead.** v4 was consensus-broken — anyone could
> mint unlimited NIGHT, see [`AUDIT-2026-08-12.md`](./AUDIT-2026-08-12.md). v5
> was sound; v6 added view tags to the output format.
>
> v7 changes no format. The v6 chain was abandoned because two networking
> faults made it untrustworthy rather than invalid: nodes stopped mining while
> waiting for each other across a fork, and wallets never un-did what a reorg
> had undone, so a confirmed payment could disappear with nothing reporting it.
> The result was a chain with a transaction on one branch and not the other and
> no way to say which was true. The genesis differs, so nothing can mix. Delete
> any pre-v7 datadir.
>
> Coins mined on a pre-v7 chain do not carry over. There is no migration and
> there was never a claim there would be — see [`FAIR_LAUNCH.md`](../FAIR_LAUNCH.md).

Peers with a different genesis_hash are incompatible — always use matching release builds.

---

## 1. Build

```bash
cd nightfall
cargo build --release -p nightfall-node -p nightfall-wallet
```

Binaries:

- `target/release/nightfalld`
- `target/release/nightfall-wallet`

---

## 2. First node (genesis miner)

You are **Node Zero**. Fair launch: empty chain, first block height 0 is mined by whoever starts first.

```bash
# optional dedicated datadir
export NF_DATA="$HOME/nightfall-mainnet"

./target/release/nightfalld --network mainnet --datadir "$NF_DATA" init

# Start P2P + mine continuously
./target/release/nightfalld --network mainnet --datadir "$NF_DATA" run \
  --listen 0.0.0.0:17891 \
  --rpc-listen 127.0.0.1:17881 \
  --mine \
  --miner-seed miner.seed
```

**Open firewall TCP 17891** so others can connect.

Note `genesis_hash` from logs/`init` — all peers must match.

Miner address is derived from `miner.seed` in the datadir. Back it up offline.

---

## 3. Running the seed node

Every release ships with `seed.nightfallcoin.org:17891` compiled in, so a fresh
install finds the network without being told anything. If you are the one
operating that machine, this section is for you.

**On a Linux VPS — the recommended home:**

```bash
curl -fsSL https://raw.githubusercontent.com/Instinctes/nightfall/main/scripts/install-seed-node-linux.sh -o install.sh
less install.sh          # read it; never pipe an installer into a shell
sudo bash install.sh
```

**On macOS:**

```bash
cargo build --release -p nightfall-node
./scripts/install-seed-node.sh
```

Both create a service that restarts on crash, verify the genesis hash before
starting, keep the RPC on loopback, and hand logging to the system so an
unattended node cannot fill a disk. Neither mines or holds keys — a seed whose
operator earns block rewards has an incentive to be selective about what it
relays; one with nothing to gain is the only kind worth trusting more than any
other stranger.

The Linux unit additionally runs as a dedicated unprivileged user under a
strict systemd sandbox: read-only filesystem outside its own data directory, no
privilege escalation, no device access, a syscall filter. It parses untrusted
bytes from strangers continuously, so it gets as little as it can work with.

**Prefer a VPS over a machine at home or in an office.** Both of those sit
behind NAT, and NAT is what produced two irreconcilable chains on this network:
two miners who simply could not reach each other. A public IPv4 removes the
entire class of problem, and the smallest tier any provider sells is enough.

Two things no script can do for you:

- **Make TCP 17891 reachable.** On a VPS that means the provider's own
  firewall, which is separate from the one on the host. Behind NAT it means a
  port forward plus a static local IP or DHCP reservation. Without this the
  node dials out fine but nobody can dial in,
  which is the entire job.
- **Point the DNS name at your public address** (`curl -s https://api.ipify.org`).

On a home or office line the address changes without warning, and when it does
the name stops resolving to anything useful. That is harmless for nodes already
connected and quietly bad for new ones: they find one seed instead of two, and
nothing says so. `scripts/cloudflare-ddns.sh` keeps a Cloudflare A record
pointed at the machine it runs on:

```bash
./scripts/cloudflare-ddns.sh --setup      # how to create a scoped API token
./scripts/cloudflare-ddns.sh --status     # what it would change, changes nothing
./scripts/cloudflare-ddns.sh --install    # launchd agent, checks every 5 minutes
```

The token needs exactly `Zone · DNS · Edit` on this one zone — a machine in a
cupboard should hold the least authority that still does the job. It is read
from a file the script never prints, and the script forces `proxied: false` on
every write, because Cloudflare's proxy carries only HTTP: with it on, the name
resolves to Cloudflare and nobody reaches port 17891, while the record looks
perfectly healthy.

Verify from a *different* network — the machine will always reach itself:

```bash
nc -vz seed.nightfallcoin.org 17891
```

A seed that does not resolve or does not answer is harmless in the sense that
nodes log the failed dial and carry on. It is not harmless in the sense that
matters: new installs then find nobody, mine alone, and lose the work when they
eventually connect. Check it periodically.

> **Seeds are a single point of failure for discovery only.** No seed can forge
> a block, hide one, or change the rules — every node validates independently.
> But if none are reachable, the network is invisible to newcomers.
>
> Two names are compiled in: `seed.nightfallcoin.org` (live, Vultr Frankfurt)
> and `seed2.nightfallcoin.org`. seed2 is reserved for a **second machine on a
> second network** — not a second process on the first seed, and not a miner
> behind NAT. A name that does not resolve is cheaper than a name that points
> at the same host and pretends to be redundant. Bring a second VPS up with
> `scripts/install-seed-node-linux.sh`, point the A record (unproxied) at it,
> and the compiled name starts working without a new release. **A seed run by
> somebody else is worth more than a third run by us.**

---

## 4. Invite other miners

Recent builds connect automatically. For anything else, share:

1. Your public IP / DNS
2. Port **17891**
3. Confirm they use `--network mainnet`
4. The same release — protocol v8 / wire v6, genesis `061a052d…`
5. Phones: `--mobile-listen 0.0.0.0:17888` (light API only)
6. Browser wallet: Cloudflare can only `fetch()` ports 80/443, so the
   seed also forwards TCP 80 → 17888 (`nightfall-mobile-http.service`).
   `POST https://nightfallcoin.org/wallet-api` is the same allowlist.

### Joiner command

```bash
export NF_DATA="$HOME/nightfall-mainnet"
export SEED_NODE="YOUR.IP.OR.HOST:17891"

./target/release/nightfalld --network mainnet --datadir "$NF_DATA" init

./target/release/nightfalld --network mainnet --datadir "$NF_DATA" run \
  --listen 0.0.0.0:17891 \
  --rpc-listen 127.0.0.1:17881 \
  --connect "$SEED_NODE" \
  --mine \
  --miner-seed miner.seed
```

Multiple `--connect host:port` allowed.

Node will:

1. Handshake (network + genesis check)  
2. Pull chain / reorg to **longest valid chain**  
3. Mine on the shared tip  
4. Broadcast new blocks  

---

## 5. Wallet (against local RPC)

```bash
export NF_DATA="$HOME/nightfall-mainnet"
export RPC=127.0.0.1:17881
W="./target/release/nightfall-wallet --network mainnet --datadir $NF_DATA --rpc $RPC"

# create a wallet
$W --seed-file alice.seed init

# your receive address (nf1… with checksum — share this)
$W --seed-file alice.seed address

# if you mine, scan your rewards
$W --seed-file miner.seed sync
$W --seed-file miner.seed balance

# pay someone
$W --seed-file miner.seed send --to nf1... --amount 10 --memo "invoice 42"

# recipient
$W --seed-file alice.seed sync
$W --seed-file alice.seed balance
$W --seed-file alice.seed outputs

# watch-only credential: sees amounts and memos, cannot spend
$W --seed-file alice.seed export-view-key

# verify the network's total supply cryptographically
$W --seed-file alice.seed verify-supply
```

Coinbase outputs mature after **1,440 blocks** (~6 h) before they can be spent.

A transaction sits in the mempool until a block is mined.

## 6. Check status

```bash
echo '{"method":"status","params":{},"id":1}' | nc 127.0.0.1 17881
```

Or:

```bash
./target/release/nightfall-wallet --network mainnet --rpc 127.0.0.1:17881 node-status
```

---

## 7. Economic parameters (locked)

| Parameter | Value |
|-----------|--------|
| Max supply | 90,000,000 NIGHT |
| Era-0 reward | 6 NIGHT / block |
| Halving | every 7,500,000 blocks (~3.56 years) |
| Block time target | 15s |
| Difficulty | LWMA-1, retargeted every block, 90-block window, floor 2,000 |
| PoW | Nighthash-v2 (Argon2id), 32 MiB / hash |
| Coinbase maturity | 1,440 blocks |
| Premine | 0 |
| Tail emission | none |
| Fee | 100% burn |

---

## 8. Security notes (honest)

- **Amounts are confidential** (Pedersen + Bulletproofs) and **no recipient
  appears on chain** (one-time output keys). The **transaction graph is
  obscured but not erased**: blocks merge every transaction into one sorted set,
  so nobody can tell which input paid which output within a block, but spent
  outputs remain visible. Cut-through would delete the per-input signature that
  makes non-interactive payments safe; the two are mutually exclusive.
- **Network-layer privacy is stem/fluff.** A locally submitted transaction
  is sent to one random peer first. Relays stay in the stem with 90 %
  probability and fluff after 12–28 s. The wire message is still `Tx`, so
  0.6.3 peers accept it — they just broadcast immediately. Optional Tor:
  `--proxy 127.0.0.1:9050` or `NIGHTFALL_PROXY`. See [PRIVACY.md](PRIVACY.md).
- **PoW is Argon2id** (Nighthash-v2) at 32 MiB per hash on mainnet. Verification
  costs the same as one mining attempt — that is the price of ASIC resistance.
- The RPC is **unauthenticated**. Binding it to a non-loopback address is
  refused unless you set `NF_ALLOW_PUBLIC_RPC=1`; only do that behind an
  authenticating reverse proxy.
- Back up all `*.seed` files — loss = permanent loss of funds. They are written
  mode `0600`; keep it that way.
- No admin keys, no freeze, no premine.
- **Not audited by a third party.** Do not put value on this network yet.

### Verify the supply yourself

```bash
./target/release/nightfalld --network mainnet status | grep supply_proof
# supply_proof... OK — Σ UTXO − Σ excess == circulating·G

# outbound through Tor (optional)
./target/release/nightfalld --network mainnet run --proxy 127.0.0.1:9050
```

If that ever reads `FAILED`, stop the node and do not relay.

---

## 9. Devnet dry-run (before mainnet)

```bash
./target/release/nightfalld --network devnet run --mine --listen 127.0.0.1:17893 --rpc-listen 127.0.0.1:17883
```

Devnet is easier PoW and 1s mine pacing for testing.

---

**NIGHTFALLCOIN** — first node starts the night. Everyone else joins with the same genesis rules.
