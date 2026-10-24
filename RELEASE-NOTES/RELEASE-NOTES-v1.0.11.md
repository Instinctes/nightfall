# NIGHTFALLCOIN 1.0.11 — wallets that keep scanning, and a safe way back from a dead fork

1.0.11 is a maintenance release on the stable 1.0.x line. It fixes a Core
wallet that stopped scanning once it grew past 16 MiB of state, adds an
offline recovery command for headless nodes stuck on a dead fork, and adds a
tool for comparing independently built binaries. Same chain and genesis,
protocol v8 and wire v6; no consensus, emission, reward or
transaction-validity rule changes.

## Fixed

- **The background wallet scan stopped for long-running miners — #64, fixed in
  #65.** Core copies the open wallet for every background scan, and that copy
  went through an import check with a 16 MiB bound meant for state handed in
  from outside. After roughly 25,000 owned outputs the scan stopped for good
  with `wallet state is too large`: blocks kept downloading and mining kept
  working, but the wallet no longer caught up and could not send. Coins were
  never at risk; every output stayed in the wallet file and on chain.
  - The scan's in-process copy no longer has a size bound.
  - The bound on outside input (imports, encrypted vaults, a legacy-to-vault
    migration) is raised from 16 MiB to 256 MiB.
  - A regression test builds a wallet above 16 MiB and checks that its copy
    round-trips byte for byte, and that oversized outside input is still
    refused before parsing.

  After updating, the automatic scan resumes from where it stopped. Do not
  delete or edit `core.seed.outputs.json`.

## New

- **`nightfalld recover-stalled-fork` — #56.** An offline escape hatch for a
  headless node that is permanently stalled on a branch it can never leave
  (the failure behind #40). Core already offers *Resync chain, keep wallet*
  from its dashboard; a `nightfalld` service has no dashboard. The command:
  - only runs on a stopped node, with an explicit `--network`, `--datadir` and
    `--force`;
  - backs the chain up into a new, uniquely named directory;
  - stages and verifies a fresh genesis before moving any original file, and
    keeps the storage format;
  - leaves the wallet, `miner.seed` and earlier backups untouched;
  - writes a durable recovery journal, so an interrupted reset blocks every
    later start instead of passing for a healthy fresh chain.

  See [docs/RECOVER-STALLED-FORK.md](../docs/RECOVER-STALLED-FORK.md).
- **Independent build comparison — #62.** `scripts/compare_independent_builds.py`
  lets two builders record a receipt for the same reviewed commit and compare
  them offline. It reports matching bytes, different bytes, or that the builds
  are not comparable (different source, lockfile or compiler). Matching
  self-reported receipts are evidence to inspect, not a signed attestation.
  See [docs/INDEPENDENT-BUILD-COMPARISON.md](../docs/INDEPENDENT-BUILD-COMPARISON.md).

## Thanks

Thank you **@itsct-labs** for the precise report in
[#64](https://github.com/Instinctes/nightfall/issues/64), **@parthod0x** for
[#56](https://github.com/Instinctes/nightfall/pull/56) and four careful review
rounds with Linux and Windows evidence, and **@nightfall-wizard** for
[#62](https://github.com/Instinctes/nightfall/pull/62) and the original
recovery proposal in #42.

## Upgrading

Close Core completely before replacing the app or binary. Keep your 24 words
and an encrypted wallet backup, and never delete your data folder to update.
Node operators need do nothing; nodes on 1.0.7 and later stay fully
compatible.

## Build and verification

Official macOS arm64/Intel, Windows x64 and Linux x64 release artifacts are built
by GitHub Actions with `--locked`, gated by platform tests, and published with
SHA256 manifests and GitHub build provenance. Verify the supplied hashes and
attestation using `scripts/check_release_provenance.py` or
`gh attestation verify <file> --repo Instinctes/nightfall`. Provenance is not
Apple notarisation, Windows Authenticode signing or an independent security
audit.

Atomic swaps remain withdrawn. The separately hosted web wallet stays on its
tested 1.0.7 channel; this desktop/node release does not relabel an unchanged
browser module as a new build.
