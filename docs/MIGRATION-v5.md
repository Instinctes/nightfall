# Migration from protocol v4 to v5

**Status:** required. v4 is consensus-broken — see [`AUDIT-2026-08-12.md`](./AUDIT-2026-08-12.md).

---

## 1. Why the v4 chain cannot be carried forward

v4's balance proof was a tautology. Any participant could mint arbitrary NIGHT at any time, and the ledger's supply counter would not even register it. This means:

**The recorded supply of a v4 chain is not evidence of anything.** Not because we know it was attacked, but because the chain provides no way to prove it was not. There is no invariant to check, no proof to re-verify. "84 blocks × 20 NIGHT = 1,680 NIGHT" is an assumption, not a fact the chain can substantiate.

v5 makes this checkable forever:

```
Σ UTXO − Σ kernel_excess = (minted − burned) · G
```

That equation cannot be satisfied by coins that were never minted. It is verified after every block, on startup, and on demand via `nightfall-wallet verify-supply`.

The formats are also fundamentally incompatible — different transaction structure, different key curve, different address format, different block header, different genesis commitment. v5 nodes will not peer with v4 nodes (`WIRE_VERSION` 1 → 2, `PROTOCOL_VERSION` 4 → 5), and the two chains cannot mix.

---

## 2. State of the live chain at audit time

Read directly from the mainnet datadir on 2026-08-12:

| | |
|--|--|
| Blocks | 84 |
| Minted | 1,680 NIGHT |
| Non-coinbase transactions | 1 |
| Distinct recipient keys | 2 |
| Evidence of exploitation | **None found** |

Two recipient keys and a single transfer means the chain never left the operator's own machine. There are no third-party miners with balances to protect.

---

## 3. The migration options, and the tension in each

### Option A — Clean relaunch (recommended)

Delete the v4 chain, start v5 from an empty genesis, mine from zero.

- Preserves the `Premine = 0` lock in `ATTRIBUTES.md` exactly.
- Preserves the fair-launch claim without an asterisk.
- Costs 1,680 NIGHT of self-mined balance on a chain that had no external participants.

### Option B — Credit existing balances into the v5 genesis

**This is a premine.** `ATTRIBUTES.md` locks:

> | Premine | **0** | Fair launch |
>
> **Locked supply invariant:** `total_minted ≤ 90_000_000 NIGHT` forever. No tail. No emergency mint.

and its change-control table classifies premine as a **hard** change requiring "explicit new lock + hard-fork social consensus".

Crediting balances derived from a chain where anyone could mint means the v5 genesis would contain allocations whose provenance cannot be proven. It also requires deleting `GenesisConfig::assert_fair()`, which is the code that enforces the fair-launch promise. Every future participant would have to take the founder's word for the opening balance.

For 1,680 NIGHT on a single-operator chain, the reputational cost is far larger than the amount.

### Option C — Relaunch, keep v4 as a testnet artifact

Same as A, but archive the v4 datadir and re-label that chain `testnet`. It becomes documented history rather than something quietly deleted. This is the honest framing if anyone ever asks what the first 84 blocks were.

**Recommendation: A, presented as C.** Relaunch clean; keep and publish the v4 chain as a labelled pre-launch artifact alongside the audit.

---

## 4. Performing the migration

### 4.1 Archive the old chain

```bash
NF_OLD="$HOME/Library/Application Support/nightfall/mainnet"
mkdir -p "$HOME/nightfall-v4-archive"
cp -a "$NF_OLD" "$HOME/nightfall-v4-archive/mainnet-v4-$(date +%Y%m%d)"
```

Keep it. It is the evidence trail for the audit.

### 4.2 Back up your seeds — separately

```bash
cp "$NF_OLD/core.seed"   "$HOME/nightfall-v4-archive/core.seed.v4"
chmod 600 "$HOME/nightfall-v4-archive/"*.seed*
```

> The v4 seed derives v4 keys (ed25519 spend + x25519 encrypt). v5 uses a
> different hierarchy (Ristretto scan + spend). **The same 32-byte seed produces
> a different address in v5.** The seed is still worth keeping, but do not
> expect the old address to reappear.

### 4.3 Clear the v4 chain data

A v5 node **refuses to start** on a datadir containing `chain.json` and tells you why. Remove or move it:

```bash
mv "$NF_OLD/chain.json" "$HOME/nightfall-v4-archive/chain.json.v4"
rm -f "$NF_OLD/core.notes.json"     # v4 note format, unreadable by v5
```

### 4.4 Rebuild and restart

```bash
cd /path/to/0_Crypto
cargo build --release -p nightfall-node -p nightfall-wallet -p nightfall-core

./target/release/nightfalld --network mainnet init
./target/release/nightfalld --network mainnet run \
    --listen 0.0.0.0:17891 \
    --rpc-listen 127.0.0.1:17881 \
    --mine --miner-seed miner.seed
```

Note the new `genesis_hash` from the output. **It differs from v4's**
`1e2cae4e…` — that is the intended, load-bearing incompatibility.

### 4.5 Verify

```bash
./target/release/nightfalld --network mainnet status
```

Look for:

```
supply_proof... OK — Σ UTXO − Σ excess == circulating·G
```

If that line ever says `FAILED`, stop the node and do not relay. It means the chain state is inconsistent, which under v5 should be impossible.

---

## 5. Before inviting anyone else to mine

The audit closed the breaks that made v4 worthless. It did not turn the project into something that should hold other people's value yet. Outstanding items, in the order they matter:

1. **Independent third-party review.** The replacement code was written by the same party that audited the original. That is a conflict of interest and no substitute for outside eyes.
2. **Reconcile `ATTRIBUTES.md` with what Mimblewimble actually provides.** The document locks a "protocol-scale anonymity set". Mimblewimble hides amounts and eliminates addresses, but the transaction graph remains linkable without kernel aggregation and cut-through — which are not implemented. Amend the claim or build the feature; do not ship the gap.
3. **Nighthash-v2.** `DECISIONS.md` promised Argon2id before production economic security. PoW is still Blake3, which is ASIC-friendly and offers no CPU fairness.
4. **Sustained multi-node testnet.** Run three or more independent nodes across real networks for weeks, with induced reorgs, before mainnet carries value.
5. **Correct the supply figure in public materials** — the emission curve terminates at 89,999,999.7075 NIGHT, not 90,000,000.

---

## 6. Address and format changes

| | v4 | v5 |
|---|---|---|
| Spend curve | ed25519 | Ristretto |
| Encrypt curve | x25519 | Ristretto (same key hierarchy) |
| Address | `nf1` + 32-byte hash, no checksum | `nf1` + scan_pk ‖ spend_pk ‖ 4-byte checksum |
| Payment identifier | 128 hex chars, pasted raw | the address itself |
| View key | none | `nfview1…`, detects and decrypts, cannot spend |
| On-chain recipient | `recipient_spend_pk` in cleartext | none — fresh one-time key per output |
| Amount proof | none | Bulletproof, `[0, 2^64)` |
| Balance proof | tautology | Schnorr excess signature over `H` |
| Chain file | `chain.json` (full rewrite) | `blocks.jsonl` (append-only) |
| Wallet notes | `core.notes.json` | `core.seed.outputs.json` |

Old wallet files are not migrated. They describe a note format that no longer exists.
