# Nightfall — Design decisions

## v5 lock — 2026-08-12 (post-audit)

These supersede the v4 rows below, which were made before the
[security audit](./AUDIT-2026-08-12.md) established that v4 was
consensus-broken.

| # | Question | Decision |
|---|----------|----------|
| 1 | Privacy architecture | **Mimblewimble** (Grin-class) with **one-sided stealth outputs** (Litecoin MWEB construction). Chosen over a Zcash-style shielded pool because it is cryptographically complete *today* without a trusted setup or SNARK circuits. Accepted trade-off: the transaction graph stays linkable until cut-through is built. |
| 2 | Value soundness | **Schnorr excess signature over generator `H`** proving the excess has no `G` component, plus **Bulletproof range proofs** on every output. Replaces the v4 "balance proof", which proved nothing at all. |
| 3 | Supply verification | Global invariant `Σ UTXO − Σ kernel_excess = (minted − burned)·G`, checked after every block, on startup, and via RPC. Supply is now *provable*, not asserted. |
| 4 | Non-interactive payments | Required — the Core Wallet's paste-and-send flow cannot ask both parties to be online. Solved with one-time output keys `Ko = B + H("ko" ‖ t)·G`, which also prevent the sender from reclaiming what they sent. |
| 5 | Curve | **Ristretto everywhere.** v4 mixed ed25519 and x25519 and derived the nullifier from the spend key's bytes, which made a safe view key impossible. |
| 6 | View keys | **Implemented.** Scan/spend key split; `(a, B)` detects and decrypts, cannot sign. Enforced by the type system. |
| 7 | Difficulty | `u64` difficulty with the exact test `hash · D < 2^256`. **LWMA-1** retarget every block over a 90-block window. Replaces "leading zero bits" clamped to 28, which capped network security at a few seconds of CPU time. |
| 8 | Chain selection | **Cumulative work**, never block count. |
| 9 | Timestamps | Median-time-past over 11 blocks; future drift 120 s (was 2 h). Solve times clamped for time-warp resistance. |
| 10 | Coinbase maturity | 1,440 mainnet / 60 testnet / 10 devnet. v4 had none. |
| 11 | Fee burn | Unchanged: **100 %**. The fee is public in the kernel specifically so the burn is auditable. |
| 12 | Multi-asset | **Removed for now.** v4 carried an `asset_id` the value commitment ignored, which would have allowed cross-asset inflation the moment a second asset existed. Reintroduce only with per-asset generators. |
| 13 | Atomicity | Block application is two-stage: validate against a staged view, then commit. A rejected block leaves state byte-identical. |
| 14 | PoW algorithm | **Argon2id (Nighthash-v2)**, 32 MiB per hash on mainnet. Memory-hardness is what makes purpose-built hardware uneconomical. Accepted cost: verification is as expensive as one mining attempt (~11 ms), so nodes keep a local validation marker instead of re-hashing their own chain on restart. |
| 15 | Block structure | **Aggregated.** A block holds one flat, canonically sorted set of inputs, outputs and kernels; transactions do not survive into it. Kernels therefore sign only their own fields, and output integrity moved to a per-output sender signature. |
| 16 | Cut-through | **Not applied.** It would delete the per-input signature that makes one-sided payments safe. Choosing non-interactive payments (row 4) rules it out. Documented rather than hidden. |

### Open, deliberately

- **Cut-through**, or a spend-authorisation scheme that does not require a per-input signature. Until then the transaction graph is obscured, not erased.
- **Dandelion++** — no network-layer privacy today.
- **Independent audit** — the v5 code was written by the same party that audited v4.
- **UTXO snapshots / headers-first sync** — initial sync still replays and re-verifies every block, which memory-hard PoW makes expensive.

---

## v4 lock — superseded

**Date:** 2026-08-12 (morning)
**Authority:** builder autonomy granted by the project owner.

Retained for the record. Rows 1, 2, 4 and 9 were **not correctly implemented**;
see the audit.

| # | Question | Decision |
|---|----------|----------|
| 1 | PoW algorithm | Nighthash-v1: Blake3(header ‖ nonce) ≤ target. Argon2id migration path before production. |
| 2 | ZK / privacy proofs | "Nightproof-α": Pedersen commitments for amount balance, ed25519 spend auth, note plaintexts never on chain. — **The balance check was a tautology and the recipient key was published in cleartext.** |
| 3 | Fee burn | 100 % base fee burn. Miner income = subsidy only. |
| 4 | Light / mobile | Note-scan via birthday height + trial decrypt. |
| 5 | Unshield | No transparent unshield on mainnet. |
| 6 | Bridges | None in v1. |
| 7 | Block time | 15 s. |
| 8 | Difficulty | Exponential moving retarget every 240 blocks. — **Replaced; see v5 row 7.** |
| 9 | Coinbase | Shielded miner note bound by emission schedule and Pedersen balance. — **The binding did not exist.** |
| 10 | Network modes | devnet / testnet / mainnet chain ids. |
