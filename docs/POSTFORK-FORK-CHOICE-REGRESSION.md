# Post-500,000 Fork-Choice Regression

This test-only contribution extends PR #10.

It does not modify:
- Production consensus
- Mining
- Wallets
- P2P wire format
- Monetary emission
- Checkpoint trust assumptions

## Motivation

NIGHTFALLCOIN activated a consensus upgrade at
mainnet block 500,000.

A shorter competing chain may carry greater
cumulative proof of work.

Height alone is not a valid fork-choice rule.

PR #10 addresses the production behavior.
These tests strengthen its regression coverage.

## Verification

The new tests cover:

1. Reorg limits across activation height 500,000.
2. Exhaustive synthetic ancestor-search cases.
3. Logarithmically bounded ancestor discovery.
4. Failed peer responses and fail-closed behavior.
5. Correct classification of shorter divergent peers.
6. Boundary arithmetic near u64::MAX.

The existing real-TCP integration test supplies
the actual two-node proof-of-work exercise.

Run:

    cargo fmt --all -- --check

    cargo test --locked -p nightfall-node \
      --lib post_activation_fork_choice_property_tests \
      -- --test-threads=1

    cargo test --locked -p nightfall-node \
      --test shorter_heavier \
      -- --test-threads=1

## Limitations

These tests do not prove that the live mainnet
is valid or independently decentralized.

They do not verify cryptographic supply proofs
from an untrusted external node.

An independent security audit remains necessary.
