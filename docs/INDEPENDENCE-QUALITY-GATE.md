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
