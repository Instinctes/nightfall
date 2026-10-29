# Short-page IBD release gate

This is a reviewability improvement for the short-page
synchronization fix in PR #79.

It is not a claim that mainnet has become faster.

## Why this gate matters

A peer may return fewer than 128 blocks because a response
has a byte limit.

Advancing the request cursor as though all 128 blocks
arrived can leave unfilled heights.

The real-node Devnet test requests those missing
successors through TCP.

It checks the resulting chain tip and monetary invariant.

## Acceptance conditions

- The actual Rust integration test must exist.
- Every run must execute the named test.
- Zero filtered tests cannot count as success.
- Three consecutive real-node Devnet runs must pass.
- Any timeout or non-zero exit fails closed.
- Excessive output fails closed.
- Evidence records the exact Git commit.
- Evidence records Cargo.lock SHA256.
- Evidence records the integration-test SHA256.
- Evidence contains no production keys or wallet state.

The runner is tested with normal completion,
failure, oversized output and a hanging child.

GitHub Actions uploads its bounded JSON report
even when a regression fails.

## Run

In Termux without compiling Rust:

    python3 scripts/check-ibd-release-gate.py --self-test

On a Rust-equipped host from the PR branch:

    python3 scripts/check-ibd-release-gate.py --repeats 3 --timeout 300

## Scope and limits

This covers a real isolated loopback Devnet
short-page integration test.

It does not benchmark network-wide IBD speed.

It does not test storage checkpoint imports.

It does not prove cryptographic soundness.

It does not authorize mainnet recovery.

Independent review and additional restart
and multi-peer tests remain necessary.

No changes to genesis, consensus, emission,
keys, RPC or chain storage.

No mainnet state is read or written.


### CI stdout regression (2026-10-29)

The Rust integration test itself passed, but the
Python gate initially rejected its output.

`--nocapture` allows the test's `eprintln!`
diagnostics to appear inside the harness status
line, defeating the strict `test ... ok` parser.

Use `--show-output` instead.

Rust reports the canonical single-test status
and then prints captured diagnostics for
passing tests.

The process exit code, test identity, output
limit and timeout are still checked.

This is a harness repair, not proof of
mainnet synchronization performance.
