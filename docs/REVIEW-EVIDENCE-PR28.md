# PR #28: Cut-through authorization review

Status: experimental, non-consensus prototype.
Not production ready.

The canonical v8 ledger, protocol version,
genesis, supply and existing transaction
validity rules remain authoritative.

## Required maintainer review

- [ ] Independently verify the Ristretto
      adaptation of LIP-0004.
- [ ] Validate receiver-only authorization.
- [ ] Verify Ko/Ki, commitment and kernel binding.
- [ ] Test rogue-key attacks.
- [ ] Test aggregate cancellation attacks.
- [ ] Test key substitution and replay attacks.
- [ ] Test duplicate and malformed inputs.
- [ ] Verify conservation of value.
- [ ] Exclude unauthorized spending and inflation.
- [ ] Review sidecar persistence and fsync behavior.
- [ ] Test crash and restart boundaries.
- [ ] Test failed reorg rollback.
- [ ] Verify pruning and retention invariants.
- [ ] Verify existing v8 consensus isolation.
- [ ] Obtain independent cryptographic review.
- [ ] Design wallet, mempool and RPC integration.
- [ ] Develop migration and activation proposal.

## Evidence

Linux CI provides reproducible regression tests
and static validation of important v8 constants.

Passing tests do not constitute a cryptographic
security audit.

## Explicit non-goals

No mainnet activation.
No protocol upgrade.
No chain reset.
No wallet transactions.
No mining changes.
No claim of production readiness.
