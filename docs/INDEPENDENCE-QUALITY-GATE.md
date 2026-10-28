# Nightfall Independence Quality Gate

## Scope

This is a Devnet-only acceptance gate for PR #19.

The existing Rust test harness starts real node processes with
isolated data directories and controlled loopback proxies.

No consensus, emission, wallet-key, or P2P wire change is introduced.

## Scenarios

1. Interrupt a real P2P session, verify isolation, reconnect,
   and confirm resynchronization.

2. Construct persistent competing chains, restore connectivity,
   and verify selection by cumulative work.

3. Remove the original node, allow surviving nodes to advance,
   restore the original node, and verify convergence.

## Acceptance

All three named tests must pass in three consecutive runs.

Missing tests, nonzero exits, incomplete output, and timeouts fail
the quality gate.

The runner drains subprocess output continuously, retains at most 2 MiB
in memory, and kills its isolated process group if the byte limit or
deadline is exceeded. No unbounded temporary log file is written.

The JSON receipt contains:

- Exact tested source commit
- Cargo.lock SHA256
- Test-file SHA256
- Rust toolchain versions
- Per-run results and durations

The receipt contains no wallet keys, credentials, or chain data.

## Reproduction

Run from the source repository:

    python3 scripts/check_multinode_independence.py --self-test

    cargo fmt --all --check

    python3 scripts/check_multinode_independence.py --repeats 3 --report /tmp/independence-evidence.json

## Limitations

All simulated operators execute on one physical host.

The tests use configured connections, not independent public
peer discovery.

They do not prove:

- Independent real-world node ownership
- Global network decentralization
- Payment reliability between unrelated operators
- Independent cryptographic security assurance
- Complete protection against network attacks

Network timing is nondeterministic.

Actual independence requires independently administered machines,
documented recovery exercises, and disposable Devnet payments.

Publish failures as well as successes.

## Monetary-state acceptance after healing

All three real-process Devnet scenarios additionally
verify monetary-state consistency after network recovery.

Acceptance conditions:

- Matching chain tips and cumulative work
- Matching UTXO roots
- Matching minted totals
- Matching burned fees
- Matching circulating supply
- Successful node supply-invariant verification
- Agreement between status and verify_supply RPC
- Stable chain tip throughout evidence collection

These checks exercise real persisted node state
across partitions, restarts and reorganizations.

They rely on the node's own validation implementation.

They do not establish an independent cryptographic
audit, mainnet correctness or actual operator diversity.
