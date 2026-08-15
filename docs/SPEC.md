# Nightfall L1 — Protocol Spec (Nightproof)

**Native asset:** NIGHTFALLCOIN (`NIGHT`)
**Protocol version:** 8 · **Wire version:** 6 · **Magic:** `NFL2`
**Genesis:** `061a052d49607ff8f4b306c75d622ebd230cff4ec3a45a6dffc2f7738d4b20de`
**History:** [HISTORY.md](HISTORY.md) · [RESET.md](RESET.md)
**v4 was consensus-broken** — see [`AUDIT-2026-08-12.md`](./AUDIT-2026-08-12.md).

---

## 0. Invariants

1. Max supply **90,000,000 NIGHT**, no tail. (Emission terminates at 89,999,999.25 — §3.)
2. Premine **0**.
3. Confidential amounts, no addresses on chain.
4. No admin / freeze / mint keys.
5. Fees **burned while a subsidy exists**; after the last subsidy they **pay the miner**.
6. No bridges, no transparent unshield.
7. **`Σ UTXO − Σ kernel_excess = (minted − burned)·G`** holds at every height.

---

## 1. Cryptographic suite

| Piece | Choice |
|-------|--------|
| Curve | **Ristretto** (curve25519) throughout — one curve, one encoding |
| Hash | **Blake3**, domain-separated and length-prefixed |
| PoW | **Nighthash-v2** = `Argon2id(header_preimage ‖ nonce_le, salt, m, t, p)` — memory-hard |
| Commitments | **Pedersen** `C = v·G + b·H`, generators from `bulletproofs::PedersenGens` |
| Range proofs | **Bulletproofs**, 64-bit, one per output |
| Signatures | **Schnorr** on Ristretto, generator-parameterised |
| Payload encryption | **XChaCha20-Poly1305**, key from ECDH shared secret |
| Address | `nf1` ‖ hex(scan_pk ‖ spend_pk) ‖ hex(checksum[..4]) |

`H = hash_to_group(G)` is a NUMS point. The whole soundness argument rests on
nobody knowing `x` with `H = x·G`.

### 1.1 Key hierarchy

```
seed ─┬─> scan_sk  (a)   A = a·G   detect + decrypt
      └─> spend_sk (b)   B = b·G   authorise spending
```

- **Address** = `(A, B)`.
- **View key** = `(a, B)` — finds and opens every output, cannot sign.

### 1.2 Output (one-sided stealth, MWEB-style)

Sender draws ephemeral `r`, publishes `Ke = r·G`:

```
t   = H("shared" ‖ r·A)        receiver recomputes as H("shared" ‖ a·Ke)
b   = H("blind"  ‖ t)          blinding factor
o   = H("ko"     ‖ t)          one-time key offset
Ko  = B + o·G                  one-time output key
key = H("aead"   ‖ t)          payload encryption key
```

On-chain fields:

```
features:     Plain | Coinbase   public; drives maturity
commit:       Commitment        v·G + b·H
range_proof:  Bulletproof       proves v ∈ [0, 2^64)
ephemeral_pk: [u8;32]           Ke
output_pk:    [u8;32]           Ko
payload:      Vec<u8>           AEAD(value ‖ blind ‖ memo[64])
sender_sig:   SchnorrSig        by r, verifiable against Ke
```

The payload is constant length, so ciphertext size reveals nothing about the
memo. **No recipient identifier appears anywhere.**

Spending `Ko` requires `b_spend + o`. The sender knows `o` but not `b_spend`, so
the sender cannot reclaim what they sent.

A scanning wallet must verify `Commitment::new(value, blind) == commit` before
accepting an output — otherwise a malicious sender can claim a payment they did
not make.

### 1.3 Input

```
commit: Commitment    must already exist in the UTXO set
sig:    SchnorrSig    under the Ko recorded in the set, over input_message
```

The one-time key is read from the UTXO set, never from the transaction.

### 1.4 Kernel

