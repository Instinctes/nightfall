# Confidential Merkle-sum evidence: isolated research v1

**Status:** Implemented research primitive, not consensus-active, not a deployable
supply certificate. Paradigm Creator is an architectural ambition here, not an
assertion of worldwide novelty or cryptographic assurance.

## Motivation and scope

The Roadmap calls for incrementally authenticated state and independent
verification. The Whitepaper and RFC #33 propose a Merkle-sum construction.
This implementation contributes *the first independently testable primitive*
without modifying the canonical UTXO root, consensus, protocol, genesis,
headers, supply counters, mining, wallet files or network.

- Canonical ordering by compressed commitment; duplicate commitments rejected.
- Domain-separated hashes over leaf commitment plus output public key,
  creation height, coinbase flag, and child hashes and subtree sums.
- Exact Ristretto commitment aggregation with invalid-encoding rejection.
- Deterministic padding for odd trees, including enforced empty siblings.
- Bounded depth/leaf count and all-level membership proof verification.
- A separate conservation equality check that *only* checks supplied values.
- Adversarial tests: leaf metadata mutation, sibling mutation, altered count,
  index, path truncation, duplicate input, deterministic order and wrong supply.

**Not supplied:** non-membership, complete UTXO authentication, range proof
verification, ownership verification, authenticated kernel sum, authenticated
minted/burned counters, independently validated headers or consensus anchoring,
proof of historical state transition, pruning, reorg integration, or a succinct
proof of global supply. This is not a general-purpose zk proof or SNARK.

In particular, the active Nightfall v8 block-header UTXO root is *different*.
A verifier accepting this root from the same untrusted RPC operator it is
supposed to distrust has gained **no** independent authentication. The
conservation equality checked against attacker-supplied sums is not evidence
of no inflation.

## Review / execution

    cargo test -p nightfall-ledger confidential_state_sum
    cargo fmt --all -- --check
    cargo clippy -p nightfall-ledger --all-targets -- -D warnings
    cargo test -p nightfall-ledger

Only source code and deterministic test vectors are committed. No secrets,
real nodes, mainnet state, network access, or third-party endpoints are used.

## Future prerequisites (separate decisions)

1. Independent review of leaf metadata, sum algebra, serialization and
   allowed canonical input sets.
2. Proof of absence/non-membership and incremental update/reorg semantics.
3. Compare results against the authoritative ledger on pathological forks.
4. Authenticated and versioned root commitment, requiring a consensus design
   and an explicit maintainer-approved activation plan.
5. Independently established kernel and issuance evidence plus light-client
   header verification; hostile-server tests.

This PR must not be cited as completing RFC #33, audits, assurance, or Roadmap
independent verification.
