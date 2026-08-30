# NIGHT ↔ BTC atomic swap — protocol specification

**Version 0.3 · 29 August 2026 · draft for review · not for real coins**

## What changed from v0.1, and why

v0.1 proposed a timelocked NIGHT refund (`TX_night_refund`) that would remove
the known wart of XMR↔BTC swaps, where the Monero side can be locked forever.
It asked reviewers to attack that section first.

**They did, and it is broken. The section is withdrawn.** v0.1 is kept at
`SWAP-SPEC-v0.1-withdrawn.md` so the mistake stays readable.

The break, in one paragraph. On this chain an input signature commits to
`H(commit)` and **nothing else** — not the kernel, not `lock_height`, not the
outputs (`Transaction::input_message`, verified). A kernel is signed separately,
under generator `H`, with the blinding excess `b_out − b_in`
(`TxKernel::signing_message`, verified). The two signatures do not mention each
other. So a co-signed input signature is not a pre-signed *transaction*; it is a
spending authorisation that pairs with **any** balancing kernel. And Alice, as
the sender of the lock output, knows `b_in` by construction — she chose `r`, and
`b_in = derive_blind(H(r·A_shared))`. She can therefore build a fresh output to
herself, compute a fresh excess, sign a kernel with `lock_height = 0`, and spend
the lock immediately. Then she redeems the BTC. She has both sides; an honest
Bob has nothing. No race, no reorg, no crash required — available the moment
both locks confirm.

There is no wallet-level repair. Binding a spend to a kernel is a consensus
change, and the whole point of the design was that it needed none.

A second, independent flaw: `TX_punish` and a working NIGHT refund pay Alice
twice. In XMR↔BTC, punish is *compensation for Monero that can never be
recovered*. Give the seller a working refund as well and the compensation
becomes a second payout — for every ordering of the three timelocks, because
they live on two chains that cannot make each other exclusive.

**Consequence: we ship the known protocol and accept the wart.** Below is
Option A from the attack report.

Credit: attack report of 28 August 2026 against v0.1, checked against the 0.8.4
source rather than against v0.1's summary of it.

---

## 1. Scope

Two people who have already agreed on a price exchange NIGHT for BTC without
either of them, or anyone else, being able to take both sides.

**Out of scope, deliberately:** order discovery (an order book needs an
operator), price, escrow, arbitration, anything custodial.

**No consensus change**, and after v0.1 that constraint is understood as a hard
limit on what is expressible, not merely a deployment preference.

---

## 2. What the chain provides, stated precisely

| Object | Signed message | Key / generator |
|---|---|---|
| Input | `H(commit)` | `Ko`, generator `G` |
| Kernel | `H(feature, fee, reward, lock_height, excess)` | blinding excess, generator `H` |
| Output | `H(features, commit, Ke, Ko, proof, payload)` | sender's ephemeral `r`, generator `G` |

**None of the three binds a transaction body.** That is deliberate — aggregation
dissolves transactions, so a body-bound signature would stop verifying once the
body ceased to exist. It is also the reason v0.1 failed.

`G = PedersenGens::default().B` from the bulletproofs crate. v0.2 asserted this
was **not** the Ristretto basepoint; that was an assumption, and it is wrong.
Measured (`swap::generator_tests`):

```
generator_g()  = e2f2ae0a6abc4e71a884a961c500515f58e30b6aa582dd8db6a65945e08d2d76
basepoint      = e2f2ae0a6abc4e71a884a961c500515f58e30b6aa582dd8db6a65945e08d2d76
```

They are the same point. See §5 for what that changes.

`lock_height` is a kernel field. It delays *that kernel*. It does not lock an
output.

---

## 3. Roles

**Alice** holds NIGHT, wants BTC. **Bob** holds BTC, wants NIGHT.

Bob locks first because his chain can enforce; Alice commits second and redeems
first, and her redemption is what hands Bob his secret.

| Symbol | Meaning |
|---|---|
| `s_a`, `s_b` | Ristretto scalars, halves of the shared spend key |
| `S_x = s_x·G` | public halves, `G` as in §2 |
| `T_x = s_x·G_secp` | the same scalars on secp256k1 |
| `π_x` | cross-curve DLEQ that `S_x` and `T_x` share `s_x` |
| `a_shared` | scan secret of the shared address, known to both |
| `offset` | `derive_key_offset(H(r·A_shared))` |
| `H₁`, `H₂` | Bitcoin cancel and punish timelocks |

