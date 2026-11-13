# PR #98 — Maintainer review handoff

## Scope and provenance

Base: official NightfallCoin v1.1.3 commit
`b0c83258ac83972b40a5b5dbba1c987ae027db98`.

Target: existing `Instinctes/nightfall#98`.

The PR combines storage semantic seals, checkpoint bootstrap,
network evidence, non-consensus Merkle-sum research and
owner-gated wallet agent intentions.

The research and wallet additions are not required for
storage safety. The maintainer may request scope reduction
before merge.

## Reproducible evidence

### Release compatibility

Run:

    python3 scripts/pr98_legacy_pruned_release_replay.py --workspace .

This replays synthetic Devnet pruned fixtures created by
the original v1.1.2 and v1.1.3 release implementations.

It checks refusal, backup integrity, wrong-network handling,
interrupted recovery, identity preservation and explicit resync.

### Serialized checkpoint, restart and catch-up

Run:

    cargo test --locked -p nightfall-storage checkpoint::tests::pr98_disk_serialized_synthetic_pin_restart_catchup_matches_archive -- --exact --test-threads=1

The synthetic Devnet test:

- Constructs a full-history reference chain from genesis
- Persists and reloads the full-history archive
- Reconstructs a checkpoint at height 11
- Serializes the checkpoint and reads it from disk
- Reconstructs state against an explicit synthetic pin
- Persists the checkpoint state to an isolated datadir
- Performs a clean restart
- Validates ten subsequent blocks through the normal path
- Persists and reloads after every block
- Checks exact final tip hash and block count
- Checks UTXO root and kernel accumulator equality
- Checks minted and burned supply equality
- Checks cumulative work equality
- Checks next difficulty and median time past
- Rejects an altered snapshot header

This test uses a private synthetic checkpoint helper.
It does not execute the real mainnet-only public
checkpoint import against a production checkpoint.

### Storage and recovery

Run:

    cargo test --locked -p nightfall-storage -- --test-threads=1

    cargo test --locked -p nightfall-node --lib recovery::tests -- --test-threads=1

These include recovery-journal, interrupted-write,
pruned-state and fail-closed regression tests.

### Network and cross-platform checks

See the GitHub Actions checks belonging to the
exact reviewed PR98 head commit.

CI success does not establish production readiness.

## Remaining production acceptance requirements

1. An untouched real-world 1.1.2 pruned operator datadir
   must be tested on an offline copy, with consent.
2. Measure startup time on a Mainnet-sized pruned archive.
   Compare official v1.1.3 with this PR, including CPU,
   memory, disk size, cold/warm timing and variance.
3. Export a real independently authenticated Mainnet
   checkpoint, import into a fresh node and complete normal
   catch-up against an independently synchronized full node.
4. Compare real UTXO root, kernel sum, minted/burned supply,
   cumulative work, tip hash and height.
5. Test interruption during the real multi-file import,
   including durable journal handling and restart refusal.
6. Select and independently verify a recent compiled
   checkpoint after height 500000. The maintainer must
   validate its canonical hash and provenance.
7. Complete maintainer security review, scope decisions,
   supported-platform review and deployment policy.

These production conditions are not claimed as completed.

## Safety and deployment boundary

All added tests use synthetic temporary Devnet directories.

No script accesses the user's active Mainnet datadir,
miner process, real seed or wallet.

No command performs a forced migration of operator data.

No consensus activation, checkpoint pinning, new pull request,
upstream merge or deployment occurs automatically.

The maintainer retains full discretion over merging
and may request the unrelated research or wallet scope
to be split before acceptance.


<!-- pr98-intel-runner-capacity-fix-v1 -->

## CI macOS runner allocation

GitHub Actions reported `The job was not acquired by Runner of type hosted` on the original `macos-latest` Apple Silicon (arm64) job. The CI matrix now explicitly chooses a standard **Intel macOS** runner (`macos-15-intel`; if runner acquisition fails, `macos-26-intel`). The macOS build and wallet tests remain enabled. **This replaces arm64 validation with x86_64 validation**: Apple Silicon CI coverage is an outstanding platform-specific check and is not claimed. A green Intel build cannot prove Apple Silicon compatibility. No consensus, wallet or mining code changed to address runner capacity.
