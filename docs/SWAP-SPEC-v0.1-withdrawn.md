# NIGHT ↔ BTC atomic swap — protocol specification

**Status: draft for review. Not implemented. Do not build from this yet.**

Version 0.1 · 28 August 2026 · target: NIGHTFALLCOIN 0.9.x

This document exists to be attacked. Every reviewer is asked to look for the
ordering mistake that lets one party keep both sides. The purpose of writing
it before any code is that in a two-party protocol holding money, a mistake in
the *sequence* is not a bug — it is a theft, and no amount of testing finds it
afterwards.

---

## 1. Scope

**In scope.** Two people who have already agreed on a price and an amount
exchange NIGHT for BTC without either of them, or anyone else, being able to
take both sides.

**Explicitly out of scope, and not by accident:**

| Not solved | Why |
|---|---|
| Order discovery | An order book needs an operator. An operator is the thing this design exists to avoid. Two parties arrive here already agreed. |
| Price | Not the protocol's business. |
| Escrow, arbitration, dispute resolution | All three mean a trusted third party. |
| Anything custodial | Neither party ever holds the other's funds under a key only they control. |

**No consensus change.** Verified against the 0.8.4 code: this is a wallet-level
protocol. Given that 10 % of the network runs the current release, anything
requiring a consensus change would not be deployable anyway.

---

## 2. What the chain already provides

Three facts about NIGHTFALLCOIN 0.8.4 that make this possible, all confirmed
in the source:

1. **`TxKernel.lock_height`** exists and is enforced by consensus
   (`nightfall-crypto/src/kernel.rs`). It is part of `signing_message()`, so it
   cannot be altered after signing. This gives NIGHT a native timelock.
2. **Outputs are spent with a Schnorr signature over generator `G`** against
   `output_pk = Ko` (`Transaction::verify_input_signature`). An output whose
   `Ko = (s_a + s_b)·G` is therefore spendable only by someone holding both
   halves. This is a 2-of-2 with no protocol support required.
3. **The kernel excess signature is a plain Schnorr signature** over generator
   `H` (`nightfall-crypto/src/schnorr.rs`). Adaptor signatures on Schnorr are
   a small, well-understood modification.

Point 1 is worth dwelling on: Monero does not have it, and its absence is the
known weak spot of XMR↔BTC swaps. See §8.4.

---

## 3. The hard part, stated up front

NIGHT is on **Ristretto255**. Bitcoin is on **secp256k1**. Different groups,
different orders (≈2²⁵² and ≈2²⁵⁶).

The protocol needs one scalar to be meaningful on both curves, which requires a
**cross-group discrete-logarithm-equality proof**: a proof that
`T_ristretto = t·G_ristretto` and `T_secp = t·G_secp` use the same `t`.

This is the expensive, error-prone component. It is several kilobytes, it is
built by bit-decomposition, and it is where an implementation is most likely to
be subtly wrong.

**It is also solved.** Monero has exactly this mismatch, and `xmr-btc-swap`
(COMIT / UnstoppableSwap) has run it in production for years using `sigma_fun`
and `ecdsa_fun`. **We adopt, we do not invent.** Any deviation from that prior
art must be justified in writing in this document, not in a commit message.

Scalars must be below the smaller order. All secret shares are drawn below
2²⁵² and rejected otherwise.

---

## 4. Roles and notation

**Alice** holds NIGHT, wants BTC.
**Bob** holds BTC, wants NIGHT.

The asymmetry is deliberate and load-bearing: the party whose chain has no
scripting locks second and redeems first.

| Symbol | Meaning |
|---|---|
| `s_a`, `s_b` | Ristretto scalars, the two halves of the shared NIGHT key |
| `S_a = s_a·G`, `S_b = s_b·G` | public halves |
| `S = S_a + S_b` | the shared output key `Ko` |
| `B_a`, `B_b` | secp256k1 public keys for the Bitcoin 2-of-2 |
| `T_a`, `T_b` | secp256k1 points corresponding to `s_a`, `s_b` |
| `π_a`, `π_b` | cross-curve DLEQ proofs that `S_x` and `T_x` share a scalar |
| `H₁`, `H₂` | Bitcoin timelock heights (cancel, punish) |
| `H_n` | NIGHT refund `lock_height` |

---

## 5. Bitcoin-side transactions

Standard XMR↔BTC construction. Taproot; all spends are key-path where possible.

- **TX_lock** — Bob locks BTC into a 2-of-2 between `B_a` and `B_b`.
- **TX_redeem** — spends TX_lock to Alice. Bob pre-signs it as an *adaptor
  signature encrypted under `T_a`*. Alice can only complete it using `s_a`, and
  completing it **publishes `s_a`**.
- **TX_cancel** — spends TX_lock, pre-signed by both, broadcastable after `H₁`.
- **TX_refund** — spends TX_cancel back to Bob. Alice pre-signs it as an
  adaptor signature encrypted under `T_b`; Bob completing it **publishes `s_b`**.
- **TX_punish** — spends TX_cancel to Alice, broadcastable after `H₂ > H₁`.
  Alice's compensation if Bob goes silent after cancel.

