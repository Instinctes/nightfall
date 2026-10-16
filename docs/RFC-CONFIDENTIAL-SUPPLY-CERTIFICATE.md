# RFC: Confidential Supply Certificate

Status: Draft
Author: nightfall-wizard
Created: 2026-10-03

## Summary

A design for a verifiable supply certificate that allows any light
client, web wallet, or independent auditor to prove that Nightfall's
global supply invariant holds, without downloading the full chain and
without trusting any node.

This RFC is documentation only. It describes a mechanism and a
roadmap. It does not change consensus, block format, or transaction
format in the first step.

## Problem

Nightfall's core security property is the supply invariant:

    Σ UTXO − Σ kernel excess = (minted − burned) · G

Every full node re-checks this equation on every block. The website
documents this as one of the three foundational claims:

> "Every node re-checks that no coin exists which was never mined."

But only a full node can perform this check. A web wallet, a light
client, or an external auditor cannot verify the invariant without
downloading and validating the entire chain. The website documents
this trust boundary explicitly:

> "Run your own node to check the chain; the website displays a node's
> reported result."

This means that for the majority of users – those who use the web
wallet or a light client – the supply invariant is trusted, not
verified.

## Why this is fundamental

The supply invariant is the only guarantee against inflation. If it
is not independently verifiable, it is only as strong as the trust
placed in a node.

The same trust problem applies to:

- an exchange that wants to verify NIGHT's supply before listing
- a regulator that wants to confirm no hidden minting has occurred
- a light client that wants to display a verified supply figure
- a bridge that needs to prove the amount of NIGHT locked on the
  source chain

None of these can be satisfied today without a full node.

## Non-goals

This RFC does not propose:

- changing the current consensus rules
- changing the current block or transaction format
- changing the current UTXO commitment scheme
- implementing a full Merkle-sum tree in the first step
- replacing the full-node supply check

Those are separate concerns and require separate RFCs or
implementation work.

## Threat model

The verifier is assumed to be a light client with:

- a chain header it has already validated (see RFC #31 for the header
  and work verification phase)
- a small amount of local storage
- the ability to request proofs from one or more untrusted nodes

The adversary is assumed to control:

- all nodes the verifier talks to
- the network between the verifier and those nodes

The verifier must not accept a false supply figure from any
combination of adversarial nodes.

## Proposed mechanism

The mechanism has three layers.

### Layer 1 – Merkle-sum tree over the UTXO set

A Merkle tree in which each internal node commits to:

- its left and right children (as in a standard Merkle tree)
- the aggregate Pedersen commitment sum of its subtree

The root of this tree is a single 32-byte value that binds content,
structure, and total confidential value.

This is a change to the UTXO commitment scheme and requires a hard
fork. It is a long-term goal, not part of this first step.

### Layer 2 – UTXO membership and non-membership proofs

Given a Merkle-sum root, a node can provide:

- a membership proof that a specific output exists
- a non-membership proof that a specific output does not exist
- an aggregate proof that the sum of all leaves equals the root's
  committed sum

This depends on the authenticated UTXO shadow from PRs #20 and #21.
It cannot be implemented until those are merged.

### Layer 3 – Supply certificate

A serialized certificate that contains:

- the current chain tip header
- the current UTXO Merkle-sum root
- the kernel accumulator sum
- the minted and burned counters
- a proof that the root's committed sum equals the expected
  `(minted − burned) · G`

A light client can verify the certificate against its own validated
header chain and the kernel sum, without downloading the UTXO set.

## Phased roadmap

### Phase 1 – RFC (this document)

Define the certificate format and the security properties. No code.

### Phase 2 – Merkle-sum tree prototype

Implement the tree in `crates/nightfall-light-verify` as a
standalone prototype, without integrating it into the consensus.
Test it against the existing UTXO set snapshots.

Depends on: nothing.

### Phase 3 – UTXO proof integration

Integrate the tree into the authenticated UTXO shadow from PRs #20
and #21. This requires those PRs to be merged first.

Depends on: PRs #20 and #21.

### Phase 4 – Consensus integration

Add the Merkle-sum root to the block header and enforce it in
consensus. This requires a hard fork.

Depends on: Phase 2 and Phase 3.

### Phase 5 – Light-client verification

Expose the supply certificate via RPC and integrate it into the
web wallet and mobile wallet.

Depends on: Phase 4.

## Scope of this first PR

This PR adds only:

- `docs/RFC-CONFIDENTIAL-SUPPLY-CERTIFICATE.md` (this document)

It does not add any code. It does not depend on any other PR being
merged. It does not block any other PR.

A follow-up PR would implement Phase 2, but only after this RFC has
been reviewed and its direction agreed.

## Relation to existing work

| PR | Relation |
|---|---|
| #20, #21 | Provide the authenticated UTXO structure that Phase 3 uses |
| #23 | Independent consensus oracle – a reference verifier for the supply invariant |
| #27 | Checkpoint bootstrap – adjacent to light-client sync |
| #31 | Light-client RFC – Phase 4 of that RFC is exactly this proposal |

PR #31 already lists "Merkle-sum UTXO commitment" as its Phase 4.
This RFC expands that phase into a standalone design with its own
roadmap and security analysis.

## Alternatives considered

**Do nothing.** Rejected. Without a verifiable supply certificate,
the web wallet and light clients must trust a node for the most
important security property of the chain.

**Trust a node but verify signature.** Rejected. A malicious node
can lie about the supply without any signature being invalid.

**SNARK proof of the full supply equation.** Considered. A
recursive SNARK could prove the invariant, but it requires a
trusted setup or a transparent proof system, and it is
computationally expensive on mobile. The Merkle-sum approach
is simpler and does not require a new proof system.

**Publish a periodic signed supply statement.** Rejected. A signed
statement is a promise, not a proof. It does not eliminate the
need for trust.

## Open questions

1. Which hash function should the Merkle-sum tree use? Blake3 is
   already used throughout Nightfall.
2. How large is a membership proof for a tree with ~1.7 million
   leaves (current UTXO set size)?
3. Should the certificate include a timestamp or a block height, and
   how should the verifier handle rollbacks?
4. How does the certificate interact with pruned nodes and
   checkpoint bootstrap (PR #27)?
5. What is the smallest useful certificate that fits into a single
   QR code or a mobile push notification?

## References

- `docs/SPEC.md`
- `docs/PRIVACY.md`
- `crates/nightfall-ledger/src/utxo.rs` (supply invariant)
- PR #20, #21 (authenticated UTXO state)
- PR #23 (independent consensus oracle)
- PR #31 (proof-carrying light client)
- nightfallcoin.org (current trust boundary documentation)