```
feature:     Plain | Coinbase
fee_darks:   u64        burned in full; public by design so the burn is auditable
reward_darks:u64        coinbase only
lock_height: u64
excess:      Commitment excess·H
excess_sig:  SchnorrSig Schnorr over generator H
```

**Balance equation:**

```
Σ outputs − Σ inputs + fee·G − reward·G  =  Σ kernel_excess
```

Sign check: `Σ excess` must be signed under `H`. Knowing the discrete log of a
point w.r.t. `H` proves it has no `G` component, i.e. the amounts cancel exactly.
*Equality alone proves nothing* — that was the v4 bug. The signature carries the
soundness.

### 1.5 Signature binding

```
kernel_msg = H(feature, fee, reward, lock_height, excess)
input_msg  = H(commit)
output_msg = H(features, commit, Ke, Ko, range_proof, payload)
```

Every consensus-relevant field is signed by exactly one of the three. **None
binds a transaction body** — see §2.0; a block dissolves its transactions, so a
body-bound signature would stop verifying.

Replaying a captured input signature is useless: the output leaves the UTXO set
the moment it is spent. Re-routing value is prevented by the kernel, which
requires knowledge of the input's blinding factor that no third party has.

---

## 2. Blocks

```
header:
  version:      u32
  height:       u64
  prev_hash:    Hash256      hash of the previous header, nonce included
  utxo_root:    Hash256      Merkle root over the sorted UTXO set
  kernel_sum:   Commitment   running Σ of every kernel excess ever accepted
  body_root:    Hash256      hash over the aggregated body
  timestamp_unix: u64
  difficulty:   u64
  nonce:        u64
  reward_darks: u64
body:
  inputs:  [Input]    sorted by commitment
  outputs: [Output]   sorted by commitment
  kernels: [TxKernel] sorted by kernel id
```

### 2.0 Aggregation

**A block contains no transactions.** Every transaction selected for a block is
merged into one flat set of inputs, outputs and kernels, then sorted
canonically. An observer sees *n* inputs, *m* outputs and *k* kernels with no
indication of which input paid which output — CoinJoin at block level, applied
automatically and without coordination.

Canonical ordering is a consensus rule. A block whose sets are not sorted is
rejected, because the order a miner chose would otherwise leak the original
transaction grouping.

Aggregation is why no signature binds a transaction body:

| Signature | Covers | Verified against |
|---|---|---|
| Kernel excess | feature, fee, reward, lock_height, excess | generator `H` |
| Input | the commitment being spent | the `Ko` in the UTXO set |
| Output (sender) | features, commit, `Ke`, `Ko`, proof, payload | `Ke` |

**Cut-through is not applied.** See §7.

`pow_hash = Nighthash-v2(header_preimage, nonce)`; the pre-image excludes the
nonce. Block identity is `H(preimage ‖ nonce)`.

**PoW parameters are consensus data**, not a local setting:

| Network | Memory per hash | Iterations | Lanes | Verify cost |
|---------|-----------------|------------|-------|-------------|
| Mainnet | 32 MiB | 1 | 1 | ≈ 11 ms |
| Testnet | 8 MiB | 1 | 1 | ≈ 2.5 ms |
| Devnet | 1 MiB | 1 | 1 | ≈ 0.8 ms |

Memory-hard proofs are symmetric: verifying costs the same as one mining
attempt. A node therefore records a local validation marker so it does not
re-hash its own chain on every restart; any change to the stored blocks file
forces full re-verification.

### 2.1 Difficulty

A hash `H` (big-endian 256-bit) satisfies difficulty `D` when

```
H · D < 2^256
```

computed exactly with 64-bit limbs — no division, no big-integer dependency.
Work per block is `D`; cumulative chain work is an exact `u128` sum.

**Retarget:** LWMA-1 (Zawy) every block over a 90-block window, target 15 s.
Solve times clamped to `[−5T, +6T]`, per-block movement clamped to ±2×, with a
network-specific floor.

