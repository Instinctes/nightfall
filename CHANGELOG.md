# Changelog

All notable changes to Nightfall are documented here.

## Unreleased

### Added

- **Ephemeral state: chain-anchored lifecycle.**
  Non-consensus ephemeral state (`EphemeralStatePool`) is now cryptographically
  bound to a canonical chain position. A `SignedEphemeralProposal` carries an
  `EphemeralChainAnchor { tip_hash, tip_height }` that is committed into both
  the signing message and the scoped state key, so a proposal cannot be
  replayed across forks or resurrected after a reorg.

- **`EphemeralStatePool::propose_signed_on_chain` and `reconcile_with_chain`.**
  Network-facing admission checks the anchor against the node's actual
  canonical chain. A reorg only drops state whose anchored block disappeared;
  extensions above the anchor are preserved.

- **Per-authority admission quota.**
  `MAX_STATES_PER_AUTHORITY = 64`. The check runs *before* signature
  verification, so a single signing key cannot force an unbounded stream of
  distinct-state signature checks or monopolise pool capacity. Enforced via a
  new `authority_of` reverse index kept in lock-step with `entries` at every
  mutation point (insert, conflict replacement, expiry, reorg reconciliation).

- **Node tip hook.**
  `NodeInner::bump_tip` now runs `EphemeralStatePool::reconcile_with_chain`
  on every canonical tip transition (peer block, IBD extension, reorg,
  locally mined block, disk replay), so ephemeral state cannot drift out of
  sync with the chain through a less-travelled code path.

- **Wire-roundtrip safety.**
  `EphemeralChainAnchor` and `SignedEphemeralProposal` are `serde::Serialize`
  / `Deserialize`. Adversarial tests confirm that transport-level tampering
  with either the payload or the anchor is rejected by the existing
  signature and canonicality checks.

### Tests

- 45 tests in `crates/nightfall-consensus/tests/ephemeral_state.rs`, including
  chain-bound admission, reorg drop/keep semantics, authority quota flood,
  wire roundtrip, and serialized-tampering cases.