---

## 4. The shared address

An output key is not freely chosen. `Ko = B + offset·G`, so the spend secret is
`b + offset`.

The shared address is `B_shared = S_a + S_b` with a scan key whose secret both
parties hold. **The on-chain spend secret is therefore `s_a + s_b + offset`, not
`s_a + s_b`.** v0.1 stated the latter in one section and the former in another;
an implementation following the wrong one produces signatures that do not
verify.

`a_shared` must be exchanged before Alice locks: without it Bob can derive
neither `offset` nor the blinding factor, and can neither find nor spend the
output even holding both key halves.

**Both parties can see this output. It carries no privacy from the
counterparty.** Say so in the interface.

---

## 5. Cross-curve DLEQ

NIGHT is Ristretto255, Bitcoin is secp256k1. One scalar must be meaningful on
both, which needs a cross-group discrete-log-equality proof. This is the most
error-prone component and it is adopted, not invented — Monero has the same
mismatch and `xmr-btc-swap` has run this in production for years.

**How much of it is a copy — measured, not assumed.** v0.2 warned that the
proof would have to be re-instantiated on a non-standard generator and that
`xmr-btc-swap`'s vectors would not apply. That warning was based on an
unverified reading of `commit.rs`. The generator is in fact the Ristretto
basepoint (§2), so a proof written against the standard basepoint *is* a proof
about `S_x`, and the reference construction carries over directly.

One real difference survives, and it is about encoding rather than about the
generator: Monero works with **Ed25519** points, we work with **Ristretto**.
The scalar field is identical — same order ℓ — so the proof's scalar
arithmetic is unchanged; the point type and its serialisation are not. Test
vectors therefore have to be regenerated for Ristretto encodings even though
the underlying construction is the same. That is a much smaller job than
re-instantiating a proof on a foreign generator, and it should be stated as
such rather than left as the earlier, scarier claim.

Scalars are rejection-sampled below `2²⁵²`. Interpreting the same 32 bytes as
both a Ristretto and a secp256k1 scalar without rejection yields two different
integers whenever the bytes exceed either order.

**DLEQ is load-bearing against rogue keys.** Without it Bob picks
`S_b = P − S_a` for a `P` he controls and spends the lock alone. Verification
must fail closed, on both sides, before anything is locked.


### 5.1 What `sigma_fun` actually gives us — measured, task B1

Four findings, all against `sigma_fun` 0.9.0 and `curve25519-dalek` 4.1.3.

**The generator is not a problem.** `generator_g()` is the Ristretto basepoint
(§2), and `sigma_fun`'s `DL` takes the generator as part of the statement
anyway, so an arbitrary one would have been supported regardless. The earlier
warning was doubly wrong.

**The 252-bit bound is not ours, it is theirs too.** `CrossCurveDLEQ::prove`
asserts `secret.as_bytes()[31] & 0b0001_0000 == 0` — the same bound
`SwapShare::generate` samples under. That agreement was luck, and it is now a
documented constraint rather than a coincidence.

**The real mismatch is the point type, not the curve.** `sigma_fun`'s ed25519
module works on `EdwardsPoint`; our keys are `RistrettoPoint`.
`RISTRETTO_BASEPOINT_POINT` is *defined* as `RistrettoPoint(ED25519_BASEPOINT_POINT)`,
so `s·G_ristretto` and `s·G_ed25519` are the same underlying point — but
`RistrettoPoint`'s inner field is `pub(crate)` and dalek exposes no
Edwards→Ristretto map. **A verifier cannot check the correspondence through the
public API.** So the stock proof, used unchanged, would prove a statement about
an Edwards point that we cannot tie to the `S_x` in our address.

**The fix is a new leaf, not a fork.** `Sigma` is a public trait. We implement
`ristretto::DL` and `ristretto::DLG` ourselves and compose them with the
crate's existing `Eq`, `And`, bit-decomposition and Fiat-Shamir machinery,
which stays reviewed and unmodified. Only the leaf is ours.

