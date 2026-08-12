# NIGHTFALLCOIN Mainnet — Operator Guide

**Network:** `mainnet`  
**Coin:** NIGHTFALLCOIN (`NIGHT`)  
**Max supply:** 90,000,000 (hard cap, no tail)  
**Premine:** 0  
**Fee:** 100% burn  
**P2P default:** `0.0.0.0:17891`  
**RPC default:** `127.0.0.1:17881` (local wallet only)  
**Protocol:** v5 (Nightproof-β) · **Wire:** v2

> ⚠ **The v4 genesis `1e2cae4e…` is dead.** v4 was consensus-broken — anyone
> could mint unlimited NIGHT. See [`AUDIT-2026-08-12.md`](./AUDIT-2026-08-12.md)
> and [`MIGRATION-v5.md`](./MIGRATION-v5.md). Run `nightfalld init` to obtain the
> v5 genesis hash; a v5 node refuses to start on a v4 datadir.

Peers with a different genesis_hash are incompatible — always use matching release builds.

---

## 1. Build

```bash
cd /path/to/0_Crypto
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

```bash
cargo build --release -p nightfall-node
./scripts/install-seed-node.sh
```

The script installs a launchd agent that starts at login and restarts on crash,
verifies the genesis hash before advertising anything, keeps the RPC on
loopback, and rotates logs so an unattended node cannot fill the disk. It does
**not** mine and holds no keys — a seed node whose operator earns rewards has an
incentive to be selective about what it relays.

Two things it cannot do for you:

- **Forward TCP 17891** to the machine, with a static local IP or a DHCP
  reservation. Without this the node dials out fine but nobody can dial in,
  which is the entire job.
- **Point the DNS name at your public address** (`curl -s https://api.ipify.org`).
  Use a dynamic DNS updater if your ISP rotates it.

Verify from a *different* network — the machine will always reach itself:

```bash
nc -vz seed.nightfallcoin.org 17891
```

A seed that does not resolve or does not answer is harmless in the sense that
nodes log the failed dial and carry on. It is not harmless in the sense that
matters: new installs then find nobody, mine alone, and lose the work when they
eventually connect. Check it periodically.

> **One seed is a single point of failure for discovery.** No seed can forge a
> block, hide one, or change the rules — every node validates independently.
> But if the only one is down, the network is unreachable to newcomers. A
> second seed run by someone else, on unrelated hardware, is worth more than
> any amount of hardening on the first.

---

## 4. Invite other miners

Recent builds connect automatically. For anything else, share:

1. Your public IP / DNS
2. Port **17891**
3. Confirm they use `--network mainnet`
4. The same release — protocol v5 / wire v2, matching `genesis_hash`

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
| Era-0 reward | 20 NIGHT / block |
| Halving | every 2,250,000 blocks |
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
- **No network-layer privacy.** No Dandelion++; the first relaying node is
  probably the origin.
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