The whole design rests on one property: **taking the money reveals the secret.**

---

## 6. NIGHT-side transactions

- **TX_night_lock** — Alice sends her NIGHT to an output with `output_pk = S`.
  An ordinary transaction; the chain does not know it is special.
- **TX_night_claim** — spends that output. Requires a Schnorr signature under
  `s_a + s_b`. Whoever holds both halves can build and broadcast it alone.
- **TX_night_refund** — *new relative to XMR↔BTC; see §8.4.* A transaction
  spending the shared output back to Alice, co-signed by both parties **before**
  Alice locks, with kernel `lock_height = H_n`. Alice holds it; the chain
  refuses it before `H_n`.

### 6.1 How the shared output is actually built

Checked against `nightfall-crypto/src/stealth.rs`, and the answer changes the
setup phase, so it belongs here rather than in an open-questions list.

An output key is **not** freely chosen by the sender. The construction is:

```
t      = H(r·A)                    shared secret, receiver recomputes as H(a·Ke)
offset = derive_key_offset(t)
Ko     = B + offset·G              B is the recipient address's spend key
```

So the spend secret is `b + offset`, not `b`. A shared output therefore needs a
**shared address**, not merely a shared key:

- **spend half:** `B_shared = S_a + S_b`. Spend secret `s_a + s_b + offset`.
- **scan half:** `A_shared`, whose secret `a_shared` **both parties must know**.

The scan key grants visibility, never spending authority, so sharing it is safe
— and it is *required*: without `a_shared` Bob cannot recompute `t`, and without
`t` he can derive neither `offset` nor the output's blinding factor, and can
therefore neither find nor spend the output even holding both key halves.

`a_shared` must be exchanged in phase 0, before Alice locks.

**And this creates a verification duty that phase 2 must not skip.** Alice
builds the locking transaction and chooses `r`, so nothing stops her
constructing an output Bob can never spend. Before Bob treats the NIGHT as
locked he must recompute, from `Ke` and `a_shared`:

```
t' = H(a_shared·Ke)   →   offset' = derive_key_offset(t')
assert Ko == (S_a + S_b) + offset'·G
```

and additionally check the committed amount against the agreed one using the
blinding factor derived from `t'`. A Bob who skips either check can be induced
to release his BTC secret for an output that is not there.

**Consequence to state in the interface:** both parties can see this output.
That is unavoidable and correct — but it means a swap output carries no privacy
from the counterparty. See §9.

---

## 7. Protocol flow

### Phase 0 — setup, nothing at risk

1. Alice and Bob exchange `S_a, T_a, π_a` and `S_b, T_b, π_b`.
2. **Both verify the DLEQ proofs.** A party that skips this can be given a
   `T_x` unrelated to `S_x` and will pay out for a secret that unlocks nothing.
   This check is not optional and must fail closed.
3. They agree amounts, `H₁`, `H₂`, `H_n`, and construct the shared address
   (`B_shared = S_a + S_b`, plus a scan key whose secret both hold — see §6.1).