**This is a deviation, and §3 requires it be justified here.** Justification:
there is no alternative that keeps the proof about our actual keys. It also cuts
one class of risk — `sigma_fun`'s ed25519 leaf must reason about torsion, while
Ristretto is prime-order and cannot have any. It adds another: the leaf is code
that has *not* been reviewed by anyone, in a proof system where a subtly wrong
leaf verifies happily. Task G2 must look here first.

---

## 6. Bitcoin side — ECDSA, P2WSH, no Taproot

**Decision, and it is a reversal of v0.1.** v0.1 said "Taproot, key-path where
possible" while also naming `ecdsa_fun`. Those are two different protocols.

We use **ECDSA adaptor signatures on a P2WSH 2-of-2**, exactly the
`xmr-btc-swap` transaction tree. Reasons: it is the construction that has been
reviewed and running; BIP340 adaptor plus MuSig2 has nonce-handling footguns and
no track record for *this* protocol; and the abort tree needs script-path CSV
anyway, so "key-path where possible" was never true of it. ECDSA malleability
(`s` vs `n−s`) must be handled explicitly or secret extraction breaks.

Transactions: **TX_lock** (Bob's BTC into 2-of-2) · **TX_redeem** (to Alice, Bob
pre-signs as adaptor under `T_a`; completing it publishes `s_a`) ·
**TX_cancel** (both-signed, after `H₁`) · **TX_refund** (to Bob, Alice pre-signs
as adaptor under `T_b`; completing it publishes `s_b`) · **TX_punish** (to
Alice, after `H₂ > H₁`).

`B_a`, `B_b` need proof of knowledge. The DLEQ is over `(S_x, T_x)`; the Bitcoin
2-of-2 keys are different keys. On a script 2-of-2 both signatures are required
so this is safe; it would not be on a summed key.

**The adaptor lives on Bitcoin, not on the NIGHT kernel.** v0.1's implementation
plan began with adaptor signatures on the Ristretto Schnorr; that is the wrong
object and would have been months on the wrong thing. An adaptor on the NIGHT
kernel encrypts under a blinding excess — the wrong secret. The happy path needs
no NIGHT adaptor at all.

---

## 7. NIGHT side

- **TX_night_lock** — Alice pays the shared address. An ordinary transaction.
- **TX_night_claim** — spends it with a signature under `s_a + s_b + offset`.

That is all. There is no NIGHT refund. See the header.

---

## 8. Flow

Message order is written in `nightfall-swap::session`. The two sides do **not**
alternate — Bob sends 0, 2, 4, 5; Alice sends 1, 3 — so sequence numbers are
named, not computed.

```
Bob   → Message0            b_btc, offer_b (DLEQ), refund_spk, amounts, depths
Alice → Message1            a_btc, offer_a, redeem_spk, punish_spk, scan_secret
Bob   → Message2            TX_lock, unsigned (Alice rebuilds every child from it)
Alice → Message3            TX_cancel signature, TX_refund adaptor under T_b
Bob   → Message4            TX_punish and TX_cancel signatures
Bob   → MessageRedeemEnc    TX_redeem adaptor under T_a
```

**Phase 0 — nothing at risk.** Fresh keys (§10). Exchange `S_x, T_x, π_x` and
`a_shared`; **verify both DLEQs, fail closed**. Agree amounts and `H₁`, `H₂`.
Alice signs Bob's abort path (message 3) **before** he broadcasts the lock —
that is the whole of Bob's safety. Depths in message 0 are Bob's; Alice
refuses any set that leaves no redeem window after the Bitcoin lock confirms
(`Depths::alice_can_finish`).

**Phase 1.** Bob broadcasts TX_lock (signed in his own Bitcoin wallet; this
wallet exports the unsigned transaction / PSBT and checks the confirmed tx
equals the one both sides built). Both wait for depth (§11).

**Phase 2.** Bob sends his TX_redeem adaptor under `T_a`. Alice verifies it is a
valid adaptor over the right transaction under the right point — a wrong one
here costs her the NIGHT. Alice broadcasts TX_night_lock.