### 2.2 Timestamps

- Must be **strictly greater** than the median of the previous 11 blocks.
- Must not exceed local time by more than **120 s**.

### 2.3 Chain selection

**Highest cumulative work wins.** Never block count. Reorgs deeper than 500
blocks are refused.

### 2.4 Coinbase maturity

| Network | Blocks |
|---------|--------|
| Mainnet | 1,440 (~6 h) |
| Testnet | 60 |
| Devnet | 10 |

---

## 3. Emission

Era-0 reward **6 NIGHT**, halving every **7,500,000 blocks**, hard cap
**90,000,000 NIGHT**, no tail.

Because each halving truncates (`reward >> halvings`), the curve terminates at
**89,999,999.25 NIGHT** — 0.75 below the ceiling. The cap is an upper bound.

---

## 4. Fees

While the subsidy is positive, `fee_darks` is burned. The miner receives only
the subsidy. After the subsidy is zero, the coinbase kernel carries the block's
fee total: nothing is minted, nothing is burned, circulating is unchanged, the
miner is paid. The fee remains a public `fee·G` term so the books still close.

---

## 5. Validation checklist

Per transaction (stateless):

- shape limits: ≤ 512 inputs, ≤ 512 outputs, ≤ 64 kernels, ≥ 1 output
- no duplicate inputs or output commitments
- all commitments and one-time keys are valid group elements
- every output carries a valid range proof bound to the network context
- every kernel signature verifies under `H` against `body_hash`
- `Σout − Σin + fee·G − reward·G == Σ kernel_excess`
- coinbase kernels carry no fee; plain kernels carry no reward
- a coinbase spends no inputs

Per block (stateful):

- protocol version matches
- height links, `prev_hash` links
- timestamp > median-time-past, ≤ now + 120 s
- difficulty equals the schedule; PoW meets it
- reward equals the emission schedule at this height and supply
- minting would not exceed the cap
- coinbase is transaction 0 and the only one
- every input exists in the UTXO set, is mature, and is correctly signed
- `tx_root`, `utxo_root` and `kernel_sum` match after application
- **the supply invariant still holds**

State is applied atomically: a rejected block leaves the node byte-identical.

---

## 6. Networking

Newline-delimited JSON over TCP.

- Messages capped at **4 MB**, enforced before allocation.
- ≤ 128 blocks per request; ≤ 64 peers.
- Handshake checks network id, wire version and genesis hash.
- Sync fetches forward from the local tip; it does not replay from genesis.
- Mempool: fee-ordered, conflict-checked, capped at 10,000 entries.

---

## 7. Known gaps

Deliberately listed in the spec so they are not forgotten.

- **Cut-through is not applied, and cannot be under this design.** Plain
  Mimblewimble can delete an output created and spent inside the same block,
  because ownership there is proven only by knowing a blinding factor that the
  kernel already covers. Nightfall additionally signs each input with the
  one-time key of the output being spent — the signature that makes
  *non-interactive* payments safe, without which the sender could sweep back
  what they sent. Deleting the input would delete that authorisation.
  One-sided payments and cut-through are mutually exclusive; `DECISIONS.md`
  chose one-sided payments.

  The consequence, stated plainly: **spent outputs stay visible, so the graph
  is obscured by aggregation but not erased.** Closing the remaining gap needs
  a proof system that can authorise a spend without publishing a per-input
  signature.
- **Dandelion-class stem/fluff** on the existing `Tx` message (not a separate stem graph). Optional SOCKS5/Tor for outbound dials. See `docs/PRIVACY.md`.

- **UTXO root is O(n log n) per block.** A Merkle Mountain Range would make it
  incremental.
- **Startup replays the whole chain.** No headers-first sync, no snapshots, no
  pruning.
- **No multi-asset support.** `asset_id` was removed rather than left as a field
  the value commitment ignored — the v4 arrangement would have permitted
  cross-asset inflation the moment a second asset existed.
