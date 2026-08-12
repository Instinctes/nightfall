# Contributing

Patches welcome. A few things are worth knowing before you start.

## Ground rules

**Consensus changes need a test that fails without them.** Not "I ran it and it
looked fine" — an actual test that goes red when the change is reverted. Most
of the bugs in this codebase's history were found by tests that were written
*after* someone assumed the code was correct.

**Anything touching the supply invariant needs a very good reason.**
`Σ UTXO − Σ kernel_excess = (minted − burned)·G` is the one property that makes
a confidential chain trustworthy. If your change requires relaxing it, the
change is probably wrong.

**Never weaken `crates/nightfall-ledger/tests/exploit_regression.rs`.** Every
test in there is the inverse of a proof-of-concept that once worked against a
real build. If one starts failing, the chain is broken — fix the code, not the
test.

## Before opening a PR

```bash
cargo fmt --all
cargo clippy --workspace --all-targets    # must be clean, not "mostly clean"
cargo test --workspace
```

All three are enforced by CI, so running them locally just saves you a round
trip.

## Consensus-breaking changes

Anything that changes what a valid block looks like — the block body, a
signature message, PoW parameters, difficulty, the emission curve — splits the
network. Such a change must:

1. bump `PROTOCOL_VERSION` (and `WIRE_VERSION` if the wire format moved),
2. explain in `docs/DECISIONS.md` what changed and why,
3. update `docs/SPEC.md` in the same commit,
4. say plainly in the PR description that existing chains will not survive.

The genesis commitment covers the protocol version, so bumping it makes the old
and new chains mutually unreachable. That is the intended behaviour.

## Cryptography

Use the primitives that are already here: `curve25519-dalek`, `bulletproofs`,
`argon2`, `blake3`, `chacha20poly1305`. Do not hand-roll anything.

If you add a hash that feeds into consensus, use `hash_multi` with a fresh
domain constant. The length-prefixed encoding is what stops two different
inputs colliding by concatenation — plain `blake3(a || b)` is not equivalent
and will eventually bite.

## Style

The code is written to be read by someone deciding whether to trust it with
money. That means:

- Comments explain **why**, not what. `// increment counter` helps nobody.
- When a decision has a non-obvious trade-off, say what the trade-off is.
- When something exists because a specific bug happened, name the bug. Several
  comments in this codebase reference audit findings by number for exactly
  that reason.

## Testing philosophy

Unit tests for arithmetic and encodings. Integration tests for anything with
state. And where it matters, tests written from an **attacker's** point of
view — constructing the wire struct by hand rather than going through the
honest builder, because that is what an attacker does.

Three of the worst bugs in this project's history were invisible to unit tests
and only appeared when two real nodes were run against each other. If you are
touching P2P, persistence or chain selection, run two nodes.

## Areas that need work

| Area | Why it matters |
|------|----------------|
| **Cut-through** | The single biggest privacy gap. Needs a spend-authorisation scheme that does not publish a per-input signature. |
| **Dandelion++** | No network-layer privacy today. |
| **UTXO snapshots / headers-first sync** | Initial sync replays and re-verifies every block; memory-hard PoW makes that expensive. |
| **Merkle Mountain Range** | The UTXO root is recomputed per block, O(n log n). An MMR would make it incremental. |
| **Independent review of the cryptography** | The most valuable contribution anyone could make right now. |

## What will be turned down

- Anything adding a mint authority, freeze capability or admin key. Not negotiable.
- Premine, treasury or "ecosystem fund" allocations. The genesis is empty and stays empty.
- Optional privacy, or a transparent transaction path.
- Telemetry, analytics, or any phone-home behaviour, in the app or the website.

## Licence

Contributions are dual-licensed MIT or Apache-2.0, matching the project.