4. They construct and pre-sign, in this order:
   - TX_cancel (both signatures)
   - TX_refund (Alice's adaptor signature under `T_b`)
   - TX_punish (Bob's signature)
   - **TX_night_refund** (co-signed 2-of-2 under `s_a + s_b`, `lock_height = H_n`)

   Alice must not proceed past this point without a valid TX_night_refund in
   hand. It is her only unilateral exit.

### Phase 1 — Bob locks BTC

5. Bob broadcasts TX_lock and waits for confirmations.
6. Alice waits for the same confirmations before doing anything.

**Bob is now exposed and Alice is not.** This is correct: Alice's chain cannot
enforce anything, so she must be the one who commits second.

### Phase 2 — Alice locks NIGHT

7. Bob sends Alice his adaptor signature for TX_redeem under `T_a`.
8. **Alice verifies it** — that it is a valid adaptor signature, over the right
   transaction, under the right point. A wrong signature here means Alice locks
   NIGHT and cannot take the BTC.
9. Alice broadcasts TX_night_lock and waits for confirmations.
9a. **Bob verifies the locked output** against §6.1 — key derivation and
    amount — before treating phase 2 as complete. Skipping this loses his BTC.

### Phase 3 — the exchange

10. Alice completes the adaptor signature with `s_a` and broadcasts TX_redeem.
    She has her BTC. `s_a` is now public on the Bitcoin chain.
11. Bob extracts `s_a`, computes `s_a + s_b`, builds TX_night_claim and
    broadcasts it. He has his NIGHT.

Step 10 is the point of no return, and it is Alice's alone. Bob cannot be
harmed by it, because it is exactly what hands him his secret.

---

## 8. Abort paths

The happy path is the easy part. These are the cases that decide whether the
protocol is honest.

### 8.1 Bob never locks BTC
Nothing has happened. Alice walks away. No loss.

### 8.2 Alice never locks NIGHT
Bob's BTC sits in TX_lock. After `H₁` he broadcasts TX_cancel, then TX_refund,
recovering his BTC. TX_refund publishes `s_b` — harmless, since Alice never
locked anything.

### 8.3 Alice locks, then disappears without redeeming
After `H₁`, Bob cancels. He then faces a choice:
- Refund the BTC → publishes `s_b` → Alice can claim her NIGHT back. Both
  whole.
- Do nothing → after `H₂` Alice punishes and takes the BTC. She keeps the BTC,
  he keeps nothing, but he chose that.

**Bob is never worse off for behaving correctly.** That is the property to
check.

### 8.4 The case XMR↔BTC cannot solve, and we can

In Monero, if Bob never publishes `s_b`, Alice's XMR is **locked in the shared
address forever**. She is compensated with the BTC via punish, but the XMR is
destroyed. This is a known and accepted wart of that protocol.

`lock_height` removes it. TX_night_refund, co-signed in phase 0, lets Alice
recover her NIGHT unilaterally after `H_n` regardless of what Bob does.

**This is the only part of this design that is not inherited from reviewed
prior art, and it is therefore the part most likely to be wrong.** Two
requirements fall out of it, and both are load-bearing:

- `H_n` must sit **after** the window in which Bob may legitimately claim, or
  Alice can refund a swap Bob already paid for. Bob learns `s_a` at step 10 and
  needs time to broadcast; `H_n` must exceed the worst case of that plus
  reasonable confirmation delay and mempool congestion.
- The pre-signed refund must be bound to exactly one shared output. A refund
  that could be replayed against a later output is a theft primitive.

**Open question for reviewers: is there a Bob strategy that makes Alice's
refund and Bob's claim both valid in the same window?** If yes, the design
fails and we fall back to accepting Monero's wart.

### 8.5 One side's chain reorganises
Both parties wait for confirmations before every irreversible step. NIGHT's
`MAX_REORG_DEPTH` is 500 blocks at a 15-second target — roughly two hours.
Confirmation counts must be chosen against that, not against feel.

### 8.6 A party crashes mid-swap
Every secret and every pre-signed transaction must be persisted to disk before
the step that makes it necessary. A swap that cannot survive a power cut is a
swap that loses money to a power cut.

---

## 9. Threat model

Assumed: both parties are hostile and will deviate at any point if it profits
them. Network messages may be dropped, delayed, reordered or forged. Either
chain may reorganise within its stated bound.

Not defended against: an attacker who controls the majority of hashrate on
either chain; endpoint compromise; a party who loses their keys.

**Privacy note, and it matters for this project specifically:** an atomic swap
publishes a NIGHT transaction and a Bitcoin transaction that are linked in time
and amount. The chain analysis that NIGHT exists to defeat becomes materially
easier against swap traffic. This must be said plainly in the interface, not
buried in a document. Users who need the privacy property should not assume it
survives a swap.

---

## 10. Implementation plan

| Phase | Deliverable | Gate before proceeding |
|---|---|---|
| 0 | This document, publicly reviewed | At least one competent outside reader has tried to break it |
| 1 | Adaptor signatures on the existing Ristretto Schnorr | Test vectors; property tests; no protocol code yet |
| 2 | Cross-curve DLEQ, adopted from `sigma_fun` | Verified against the reference implementation's vectors |
| 3 | Shared output + pre-signed timelocked refund on **testnet** | §8.4 resolved in writing |
| 4 | Full state machine, every abort path in §8 | Each path has an automated test that forces it |
| 5 | Long testnet operation, small amounts, real counterparties | Weeks, not days |
| 6 | External cryptographic review | **Before mainnet. Not negotiable.** |
| 7 | Wallet interface | Last, deliberately |

Crates: `curve25519-dalek` (present), `rust-bitcoin`, `sigma_fun`, `ecdsa_fun`.

Honest estimate: `xmr-btc-swap` took a funded team well over a year to
production. Part-time and alone, this is a 2027 project.

---

## 11. Open questions

1. §8.4 — can Alice's refund window and Bob's claim window overlap?
2. What confirmation depths on each chain, derived from reorg bounds rather
   than from convention?
3. Can the pre-signed NIGHT refund be bound tightly enough to one output to
   make replay impossible?
4. Kernel fees on the pre-signed refund: fees are burned and fixed at signing
   time. What happens if the fee is too low when `H_n` arrives?
5. Transport between the two parties — out of scope here, but it must not
   become an operator by the back door.
6. ~~Does the stealth output construction interfere with using `S` as
   `output_pk`?~~ **Answered in §6.1: it works, but requires a shared
   *address* with a shared scan key, plus a verification duty on Bob.**
7. Is `derive_key_offset` domain-separated well enough that a shared scan key
   cannot be abused across two concurrent swaps between the same pair?

---

## 12. What would make this document wrong

Ways this could be a bad design that reading it will not reveal:

- The cross-curve DLEQ is adopted incorrectly, and the proof verifies for
  mismatched scalars.
- `H_n` is chosen by intuition rather than derived, and a congested mempool
  turns a correct protocol into a loss.
- The NIGHT refund is a genuinely new construction in a field where new
  constructions are usually broken.

If you are reviewing this: start at §8.4.
