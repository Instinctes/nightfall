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

## Byte-integrity handoff (storage-only regression)

The importer hashes the exact bytes consumed by JSON deserialization and
cryptographically compares them to the fsynced scan-archive staging file.
A same-length source replacement between authentication and the durable
copy is rejected, with the interrupted-import journal retained. Tests
cover accepted identical bytes and rejected equal-length mutation.

This closes a copy-time TOCTOU gap. It is **not** a substitute for a
genuine independently checked Mainnet archive, offline operator data,
Core GUI restoration, representative benchmarks or maintainer approval.
No checkpoint, emission, network or wallet format changes are made.

<!-- pr100-termux-offline-evidence-gate-v1 -->

## Offline production evidence runner

Self-test:

`python3 -m unittest discover -s scripts -p test_pr100_production_gate.py -v`

Evidence run:

`python3 scripts/pr100_production_gate.py`

The runner never accesses the live node or its wallet.

It accepts disposable offline copies under:

- `~/nightfall-pr100-fixtures/untouched-1.1.2-pruned/`
- `~/nightfall-pr100-fixtures/complete-mainnet-archive/`

The 1.1.2 test only proves unforced CLI refusal and byte
preservation. It does not establish fixture provenance,
successful migration or release acceptance.

The Mainnet test calls the existing ignored Rust archive
acceptance harness. It compares state after real checkpoint
export, import, ordinary catch-up and restart.

Missing inputs and insufficient resources are BLOCKED,
never silently interpreted as passing.

Fixture SHA256 fingerprints and raw logs stay local.

Independent checkpoint certification, real-data import
fault rehearsal, Core desktop GUI recovery, representative
cross-platform benchmarks and final security approval
remain separate, mandatory production requirements.

No changes to consensus, monetary rules, genesis,
mining, live datadirs or dedicated CI workflows.

No automatic Mainnet release or upstream merge.


<!-- pr100-evidence-consistency-v2 -->
## Acceptance receipt consistency

The offline evidence runner rejects duplicate or contaminated success
markers, malformed or negative values, oversized snapshots and
mathematically inconsistent checkpoint / tip / catch-up counts.
Adversarial Python tests are part of the existing workspace CI.
A passing local receipt is NOT a Mainnet production sign-off, independent
checkpoint review, genuine legacy migration or desktop recovery proof.

<!-- pr100-fingerprint-failclosed-v1 -->
## Offline fixture tree identity hardening

The local acceptance fingerprint now refuses unreadable subtrees instead of
silently skipping them. It rejects symbolic links, hardlinks, special files,
cross-filesystem subtrees, identity-changing file replacements and observed
concurrent mutations. Empty directories affect the tree digest. Four offline
regressions cover refusal and tree identity. The evidence remains local.

This is a local safety improvement, **not** certification of fixture origin,
independent checkpoint authenticity, successful Mainnet migration, desktop
wallet recovery, production readiness or permission to merge.