**Phase 2a — Bob's verification duty.** Against the announced output, not a
chain scan (a wrong view tag makes a scanner skip it, and Alice can name a txid
that scans clean while locking something else). Recompute
`t' = H(a_shared·Ke)`, then assert `Ko == B_shared + derive_key_offset(t')·G`,
and assert the commitment equals `Commitment::new(agreed_value, derive_blind(t'))`.
If the payload does not open, or `t'` does not explain the commitment, **abort
and wait for `H₁`**. Skipping any of this loses Bob's BTC.

**Phase 3.** Alice completes the adaptor and broadcasts TX_redeem; `s_a` becomes
public. Bob extracts it and claims the NIGHT with `s_a + s_b + offset`.

**Alice must not begin phase 3 close to `H₁`.** See §9.2.

---

## 9. Abort paths

**9.1 Bob never locks.** Nothing happened.

**9.2 Alice redeems too close to `H₁` — Bob takes both.** Inherited, and v0.1
missed it entirely. If TX_redeem does not confirm before `H₁`, Bob cancels;
TX_redeem is then invalid, but `s_a` was already published in its adaptor
completion and is recoverable from the mempool, relay logs or a reorged block.
Bob claims the NIGHT with it and refunds his BTC. **The state machine needs a
hard cutoff: if the remaining margin to `H₁` is below the time to confirm at
current fee levels, do not redeem — abort and let the cancel path run.** Derived
from fee estimation, not from feel.

**9.3 Alice never locks.** Bob's BTC is frozen until `H₁`, then cancel and
refund. Not theft, but a real capital-lockup grief, inherent to Bob-locks-first.

**9.4 Alice locks and disappears.** After `H₁` Bob cancels, then refunds —
publishing `s_b`, which lets Alice recover her NIGHT. If Bob never refunds,
Alice punishes at `H₂` and **the NIGHT is stuck forever.**

**This is the wart. It is real, it is inherited, and it must be visible in the
interface — not in a document.** A user whose counterparty crashes at the wrong
moment can lose the NIGHT side. Do not advertise a refund that does not exist.

**9.5 Reorgs.** See §11. There must be numbers.

**9.6 Crash.** Persisting state is necessary and not sufficient: the machine
must resume *into* cancel or refund, not into "wait for the peer".
`xmr-btc-swap` has shipped loss-of-funds bugs here more than once.

---

## 10. Keys are single-use

After a successful swap `s_a` is public on Bitcoin; after a refunded abort `s_b`
is. Reuse means anyone who watched the first swap can take the second lock for
free.

**Fresh `s_a`, `s_b`, `B_a`, `B_b`, `a_shared`, `r` for every swap.** Derive
`T_x` from `s_x`, never the reverse. A reused `a_shared` is a linkability leak;
reused `s_x` is theft.

---

## 11. Confirmation depths

`MAX_REORG_DEPTH` is 500 blocks — about two hours at the 15-second target. A
node will adopt a 499-block reorg.

Waiting 500 blocks before every irreversible step means two hours of both
capitals locked per swap. Waiting less means accepting that a deeper reorg is a
theft: the NIGHT lock disappears, Alice's inputs are unspent on the new fork, and
she already has the BTC.

NIGHT is a small-hashrate 15-second chain, so a 50-block reorg without any
majority attack is more plausible than Bitcoin intuition suggests.

**Numbers, derived (`nightfall-swap::timelock::Depths::mainnet`):**

| Chain | Confirmations | Why |
|---|---|---|
| NIGHT lock before Alice redeems BTC | 500 | `MAX_REORG_DEPTH`. Waiting less is accepting theft. ~2 hours. |
| Bitcoin TX_lock / TX_redeem | 6 | No equivalent bound on Bitcoin. Conventional residual, ~1 hour. |
| H₁ (cancel CSV) | 144 | ~24 h after TX_lock. 500 NIGHT blocks are ~12.5 BTC blocks; 144 leaves margin. |
| H₂ (punish CSV) | 144 | ~24 h after TX_cancel. |
| Redeem cutoff | 12 BTC blocks remaining to H₁ | Spec §9.2. Derived from ~2 h of fee-estimation slack, not from feel. |

