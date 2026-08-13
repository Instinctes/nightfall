<div align="center">

# NIGHTFALLCOIN

**Money that refuses to snitch.**

A sovereign privacy Layer-1 where amounts are hidden, addresses never touch the
chain, and the total money supply is something anyone can prove.

[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-8b5cf6)](#license)
[![Protocol](https://img.shields.io/badge/protocol-v6%20Nightproof-b845d8)](docs/SPEC.md)
[![Tests](https://img.shields.io/badge/tests-134%20passing-4ae0a8)](#testing)
[![Status](https://img.shields.io/badge/status-pre--launch-ffc85c)](#honest-status)

[Website](https://nightfallcoin.org) ·
[Protocol spec](docs/SPEC.md) ·
[Security audit](docs/AUDIT-2026-08-12.md) ·
[Run a node](docs/MAINNET.md)

</div>

---

## What makes this different

Most privacy coins ask you to trust that nobody is quietly printing money
behind the confidentiality. Nightfall does not ask. Every node evaluates one
equation across the whole chain:

```
Σ UTXO − Σ kernel_excess  =  (minted − burned) · G
```

It balances only if not a single coin was ever created from nothing. Inflation
cannot hide inside it — it breaks the equation, and every node sees the break.

```bash
$ nightfalld --network mainnet status | grep supply_proof
supply_proof... OK — Σ UTXO − Σ excess == circulating·G
```

Three pieces make that sound:

| Piece | What it prevents |
|-------|------------------|
| **Bulletproof range proofs** on every output | negative amounts minting value |
| **Schnorr excess signature** over generator `H` | value created out of thin air |
| **UTXO membership + one-time key signature** | spending outputs you do not own |
| **Per-output sender signature** | a relay corrupting a payload and destroying funds |

---

## Privacy, precisely

**What is true:**

- **Amounts are Pedersen commitments**, not numbers. Nothing on chain reveals what moved.
- **No addresses on chain.** Every payment derives a fresh one-time key from an ECDH shared secret. Two payments to the same address share no visible field.
- **View keys are real.** `nfview1…` finds and decrypts everything you receive and is structurally incapable of spending — enforced by the type system, not by convention.
- **Encrypted memos**, padded to constant length so ciphertext size leaks nothing.
- **Blocks dissolve transactions.** Every payment in a block is merged into one flat, canonically sorted set of inputs, outputs and kernels. An observer cannot tell which input paid which output.
- **No admin keys.** No mint authority, no freeze, no blacklist. A fair genesis with zero allocations is enforced in code.

**What is not true yet:**

- **The transaction graph is obscured, not erased.** Cut-through would delete the per-input signature that makes non-interactive payments safe. The two are mutually exclusive; we chose usable payments. See [`docs/SPEC.md` §7](docs/SPEC.md).
- **No network-layer privacy.** No Dandelion++ yet. The node that first relays a transaction is probably its origin.
- **Not audited by a third party.** See [Honest status](#honest-status).

---

## Economics

| | |
|--|--|
| Max supply | **90,000,000 NIGHT** (curve terminates at 89,999,999.7075 — integer truncation at each halving) |
| Premine | **0** — enforced by `GenesisConfig::assert_fair()` |
| Team / VC allocation | **0** |
| Block reward | 20 NIGHT, halving every 2,250,000 blocks |
| Tail emission | **none** |
| Fees | **100% burned**, never paid to a miner |
| Block time | 15 seconds |
| Coinbase maturity | 1,440 blocks (~6 h) |

Every coin that will ever exist has to be mined.

---

## Mining

Nighthash-v2 is **Argon2id**: one hash needs 32 MiB of randomly-addressed
memory. Purpose-built hardware would need that much fast RAM per parallel core,
which is where ASIC economics stop working.

```bash
# measured on an Apple M3 Pro, 10 mining threads
devnet   1 MiB   0.83 ms/hash   22,268 H/s
testnet  8 MiB   2.48 ms/hash    2,351 H/s
mainnet 32 MiB  11.17 ms/hash      489 H/s
```

Reproduce on your own hardware:

```bash
cargo run --release -p nightfall-crypto --example powbench
```

Memory-hard proofs are symmetric — verifying costs the same as one mining
attempt. That is the price of ASIC resistance, and it is why a node records a
local validation marker rather than re-hashing its own chain on every restart.

---

## Quick start

### Wallet (recommended)

Download a build from [Releases](https://github.com/instinctes/nightfall/releases),
or compile:

```bash
cargo build --release -p nightfall-core
./target/release/nightfall-core --network mainnet
```

One app: full node, miner and wallet. No server to trust, nothing to configure.

### Command line

```bash
cargo build --release -p nightfall-node -p nightfall-wallet

# node
./target/release/nightfalld --network mainnet init
./target/release/nightfalld --network mainnet run --mine --miner-seed miner.seed

# wallet
W="./target/release/nightfall-wallet --network mainnet --seed-file alice.seed"
$W init
$W address
$W sync
$W balance
$W send --to nf1... --amount 10 --memo "invoice 42"
$W export-view-key      # watch-only: sees everything, spends nothing
$W verify-supply        # re-prove the whole money supply
```

### Devnet, for hacking

```bash
./target/release/nightfalld --network devnet run --mine \
    --listen 127.0.0.1:17893 --rpc-listen 127.0.0.1:17883
```

Low difficulty, 10-block coinbase maturity, cheap PoW parameters — the spend
path is testable in seconds.

---

## Joining the network

Builds from `main` dial `seed.nightfallcoin.org:17891` on startup and need no
configuration. From there it is automatic: each node advertises its listening
port in the handshake and peers exchange the addresses they know, so one
working connection finds the rest of the network.

**The v0.3.0 binaries predate this** and have no seed compiled in. With those,
make the first connection by hand — **Network → Add a peer** in the wallet, or:

```bash
SEED_NODE=seed.nightfallcoin.org:17891 ./nightfall-core --network mainnet
```

> **Connect before you mine.** Two miners who never meet build two separate
> chains from the same genesis. Both look valid locally. When they finally
> connect, the lighter chain is discarded and everything mined on it is gone.
> The wallet warns you when you are mining with zero peers. Reorgs deeper than
> 500 blocks are refused outright.

Running a seed node yourself is [documented in
`docs/MAINNET.md` §3](docs/MAINNET.md) — `scripts/install-seed-node.sh` does the
setup on macOS. A second seed on unrelated hardware genuinely helps: no seed can
forge or hide a block, since every node validates independently, but if the only
one is down then new installs find nobody.

---

## Architecture

```
crates/
├── nightfall-types      constants, network ids, PoW + emission parameters
├── nightfall-crypto     commitments, range proofs, Schnorr, kernels,
│                        stealth outputs, key hierarchy, Nighthash-v2
├── nightfall-ledger     transactions, UTXO set, block aggregation,
│                        the supply invariant
├── nightfall-consensus  block structure, LWMA difficulty, chain selection
├── nightfall-storage    append-only persistence, reorg-safe
├── nightfall-p2p        wire protocol, handshake, peer exchange
├── nightfall-node       node runtime, mining loop, JSON-RPC
├── nightfall-wallet     scanning, coin selection, spending (lib + CLI)
└── nightfall-core       desktop wallet (egui)
```

No `unsafe` in the consensus path. Cryptography comes from
`curve25519-dalek`, `bulletproofs`, `argon2`, `blake3` and
`chacha20poly1305` — no hand-rolled primitives.

---

## Testing

```bash
cargo test --workspace
```

134 tests. The ones that matter most:

| Suite | What it locks down |
|-------|--------------------|
| `nightfall-ledger/tests/exploit_regression.rs` | the six attacks that worked against protocol v4, each of which must now fail |
| `nightfall-ledger/tests/ledger_flow.rs` | coinbase, transfer, fee burn, double spend, maturity, atomicity, supply invariant |
| `nightfall-consensus/tests/chain_rules.rs` | work-based fork choice, time-warp resistance, reorg bounds, emission curve |
| `nightfall-storage/tests/reorg_persistence.rs` | a chain saved after a reorg must reload (found by running two miners against each other) |

**Do not weaken `exploit_regression.rs`.** Every test in it is the inverse of a
proof-of-concept that once worked.

---

## Honest status

**This is pre-launch software that has not been reviewed by anyone outside the
project.**

Protocol v4 shipped a balance proof that was a tautology: `make_balance_proof`
computed a value from public data and `verify_balance_proof` recomputed the
same value and compared it to itself. It could not fail. Anyone could have
minted unlimited NIGHT without mining, and the recipient of every payment was
published in cleartext.

We found it, wrote it up in full, threw the chain away and started over. The
whole analysis is in [`docs/AUDIT-2026-08-12.md`](docs/AUDIT-2026-08-12.md) —
including the parts that are embarrassing.

Before this carries real value it needs:

1. **An independent audit.** The v5 code was written and reviewed by the same party. That is a conflict of interest, and passing tests is not a substitute.
2. **Cut-through**, or a spend-authorisation scheme that does not require a per-input signature.
3. **Dandelion++** for network-layer privacy.
4. **A sustained multi-node testnet** across real networks, with induced reorgs, running for weeks.

If you find something wrong, see [SECURITY.md](SECURITY.md).

---

## Documentation

| Doc | |
|-----|--|
| [docs/SPEC.md](docs/SPEC.md) | Protocol v5 in full |
| [docs/AUDIT-2026-08-12.md](docs/AUDIT-2026-08-12.md) | Security audit of v4 — read this first |
| [docs/MIGRATION-v5.md](docs/MIGRATION-v5.md) | Moving off the broken chain |
| [docs/MAINNET.md](docs/MAINNET.md) | Operator guide |
| [docs/MOBILE.md](docs/MOBILE.md) | iOS and Android wallet — architecture |
| [docs/DECISIONS.md](docs/DECISIONS.md) | Every design decision and why |
| [docs/ATTRIBUTES.md](docs/ATTRIBUTES.md) | Locked product attributes, with the gaps marked |
| [MANIFESTO.md](MANIFESTO.md) | Why this exists |
| [FAIR_LAUNCH.md](FAIR_LAUNCH.md) | Fair launch rules |

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). In short: consensus changes need a test
that fails without them, and anything touching the supply invariant needs a
very good reason.

## License

MIT **or** Apache-2.0, at your option.
See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).

No warranty of any kind. This is experimental money.

---

<div align="center">
<sub><i>Born when the lights go out.</i></sub>
</div>
