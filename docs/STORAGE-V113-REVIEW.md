# Nightfall v1.1.3 - scoped storage review

## Scope

This draft isolates the storage changes from PR #17 and #27
on the official v1.1.3 release baseline.

No monitoring, agent payment signing, Merkle-sum research
or dedicated CI workflows are included.

The existing ci.yml and macos-latest remain unchanged.

## Authenticated snapshot v2

The protocol v8 UTXO root commits to the commitment,
output key and creation height, but not the entire Output.

Snapshot v2 retains complete original unspent Output
records, including range proofs, encrypted payloads and
the signed coinbase flag.

Every surviving Output must match the complete original
creation-block body.

The body must hash to its original body_root, committed
through the compiled checkpoint's header chain.

A modified output cannot be accepted merely because
the locally constructed UTXO metadata looks consistent.

This correctness-oriented approach may increase snapshot
size substantially. Mainnet-scale memory and timing
measurements are required before production use.

## Blocking release acceptance

The following work remains mandatory.

- Untouched genuine 1.1.2 operator pruned datadir test.
- Restart and migration refusal validation.
- Independently verified Mainnet checkpoint at 505000+.
- Mainnet-sized snapshot size and startup benchmarks.
- Real export, import and network catch-up.
- Matching UTXO root, kernel sum, supply and total work.
- Interrupted multi-file import and restart testing.
- Core wallet scanning below first_height.
- Apple Silicon and other cross-platform regressions.
- Maintainer security review and explicit approval.

IMPORTANT:

Restored Core wallets with birth height zero still need
a durable authenticated candidate scan path below
first_height.

That integration is NOT implemented by this draft.

Therefore checkpoint bootstrap is NOT production-ready
and this PR must not be merged as a finished feature.

No live Mainnet, wallet, mining data or user datadir
was accessed or modified by the preparation script.
