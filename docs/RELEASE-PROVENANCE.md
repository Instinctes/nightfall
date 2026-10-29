# Signed release provenance

**Scope:** every wallet file of a release — Linux x64, Windows x64 and both
macOS DMGs (Apple Silicon and Intel) — built by GitHub Actions, starting with
the first release after this change. Phase 1 (PR #46 by @nightfall-wizard)
covered Linux and Windows; phase 2 moved the macOS build onto a GitHub runner
too. Existing release assets (1.0.6 and older) are not retroactively
authenticated.

## Threat model

A SHA256SUMS file published beside a binary does not establish the identity
of its producer: a publisher able to replace both files can change both.
The release workflow binds each artifact to its source tag and
`Instinctes/nightfall/.github/workflows/release.yml` through GitHub's
Sigstore-backed provenance. Release assets must not be silently clobbered.

## What the gate enforces

1. Tag-push event only; no manual dispatch from `main` that could attest a
   different source ref while building a requested tag.
2. Windows, Linux and macOS each run the test suite, build, and check that the
   binary reports the tag's version.
3. Only after all three succeed does the `publish` job — the only job with
   write and signing permissions — attest every file, including the
   SHA256SUMS lists.
4. `scripts/check_release_provenance.py` then verifies every platform:
   tampered assets, missing/duplicate/unexpected checksum lines, symlinks,
   missing signed attestations, a wrong repository/workflow, or a wrong tag
   ref are rejected.
5. The release is created as a draft, all assets are uploaded, and only then
   is it published. A release that already has assets is never added to or
   overwritten; upload runs without `--clobber`.

## Operator check

Download one platform's files into an otherwise empty directory:

| Platform | Files |
|---|---|
| `linux` | `nightfall-core`, `nightfalld`, `nightfall-wallet` (`-<version>-linux-x64`) + `SHA256SUMS-<version>-linux.txt` |
| `windows` | the same three as `-<version>-windows-x64.exe` + `SHA256SUMS-<version>-windows.txt` |
| `macos` | `NIGHTFALLCOIN-Core-<version>-macOS-arm64.dmg`, `…-macOS-intel.dmg` + `SHA256SUMS-<version>.txt` |

```sh
python3 scripts/check_release_provenance.py --tag v1.0.7 --platform macos --directory dist
```

Or for a single file:

```sh
gh attestation verify NIGHTFALLCOIN-Core-1.0.7-macOS-arm64.dmg --repo Instinctes/nightfall
```

Requires a sufficiently recent authenticated GitHub CLI (`gh`) and online
verification. A mock success in unit tests **does not** validate a production
attestation. Run the independent check against real assets.

## Limitations

- This is **build provenance**, not an Apple Developer ID, notarization, or
  Windows Authenticode signature. The project holds no such credentials, so
  macOS and Windows still warn on first launch.
- It does not prove source safety or byte-for-byte reproducibility, and it is
  not an independent security audit.
- GitHub Actions is in the trusted build chain; a compromised workflow could
  attest a malicious binary.
- It cannot stop a maintainer replacing assets through other tools or moving a
  tag. Now that every asset is published by one job, GitHub's Immutable
  Releases can be enabled to close that gap.
- A failed release is not repaired by re-running: use a new version tag and
  say what changed.
- Pre-release tags (`v1.2.3-dev.1`) exercise the pipeline only. Their DMGs
  default to Devnet, and pre-release builds refuse mainnet.
- Do not attach wallet seeds, account data, private keys or RPC credentials
  to reports.
