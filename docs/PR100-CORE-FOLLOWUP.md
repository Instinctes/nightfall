# PR #100 — Core follow-up

Status: proposed implementation; not release approval.

## Scope

1. Core recognition and journaled recovery for unsealed 1.1.x pruning.
2. Optional checkpoint descriptor, producer and fresh-genesis Core consumer.
3. Unknown historical counters after checkpoint import.

## Trust boundaries

- The manifest cannot supply an arbitrary host or download path.
- Snapshot bytes are size-bounded, SHA-256 checked, then independently
  authenticated against the compiled checkpoint by the storage importer.
- NIGHTFALL_NO_ASSUME_VALID retains the full-archive path.
- Proxy/privacy lockdown is unchanged.
- Existing wallet scan history cannot select checkpoint fast start.
- A live checkpoint adoption uses the existing fail-closed import journal.
- On a failed commit the node stops instead of mining on partial state.
- Legacy unsealed pruning never receives fabricated validation seals.
- Recovery requires explicit Core confirmation and retains a backup.
- Wallet keys and history are not deliberately deleted by recovery.
- Historical kernel and transaction counts are unknown, not zero.

## Release gates

- Independently review Rust compilation and all modified Core GUI code.
- Run real GUI recovery on macOS, Windows and Linux.
- Test interrupted writes, storage exhaustion and restart behaviour.
- Test Core checkpoint acquisition and catch-up on a real archive.
- Test fresh-wallet scan against the retained authenticated archive.
- Publish the real snapshot, release assets and deployed manifest.
- Obtain maintainer approval.

The contributor does not claim to have completed these release gates.

No consensus, emission, PoW or checkpoint pin changes.
