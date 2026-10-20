# Audit Readiness

Status: Draft
Author: nightfall-wizard
Created: 2026-10-03

## Purpose

An external security audit requires three things a firm cannot
invent on its own: an agreed scope, a stable set of invariants
to test, and enough material to give a fixed-price quote. This
document provides all three.

It is not an audit. It does not replace an audit. It does not
claim that the protocol is secure. It is the input document
that makes an external audit possible.

## Background

Three internal documents already exist:

- `docs/AUDIT-2026-08-12.md` — security audit of protocol v4
  (Nightproof-α). Twenty-one findings, all marked fixed in v5.
- `docs/AUDIT-2026-08-16.md` — internal review of v5. Contains
  the explicit self-disclosure I-01: *"This document is not an
  outside audit."* Eleven findings remain open (L-01 … D-01).
- `docs/AUDIT-2026-09-08.md` — operational security document.
  Regression evidence, open risks, website hardening.

None of these was performed by an independent third party.
None of them can be pointed to as an external review.

## 1. Proposed scope for an external audit

### In scope (Phase A — cryptographic core)

| Crate | File | Lines |
|---|---|---|
| `nightfall-crypto` | `commit.rs` | 137 |
| `nightfall-crypto` | `kernel.rs` | 328 |
| `nightfall-crypto` | `keys.rs` | 284 |
| `nightfall-crypto` | `pow.rs` | 417 |
| `nightfall-crypto` | `rangeproof.rs` | 155 |
| `nightfall-crypto` | `schnorr.rs` | 153 |
| `nightfall-crypto` | `stealth.rs` | 687 |

Total: approximately 2,500 lines. Compact enough for a focused,
four-to-six-week engagement.

### In scope (Phase B — ledger and consensus)

- `nightfall-ledger/src/utxo.rs` — UTXO set and supply invariant
- `nightfall-ledger/src/lib.rs` — state transition atomicity
- `nightfall-consensus/src/lib.rs` — chain, work, reorg, PoW
- `nightfall-types/src/lib.rs` — type invariants

### In scope (Phase C — node and P2P surface)

- `nightfall-p2p/src/lib.rs` — peer session handling
- `nightfall-node/src/runtime.rs` — ingest and reorg paths
- `nightfall-storage/src/lib.rs` — persistence, crash consistency

### Out of scope for the first audit

- Wallet UI (mobile, web, core)
- Mining economics beyond the emission schedule
- Governance, marketing, or distribution

These can be a second engagement once Phase A-C complete.

## 2. Security invariants

Each invariant below is a claim the protocol must satisfy at
every reachable state. An external audit should test each one
independently, with a specific reproduction method.

### Consensus-level

**I-C1.** `Σ UTXO − Σ kernel_excess = (minted − burned) · G`
under every accepted block. Reference: `nightfall-ledger/src/utxo.rs`.

**I-C2.** `total_minted_darks ≤ MAX_SUPPLY_DARKS` (90,000,000 NIGHT).
Reference: `nightfall-ledger/src/utxo.rs`.

**I-C3.** Kernel excess signature is valid under the second
generator `H` for every accepted kernel. Reference:
`nightfall-crypto/src/kernel.rs`.

**I-C4.** Block application is atomic: a rejected block leaves
`LedgerState` byte-identical to before. Reference:
`nightfall-ledger/src/lib.rs`.

**I-C5.** Chain selection is by cumulative work, not height.
Reference: `nightfall-consensus/src/lib.rs`.

### Cryptographic

**I-X1.** Bulletproof range proofs accept only values in `[0, 2^64)`.
Reference: `nightfall-crypto/src/rangeproof.rs`.

**I-X2.** Schnorr signatures reject non-canonical `s` (≥ group order).
Reference: `nightfall-crypto/src/schnorr.rs`.

**I-X3.** Stealth addresses cannot be linked to each other by an
observer. Reference: `nightfall-crypto/src/stealth.rs`.

**I-X4.** Pedersen commitments are binding under the discrete
logarithm assumption. Reference: `nightfall-crypto/src/commit.rs`.

### Network and node

**I-N1.** A remote peer cannot force unbounded memory allocation.
Reference: `docs/AUDIT-2026-08-12.md`, finding N-06.

**I-N2.** A rejected block does not trigger full chain re-pull.
Reference: `docs/AUDIT-2026-08-12.md`, finding N-07.

**I-N3.** Miner does not hold the global state lock. Reference:
`docs/AUDIT-2026-08-12.md`, finding N-03.

### Wallet (low priority for Phase A)

**I-W1.** Wallet seed is never written world-readable.
**I-W2.** RPC enforces loopback-only binding.

## 3. Attack surface map

An auditor should test each row independently.

