# RFC: Long-Term Survival — Quantum Recovery and Regulatory Resilience

Status: Draft
Author: nightfall-wizard
Created: 2026-10-03

## Summary

Two dated, external threats will affect Nightfall within the next
twenty-four months and the next ten years respectively. Neither has
a documented response in this repository today.

This RFC proposes a phased path to address both without changing
consensus rules, block format, or transaction format in the first
step. It is documentation only.

## The two threats

### Threat 1 — EU privacy-coin restriction, July 2027

Regulation (EU) 2024/1624 enters into force on 10 July 2027. From
that date, licensed crypto-asset service providers in the EU may
not hold anonymity-enhancing coins. This affects Nightfall as much
as Monero and Zcash.

The regulation does not prohibit self-custody or peer-to-peer
transfers. It restricts regulated intermediaries. Nightfall has no
intermediaries by design, but its users will lose access to
regulated on-ramps and off-ramps unless there is a documented,
verifiable compliance path.

Nightfall already has the primitive the regulation asks for: the
website documents that a view key lets an accountant see every
amount and memo a user sends or receives, and is structurally
incapable of moving a coin. That mechanism is not currently
described in a form a regulator can audit.

### Threat 2 — Quantum break of ECDLP, next 5 to 15 years

All of Nightfall's cryptography rests on Curve25519:

- Pedersen commitments
- Schnorr signatures
- Bulletproofs
- X25519 for payload encryption

Zcash announced in May 2026 a quantum-recoverable wallet within one
month and full post-quantum status within 12 to 18 months. Monero
is moving toward Seraphis and Jamtis, which include post-quantum
properties. Ethereum targets 2029. Nightfall has no documented
plan.

## Non-goals

This RFC does not propose:

- changing the current consensus rules
- changing the current block or transaction format
- changing the current address format
- migrating to post-quantum cryptography in one step
- weakening any privacy property to satisfy a regulator

Those are separate concerns and require separate RFCs.

## Phased roadmap

### Phase 1 — Quantum-recoverable wallet format

A wallet-level mechanism by which the user pre-commits to a
post-quantum recovery public key at address generation time. If
ECDLP is broken, the user can prove ownership of unspent outputs
using the recovery key, without the current spend key.

Does not change consensus. Only changes what the wallet stores.

### Phase 2 — Selective-disclosure receipts

A standardized, auditable format for proving a single payment to a
third party without revealing the rest of the wallet. The website
already describes the underlying capability (view keys). What is
missing is a receipt format with a fixed structure that a
regulator or accountant can verify independently.

Does not change consensus. Operates at the wallet and RPC level.

### Phase 3 — Hybrid transactions

Transactions that carry both an ECDLP-based proof (for current
validators) and a post-quantum proof (for future validators). Old
nodes accept them because the ECDLP proof is valid. New nodes
accept them because the PQ proof is valid.

Soft-fork compatible.

### Phase 4 — Post-quantum commitments and range proofs

Replacement of Pedersen commitments and Bulletproofs with
lattice-based equivalents that preserve additivity and
confidentiality.

Requires a hard fork and a full UTXO-set migration.

### Phase 5 — Deprecation of the ECDLP path

After a migration window, the ECDLP-based consensus rules are
disabled and the chain runs entirely on post-quantum primitives.

## What this would make Nightfall uniquely

No privacy coin today has all four properties:

- confidential amounts and stealth outputs
- proof-of-work with a memory-hard function
- a locally verifiable supply invariant
- a documented, pre-committed path to post-quantum recovery and
  regulatory auditability

Zcash has the first and is migrating on the fourth but not the
second. Monero has the first and second. Neither has the third.
Nightfall has the first three and could add the fourth.

## Scope of this first PR

This PR adds only:

- `docs/RFC-LONG-TERM-SURVIVAL.md` (this document)

It does not add any code. It does not require any change to any
other file. A follow-up PR would implement Phase 1, but only
after this RFC has been reviewed and its direction agreed.

## Relation to existing work

None of the currently open PRs (#3 through #31) address
post-quantum recovery or regulatory resilience. This RFC is
orthogonal to all of them.

PR #31 (proof-carrying light client) and this RFC are both
documentation-only. They address different future concerns.

## Alternatives considered

**Do nothing.** Rejected. The EU deadline is 21 months away. The
quantum risk has a 5 to 15 year horizon and the migration itself
takes years. The cost of not having a plan exceeds the cost of
writing one.

**Migrate to post-quantum immediately.** Rejected. Post-quantum
primitives are larger, slower, and less battle-tested. A premature
migration sacrifices usability for a threat that is not yet
realized.

**Weaken privacy to satisfy regulators.** Rejected. That would
destroy Nightfall's reason to exist. Selective disclosure is the
correct direction: the user chooses what to reveal, and nothing
else is revealed.

**Copy Zcash's migration.** Considered as a reference, not as a
template. Nightfall's commitment scheme, signature scheme, and
privacy model differ enough that a direct copy is not possible.

## Open questions

1. Which post-quantum commitment scheme preserves the additive
   homomorphism the supply equation requires?
2. What is the smallest recovery-key format that fits into the
   current address structure without breaking backwards
   compatibility?
3. Can the existing view-key capability be standardized as a
   regulator-auditable receipt format without any protocol
   change?
4. What is the expected size increase for addresses, transactions,
   and the UTXO set after a full migration?
5. Which of Phases 1 through 5 should be funded first, given
   that Phase 1 and Phase 2 require no consensus change?

## References

- Regulation (EU) 2024/1624 (AML), effective 10 July 2027
- Zcash post-quantum announcement, May 2026
- Monero Seraphis and Jamtis research
- Ethereum "Lean" post-quantum roadmap, target 2029
- NIST PQC standards FIPS 203, 204, 205
- `docs/SPEC.md`
- `docs/PRIVACY.md`
- `docs/AUDIT-2026-08-12.md`