Residual risk, on screen: a Bitcoin reorg deeper than 6 after TX_redeem, or a NIGHT reorg of 500, is out of scope (majority hashrate).

---

## 12. Implementation plan

| Phase | Deliverable | Gate |
|---|---|---|
| 1 | ECDSA adaptor on P2WSH, borrowed from `xmr-btc-swap`, with its vectors | malleability handled and tested |
| 2 | Cross-curve DLEQ instantiated on NIGHT's `G` | **our own** vectors; rogue-key test |
| 3 | Shared address, Bob's §8 phase-2a verification | every failure mode aborts, on testnet |
| 4 | State machine, every path in §9 forced by a test | resume-after-crash lands in refund |
| 5 | Long testnet operation, small amounts | weeks |
| 6 | External cryptographic review | **before mainnet, not negotiable** |
| 7 | Interface, with the wart and the privacy warning on screen | last |

Crates: `rust-bitcoin`, `ecdsa_fun`, `sigma_fun`, `curve25519-dalek` (present).

---

## 13. Open, and honestly open

1. Signet (real block times). Regtest hides every timing problem.
2. Whether the capital-lockup grief of §9.3 is acceptable between strangers.
3. External cryptographic review of `dleq.rs` (the Ristretto leaf). Not optional
   before mainnet.

Transport is copy-paste packets (§16). A mailbox that can withhold a message is
still an operator and is still forbidden.

---

## 15. Driver contract (v0.3)

The wallet tab does not move a swap. A driver does. Rules:

1. Query **both** chains in one pass. `NightConf { night, btc }` carries both
   heights from that same moment. A Bitcoin outage is `WatchError`, never
   "0 confirmations" and never "height unchanged".
2. Persist the new state **before** broadcasting. A crash between disk and wire
   must still know that a send was intended (`PendingSend`).
3. On a query error, do nothing. The next healthy tick continues.
4. Every send is idempotent. `already known` from the node is success.
   `testmempoolaccept` before `sendrawtransaction`; a reject is
   `NeedsAttention`, not a retry loop.
5. A node that reports `loading`, or whose peer height is ahead of ours, is not
   a source of tip height.
6. After a crash: resume from disk. If Bitcoin lock confirms ≥ H₁, land in
   cancel, not in "wait for the peer".

Fee ladder: abort transactions are pre-signed at several absolute fees
(`FeeLadder::mainnet`). The sighash binds the fee; it cannot be raised later.
At broadcast, pick the cheapest rung that still meets `estimatesmartfee`.

---

## 16. Copy-paste packets

Version 1, network id, swap id, sequence number, agreed amounts, checksum
(first 4 bytes of SHA-256 over those fields and the body). Import fails closed
with a distinct error for: bad version, bad checksum, wrong id, wrong network,
wrong sequence, changed amounts. There is no server.

---

## 17. Automaton defects found after v0.2, and the repair

1. `NightLocked` ignored `BtcConf`. H₁ could pass during the NIGHT wait and
   `NightConf` still opened `ReadyToRedeem`. Repair: `NightConf` carries `btc`;
   `BtcConf` in `NightLocked`/`ReadyToRedeem` aborts when the margin is gone.
2. A catch-all `TooCloseToCancel` rewound `Redeeming` and `Done`. Repair: that
   event is only valid from `NightLocked` and `ReadyToRedeem`.
3. `SwapEvent::Crash` without a height aborted before H₁. Repair: crash
   recovery is `resume(confirms)`, not an unparameterised event.
4. `Sequence::from_height` takes `u16`. `as u16` on H₁ > 65535 wrapped to a
   minable cancel. Repair: `csv_height` returns `TimelockTooLarge`.

---

## 14. What would make this document wrong

v0.1 was wrong because it invented one piece. This version invents nothing, so
the likelier failures are transcription errors: a DLEQ instantiated on the wrong
generator, ECDSA malleability mishandled, a state machine that resumes into the
wrong branch, timelocks derived from convention rather than from the two chains'
actual reorg behaviour.

**And it may still be wrong in a way nobody has found.** v0.1 read as carefully
reasoned too. If you are reviewing: the fact that every piece here is inherited
is an argument about provenance, not a proof.
