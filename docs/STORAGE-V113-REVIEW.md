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

Experimental fresh-wallet recovery is implemented below; full acceptance remains pending.

Therefore checkpoint bootstrap is NOT production-ready
and this PR must not be merged as a finished feature.

No live Mainnet, wallet, mining data or user datadir
was accessed or modified by the preparation script.

## Experimental checkpoint wallet recovery

The checkpoint import now keeps a bounded durable copy of
the complete snapshot as a scan archive under the same
interrupted-import journal.

The archive is reverified against the compiled checkpoint
and compared with the node's live horizon before a fresh
Core wallet consumes the unspent Outputs below first_height.

Only fresh unscanned wallets qualify.

The saved checkpoint hash anchors the first following body
by prev_hash. Later pages use ordinary block overlap.

Legacy wallets with unverifiable observations still refuse
rather than silently losing their payment history.

Additional pruning beyond the compiled checkpoint also
refuses until a new authenticated horizon scan is designed.

Missing or corrupted scan archives fail closed.

The retained archive may be large and authentication expensive.

This is still a draft.

No genuine 1.1.2 Mainnet operator fixture, Mainnet-sized
benchmark, or restored Core GUI end-to-end acceptance run
has been completed.

Release must not be merged solely on unit-test success.

## Android/Termux validation limitation

Local checkpoint tests: 12 passed.
Local foreign-validation tests: 5 passed.
Local wallet tests: 168 passed, 2 failed, 1 ignored.

Both failures originate in unchanged Vault security tests
calling fs::hard_link on the Android filesystem.

These failures are NOT counted as successful tests.
The full suite must pass in upstream GitHub CI.

Desktop Core compilation, Mainnet end-to-end recovery,
operator fixtures and maintainer approval are pending.
