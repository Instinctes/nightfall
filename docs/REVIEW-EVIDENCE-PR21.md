# PR #21: maintainer review / authenticated UTXO Chain shadow

Status: research integration, **not a consensus activation**.
Parent dependency: #20 (authenticated UTXO shadow model).

## Why this exists
The Whitepaper describes confidential settlement with verifiable supply;
the Roadmap prioritizes independently repeatable recovery and reorg tests.
This PR integrates a non-authoritative authenticated view of the canonical
UTXO set into block processing, replay, pruning and reorg paths.

## Acceptance matrix for a reviewer
- [ ] Review #20's sparse-tree model and soundness of membership commitments.
- [ ] Confirm the canonical UTXO set, not the shadow, decides block validity.
- [ ] Check successful block transitions agree with independently rebuilt roots.
- [ ] Inject a shadow write failure: no partially committed tip, UTXO,
      work, headers, supply, pruning horizon, or shadow state may remain.
- [ ] Force a bad candidate during a pruned reorg: active chain unchanged.
- [ ] Restart a pruned datadir from a saved horizon and replay retained blocks.
- [ ] Review any full-rebuild cost on a production-size chain.
- [ ] Compare v8 validation, block bytes, protocol/wire versions and genesis
      against the authoritative mainnet code: no unreviewed semantic drift.
- [ ] Obtain Linux CI results; independently repeat on disposable Devnet data.

## Release conditions / blockers
- Review #20 first; while #20 is unmerged, #21 is a stacked change.
- No claim of independent cryptographic audit or crash-consistent persistence.
- Permanent authenticated-state generations and forced power-loss testing
  require separate engineering and an independent review decision.
- A green CI check is evidence of tests, **not** evidence of correctness.

## Scope and safety
No network upgrade, chain reset, genesis replacement, mining-state migration,
wallet-secret handling, or change to the canonical UTXO root is requested.
Ready for review is not permission to merge or activate on mainnet.
