# RFC: Proof-Carrying Light Client

Status: Draft
Author: nightfall-wizard
Created: 2026-10-03

## Summary

A design for a Nightfall light client that verifies its own chain
position, its own wallet state, and the global confidential supply
locally, without a full node and without trusting an RPC or seed
server for these specific claims.

This RFC is documentation only. It proposes a phased roadmap and
defines a single, small first implementation step. It does not
change the consensus rules, the block format, or any existing
crate.

## Problem

The official website states, verbatim:

> "a node supplies chain data"

and

> "the website displays a node's reported result"

That is the current trust model of the web wallet. It is a
deliberate, documented trade-off, not an oversight. But it means a
malicious or compromised node can misrepresent:

- the current chain tip and its work
- whether a specific output exists or was spent
- the wallet's total balance
- which outputs belong to the wallet (selective omission)
- the global minted/burned supply

The Nightfall core wallet protects itself against this by running a
full node. The web wallet does not, and cannot, given its target
environment.

## Non-goals

This RFC explicitly does not propose:

- changing the consensus rules
- changing the block header format
- changing the transaction format
- changing the UTXO commitment scheme
- implementing a full node in WASM
- replacing the existing web wallet in one step

Those are separate concerns and require separate RFCs.

## Threat model

The client assumes:

- the Nighthash-v2 PoW function is as documented
- the Bulletproof, Pedersen, and Schnorr primitives are sound
- an adversary controls the node(s) the client talks to

The client defends against:

- a node that reports a false chain tip
- a node that reports a false difficulty or work total
- a node that presents a valid-looking but non-canonical header chain
- a node that claims an output exists when it does not
- a node that omits an output the wallet actually owns
- a node that reports a supply figure inconsistent with the chain

The client does not defend against:

- network-level censorship (a node that refuses to answer at all)
- eclipse attacks on the client's peer discovery
- a compromise of the user's device or browser
- side-channel attacks on the signing key

Censorship remains possible. Forgery of accepted state does not.

## Trust boundary

| Claim | Current web wallet | This RFC's target |
|---|---|---|
| Chain tip | trusted from node | self-verified |
| Chain work | trusted from node | self-verified |
| Output existence | trusted from node | verified via UTXO membership |
| Output spentness | trusted from node | verified via UTXO non-membership |
| Wallet balance | trusted from node | locally recomputed from verified outputs |
| Scan completeness | trusted from node | verified via scan commitment |
| Global supply | trusted from node | verified against confidential commitment |

The last four rows depend on work that is not in this RFC. See
"Phased roadmap" below.

## Phased roadmap

The full property is only reached after several independent pieces
exist. Each phase is intended to be a separate PR, reviewable on
its own, and mergeable without waiting for the later phases.

### Phase 1 — Header chain and work verification (this RFC)

A standalone verifier that accepts a sequence of block headers and
returns either Verified or a specific cryptographic error. It
re-checks:

- header hashing under the canonical domain separation
- Nighthash-v2 PoW against the declared difficulty
- difficulty transition rules
- chain ID and genesis binding
- cumulative work

It does not read the UTXO set, the supply counters, or any wallet
state. It imports nothing from `nightfall-crypto`, `nightfall-ledger`,
or `nightfall-node`; it is a fully independent implementation.

This is deliberately the smallest piece that establishes the
trust-boundary shift. It can be validated against the existing
consensus oracle from PR #23.

### Phase 2 — UTXO membership and non-membership

Verification that a specific output exists in the UTXO set (or does
not), against an authenticated state root. This depends on the
authenticated UTXO structure from PRs #20 and #21. It cannot be
started until those are merged, because the root format they define
is what the proof verifies against.

### Phase 3 — Scan completeness

A mechanism by which the client can prove that, for a given block
range, it was shown every output that could belong to its scan key.
Without this, a node can silently omit a legitimate incoming output.
This likely requires a new commitment in the block header.

### Phase 4 — Merkle-sum UTXO commitment

A UTXO tree whose internal nodes commit both to their children and
to the aggregate Pedersen commitment sum of their subtree. The
root would then bind content, structure, and total confidential
value in one hash. This is the mechanism by which Phase 1's chain
verification and Phase 2's membership proofs jointly imply a
verifiable supply certificate. It requires a consensus change.

### Phase 5 — Proof-carrying wallet checkpoints

A small serialized wallet state that records: chain anchor,
UTXO anchor, supply anchor, scan position, and reorg horizon. On
restart, the wallet verifies forward from the previous checkpoint
rather than rescanning from genesis.

### Phase 6 — Adversarial light server

