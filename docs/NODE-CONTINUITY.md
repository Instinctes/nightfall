# Nightfall Node Continuity Guard

Read-only longitudinal verification of local
Nightfall node-evidence claims.

This tool is not an independent consensus verifier.

It cannot prove that a node is independently operated.

A SHA-256 checksum detects accidental state changes.
It is not an authentication signature.

## Purpose

Preserve the last healthy observed chain reference
across repeated node checks.

Detect:

- decreasing chain height
- decreasing cumulative work
- conflicting tips at the same height
- rising height without rising cumulative work
- network identity changes
- unhealthy node-evidence reports

## Usage

From the repository root:

    python3 scripts/node_continuity.py --port 17881

The node must already be running with local RPC.

The tool does not start or stop the node.

The anchor is stored separately:

    ~/.nightfall-continuity/anchor.json

The anchor file is written with mode 0600.

## Results

BASELINED:
First healthy observed reference.

UNCHANGED:
The observed tip and cumulative work match the anchor.

ADVANCED:
Healthy forward progress; anchor updated.

CAUTION:
An anomaly was observed.
The last healthy anchor is preserved.

## Safety

A shorter chain with greater work can be a
legitimate reorganization.

The guard reports the event.
It never performs automatic fork recovery.

It never:

- resets a blockchain
- deletes historical blocks
- changes a miner seed
- modifies a wallet
- sends a transaction
- restarts mining
- rewrites consensus rules

The evidence contains local RPC claims only.

## Offline input

Existing node-evidence v1 receipts can be ingested:

    python3 scripts/node_continuity.py --receipt receipt.json

Receipts must be supplied in chronological order.
The format does not contain trusted timestamps.

## Dry run

    python3 scripts/node_continuity.py --dry-run

## Exit codes

0: Healthy observation

1: Caution

2: Error or invalid evidence

## Regression tests

    python3 -m unittest discover -s scripts -p 'test_node_continuity.py' -v

All test fixtures are synthetic and offline.
