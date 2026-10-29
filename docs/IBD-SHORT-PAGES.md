# Fast Sync: bounded P2P page recovery

## Problem

GetBlocks requests up to 128 blocks.

However, P2P responses must also respect
the maximum serialized message size.

A valid response can therefore contain
significantly fewer than 128 blocks.

Previously, advancing the request cursor
by 128 could create artificial gaps.

The node then requested more distant pages
before recovering the missing successor.

The fork suffix downloader also stopped
when a valid response was shorter than 128,
even when the peer reported more blocks.

## Changes

1. Recover the exact missing successor
   after successfully applying a short page.

2. Preserve ordinary parallel page claims
   for full responses.

3. Preserve bounded out-of-order buffering.

4. Continue fork suffix pagination
   across valid short responses.

5. Preserve all existing block validation,
   chainwork selection and safety rules.

## Consensus compatibility

No change to:

- Protocol version
- Wire version
- Genesis
- Proof of work
- Difficulty
- Supply invariant
- Wallet format
- Chain storage format
- Coinbase rewards
- Checkpoint policy

No untrusted peer state is accepted
without the existing validation path.

## Tests

Run:

cargo test --locked -p nightfall-node --lib ibd_claim_tests

cargo test --locked -p nightfall-p2p --lib

## Performance validation

A real benchmark requires two disposable
Devnet nodes.

Measure identical workloads before and
after the patch.

Record:

- Downloaded block count
- Short response count
- Total synchronization time
- CPU and RAM usage
- Final verified tip
- Reorganization behavior

Test short responses, multiple peers,
out-of-order delivery and restarts.

No seconds-to-sync guarantee is made
without measured results.

## Future work

Authenticated state snapshots,
persistent local state checkpoints
and headers-first initial synchronization
remain separate engineering projects.