A test harness that presents deliberately false statements (wrong
tip, wrong balance, omitted input, fabricated output, false
confirmation, deep reorg, false supply, split view) to the verifier,
and asserts that the verifier rejects each one. This is the
acceptance test for the whole stack. It depends on the multi-node
fault harness from PR #19.

## Scope of this first PR

This PR adds only:

- `docs/RFC-LIGHT-VERIFY.md` (this document)

It does not add any code. A follow-up PR will add
`crates/nightfall-light-verify` implementing Phase 1, but only
after this RFC has been reviewed and its direction agreed.

Rationale: at the time of writing, the repository has 27 open pull
requests and no merged contributions from any contributor. Adding
a 28th PR of untested new code without an agreed direction would
add review load without adding reviewable value. A design document
can be reviewed in ten minutes and either accepted, rejected, or
redirected without wasted implementation effort.

## Relation to existing work

| PR | Title | Relation |
|---|---|---|
| #3 | storage: harden persisted chain integrity | Independent |
| #4 | test(node): real multi-node reorg coverage | Foundation for Phase 6 |
| #5 | test(consensus): reorg depth boundary | Header-chain reference |
| #6 | test(storage): reorg reload state checks | Independent |
| #7 | node: clean shutdown lifecycle | Independent |
| #8 | node: multi-node reorg convergence | Foundation for Phase 6 |
| #9 | node: multi-peer reorg liveness | Independent |
| #10 | fix(p2p): evaluate shorter chains by work | Relevant: work comparison |
| #11–#16 | p2p hardening | Independent |
| #17 | storage: authenticate pruned state | Adjacent to Phase 5 |
| #18 | storage: crash-consistent archive appends | Independent |
| #19 | test(node): multi-node fault harness | Foundation for Phase 6 |
| #20 | RFC: authenticated UTXO shadow state | Blocking dependency for Phase 2 |
| #21 | integrate authenticated UTXO shadow into Chain | Blocking dependency for Phase 2 |
| #22 | Dandelion++ transaction relay | Independent |
| #23 | test: independent consensus oracle | Reference verifier for Phase 1 |
| #25 | wallet: reject transfers exceeding input limit | Independent |
| #26 | storage: truncated binary length prefixes | Independent |
| #27 | storage: checkpoint-state bootstrap | Adjacent to Phase 5 |
| #28 | ledger/storage: cut-through-compatible spend authorization | Independent |
| #29 | Ephemeral state: chain-anchored lifecycle | Independent |

## Testing strategy

Phase 1 will be tested against:

- the consensus oracle from PR #23, using identical header sequences
- synthetic chains with deliberately wrong PoW, wrong difficulty
  transitions, wrong chain ID, and wrong genesis hash
- long orphan reorgs of the same total work
- header sequences that are individually valid but cumulative-work
  inconsistent

Phase 6 will build on the multi-node fault harness from PR #19.

## Alternatives considered

**Trust the node (status quo).** Documented on the website. Rejected
because it makes the web wallet unsuitable for users who cannot run
a full node, which is the majority.

**SPV only.** A Merkle-block SPV client verifies inclusion in blocks
but not that the headers represent the heaviest work chain, and it
has no mechanism for confidential-supply verification. Insufficient
on its own.

**Full node in the browser.** The chain size and the Nighthash-v2
memory cost per header make this impractical on mobile browsers.
Rejected.

**FlyClient alone.** FlyClient provides probabilistic sampling of
chain work. It is a building block for Phase 1, not a complete
solution, and it does not address wallet state, scan completeness,
or supply.

**Rely entirely on ZK proofs of chain validity (Mina-style).** A
fundamentally different trust model, requiring a redesign of the
block structure and a new proof system. Out of scope here.

## Open questions

1. Which exact Nighthash-v2 difficulty transition rule is
   authoritative, and where is it defined in the codebase?
2. Should the verifier accept a pruned header chain (checkpoint +
   subsequent headers), and if so how is the checkpoint itself
   authenticated?
3. What is the smallest useful proof bundle the verifier can accept
   over a mobile connection?
4. What is the intended relationship between the consensus oracle
   (PR #23) and this verifier — independent reimplementation for
   redundancy, or shared reference vectors?

## References

- `docs/SPEC.md`
- `docs/PRIVACY.md`
- `docs/AUDIT-2026-08-12.md` (findings C-01, S-01)
- `crates/nightfall-ledger/src/utxo.rs` (supply invariant)
- `crates/nightfall-crypto/src/pow.rs` (Nighthash-v2)
- `crates/nightfall-consensus/src/lib.rs` (chain, work, reorg)
- PR #20, #21 (authenticated UTXO state)
- PR #23 (independent consensus oracle)
