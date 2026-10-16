# Ephemeral state: chain-anchored lifecycle + per-authority quota

## Summary

Adds the first adversarial-grade layer to Nightfall's ephemeral state:
cryptographic binding of non-consensus state to a canonical chain position,
plus a pre-authentication quota that stops a single signing key from
dominating the pool.

This is the consensus half of the ephemeral-state work. P2P wire exposure
and runtime admission are deliberately not included — they belong in a
follow-up PR so that this one stays reviewable.

## What changes

- `EphemeralChainAnchor { tip_hash, tip_height }` bound into
  `signing_message` and `scoped_key`.
- `EphemeralStatePool::propose_signed_on_chain` (canonicality + height match)
  and `reconcile_with_chain` (reorg drop/keep).
- `AuthorityQuotaExceeded` + `MAX_STATES_PER_AUTHORITY = 64`, checked
  **before** signature verification.
- `authority_of` reverse index, maintained at all four mutation points.
- `NodeInner::bump_tip` calls `reconcile_with_chain` on every canonical tip
  transition.
- `serde` on `EphemeralChainAnchor` and `SignedEphemeralProposal`.

## Why

A signed ephemeral proposal that is not bound to a specific canonical block
can be replayed across forks, resurrected after a reorg, or used to force an
unbounded stream of signature verifications. Committing the anchor into the
signature covers the first two; the pre-auth quota covers the third.

## Testing

- `cargo test -p nightfall-consensus --test ephemeral_state` → 45 passed
- `cargo test -p nightfall-node` → all green
- `cargo test -p nightfall-p2p` → all green

New tests cover:
- chain-bound admission, unanchored rejection, height mismatch
- reorg that removes the anchor (state dropped) vs. reorg above it (state kept)
- anchor tampering invalidating the signature
- wire roundtrip + serialized payload/anchor tampering
- per-authority quota flood (fills one authority, asserts refusal, asserts a
  second authority is unaffected)

## Out of scope

- P2P wire variant (`PeerMsg::EphemeralProposal`)
- Runtime-side admission arm
- ZK commitment of the ephemeral root into the block header

These will follow once the consensus semantics above have settled.