| Surface | Where | Threat |
|---|---|---|
| PoW verification | `nightfall-crypto/src/pow.rs` | forged header accepted |
| Kernel accumulation | `nightfall-crypto/src/kernel.rs` | balance equation bypassed |
| Range proof verification | `nightfall-crypto/src/rangeproof.rs` | negative amount accepted |
| Signature canonicality | `nightfall-crypto/src/schnorr.rs` | signature malleability |
| Stealth output scanning | `nightfall-crypto/src/stealth.rs` | output linkability |
| UTXO set mutation | `nightfall-ledger/src/utxo.rs` | double spend, inflation |
| Reorg handling | `nightfall-consensus/src/lib.rs` | chain split acceptance |
| Peer message parsing | `nightfall-p2p/src/lib.rs` | remote OOM, panic |
| Storage persistence | `nightfall-storage/src/lib.rs` | crash-consistency loss |
| RPC endpoint | `nightfall-node/src/rpc.rs` | unauthenticated access |

## 4. Known open items from internal review

The following are explicitly acknowledged in
`docs/AUDIT-2026-08-16.md` and remain open. An external auditor
should review each one and decide whether it is in scope.

| ID | Item |
|---|---|
| I-01 | This document is not an outside audit |
| H-01 | A young chain can be out-mined |
| L-01 | Light clients believe the node (partially addressed by PR #31, #34) |
| L-02 | Browser wallet stores seed in `localStorage` |
| L-03 | Worker is on the path of every web-wallet request |
| A-01 | Anonymity set is this block, on this network |
| G-01 | No cut-through (addressed by PR #28 in progress) |
| N-11 | Stem/fluff is not full Dandelion++ (addressed by PR #22 in progress) |
| N-12 | Tor falls back to clearnet |
| P-05 | Submit is origin |
| D-01 | Binaries are unsigned |

I-01 is the reason this document exists. All others are
candidates for the external audit scope.

## 5. Known gaps in verification

These are not vulnerabilities. They are structural gaps in the
verification process, which an external audit should be aware of.

- **No formal verification.** The supply invariant, the range
  proof system, and the signature schemes are tested by example,
  not proven. Zcash closed this gap for its Ironwood shielded
  pool in July 2026 (2,700+ theorems in Lean 4). Nightfall's
  Pedersen-based variant has no equivalent.
- **No third-party fuzzing campaign.** `cargo-fuzz` is not
  configured anywhere in the workspace.
- **No cross-implementation differential testing.** Two
  implementations of the same rule do not exist for the supply
  invariant. PR #23 provides an independent consensus oracle
  that partially closes this; PR #34 adds an independent
  header-chain verifier.
- **No external audit.** Which is the point of this document.

## 6. What an auditor needs before bidding

For a firm to produce a fixed-price quote, they need:

1. **Source baseline.** A commit hash and a tag. Currently
   `v1.0.5` at `main` head `41357431ba...`.
2. **Reproducible build.** `cargo build --release` produces the
   same binary from the same commit. A Dockerfile or a
   `rust-toolchain.toml` pinning the toolchain version would
   make this reproducible.
3. **Test entry points.** How to run the full test suite, the
   crypto-specific tests, and any integration tests. Currently
   `cargo test --workspace`.
4. **Test vectors.** Reference inputs and expected outputs for
   the cryptographic primitives. None exist today.
5. **Threat model.** Provided in this document, section 3.
6. **Prior audits.** Provided in this document, section 4.

Items 1, 3, and 6 exist today. Items 2, 4, and 5 do not.

## 7. Path to funding

For Nightfall, an audit is realistic without paying the full price.
Three known paths:

- **Audit contests** (Sherlock, Immunefi, Code4rena, Cantina):
  parallel reviewers, paid on result. Historically find more
  severe bugs per dollar than private engagements.
- **Grant programs** that cover 80-100% of audit cost for
  public-goods projects: Ethereum Foundation grants, Uniswap
  Security Fund, Polkadot Assurance Legion.
- **Bug bounties** after the audit: continuous coverage, pays
  only on confirmed findings.

This document is the input these applications require.

## 8. Open questions for maintainers

1. Which of the three internal documents should be treated as
   authoritative for the state of v5?
2. Is there a funded budget for an external audit, or is the
   goal to prepare the material first and seek funding later?
3. Should the audit scope be the crypto core alone (Phase A),
   or the whole workspace?
4. Are there specific areas the maintainers are already
   concerned about that should be highlighted to an auditor?

## References

- `docs/AUDIT-2026-08-12.md`
- `docs/AUDIT-2026-08-16.md`
- `docs/AUDIT-2026-09-08.md`
- `docs/SPEC.md`
- `docs/PRIVACY.md`
- `docs/ATTRIBUTES.md`
- PR #22 (Dandelion++ prototype, in review)
- PR #23 (independent consensus oracle, in review)
- PR #28 (cut-through authorization prototype, in review)
- PR #31 (light-client RFC, in review)
- PR #34 (independent header verifier, in review)
