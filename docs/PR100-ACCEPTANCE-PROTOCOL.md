# PR #100 - Storage acceptance protocol

Baseline: official Nightfall v1.1.3.

## Automated tests

- Checkpoint and pruned-state integrity.
- Serialized checkpoint restart and catch-up.
- Interrupted multi-file import refusal.
- Interrupted import with retained scan archive.
- Corrupt, missing and nonregular scan archive refusal.
- Core-wallet recovery integration.
- Ordinary workspace CI; no dedicated workflows.

## Real Mainnet acceptance

The ignored Rust integration test
pr100_real_checkpoint_acceptance requires a genuine
complete offline Mainnet archive.

It validates:

- Compiled checkpoint at height 505000 or later.
- Real authenticated snapshot export.
- Snapshot size and elapsed time.
- Import into a fresh separate datadir.
- Regular consensus validation of all later blocks.
- Matching tip hash, UTXO root, kernel sum, supply,
  total work and block count.
- State equality after persistence and restart.
- Peak memory where supported.

No Devnet simulation counts as Mainnet acceptance.

## Historical migration

A genuine untouched 1.1.2 operator pruned datadir
must be independently supplied and validated.

Its refusal and migration procedures must be tested
on a disposable copy, including exact preservation
of wallet/miner identity.

Synthetic fixtures cannot prove this requirement.

## Release blockers

Full desktop GUI restoration, real operator migration,
Mainnet-scale measurements, interrupted import
rehearsal on real data, full CI, additional pruning
review and maintainer security approval are required.

No live Mainnet state, mining or wallet activation.
No automatic merge.
