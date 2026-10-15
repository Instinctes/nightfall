# Signed release provenance: phase 1

**Scope:** CI-built Linux x64 and Windows x64 only, starting with releases
after this change merges. Existing release assets are not retroactively
authenticated. Maintainer-built macOS DMGs are not covered.

## Threat model

A SHA256SUMS file published beside a binary does not establish the identity
of its producer: a publisher able to replace both files can change both.
The release workflow must bind each artifact to its source tag and
`Instinctes/nightfall/.github/workflows/release.yml` through GitHub's
Sigstore-backed provenance. Release assets must not be silently clobbered.

## What the gate enforces

1. Tag-push event only; no manual dispatch from `main` that could attest a
   different source ref while building a requested tag.
2. Linux/Windows test + build + version gate must succeed.
3. GitHub Actions generates signed provenance for each binary and its
   platform-specific SHA256SUMS file.
4. `scripts/check_release_provenance.py` rejects tampered assets,
   missing/duplicate/unexpected checksum lines, symlinks, missing signed
   attestations, a wrong repository/workflow, or a wrong tag ref.
5. An upload without `--clobber` fails if a same-named asset already exists.

## Operator check

From a directory containing exactly three files for one platform, plus its
matching SHA256SUMS text file:

```sh
python3 scripts/check_release_provenance.py --tag v1.0.7 --platform linux --directory dist
```

Requires a sufficiently recent authenticated GitHub CLI (`gh`), online
verification, and actual signed artifacts from a release produced after
the gate is merged. A mock success in unit tests **does not** validate
a production attestation. Run the independent check against real assets.

## Limitations and next release step

- The gate verifies **build provenance**, not source safety or byte-for-byte
  reproducibility. It does not constitute an independent security audit.
- GitHub Actions is in the trusted build chain; a compromised signing
  workflow could attest a malicious binary.
- It cannot stop maintainers replacing assets through other tools or changing
  a tag. **Immutable Releases must be enabled by the upstream maintainer**,
  and the release workflow redesigned to gather both CI artifacts and the
  currently hand-built macOS artifacts **before** publishing an immutable
  release. Do not enable immutability blindly while macOS assets are still
  uploaded later by hand.
- Re-running a release that has already attached same-named assets now fails
  instead of silently replacing them. Use a new versioned tag and document
  changes transparently.
- Evidence for Linux/Windows must not be described as covering macOS.
- Do not attach wallet seeds, account data, private keys or RPC credentials
  to reports.
