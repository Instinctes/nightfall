# Verifying and reproducing releases

Local runbook, 30 September 2026 (Roadmap 2027, milestone 8).

Two different questions, often confused:

1. **Is the file I downloaded the file the project published?** — a
   checksum answers this. Use it every time.
2. **Was the published file built from the published source?** — only a
   reproducible build answers this, and only when *someone else* rebuilds it
   and gets the same bytes.

## 1. Checking a download (everyone)

```
nightfall-nodekit verify --sums SHA256SUMS-1.0.5.txt
# or, without the kit:
shasum -a 256 -c SHA256SUMS-1.0.5.txt --ignore-missing
```

Every line must say `OK`. Fetch the checksum list from the release page over
HTTPS, not from wherever you got the binary. A matching checksum proves
identity with what was published — not that the software is safe, and not
that it was built from the source you can read.

## 2. Reproducing a build (operators, reviewers)

```
scripts/release/compare-builds.sh [commit]
```

Exports the commit with `git archive` into two different temporary
directories, builds `nightfalld`, `nightfall-wallet` and `nightfall-nodekit`
in release mode with Rust 1.98.0, `--locked --offline`, and these settings
that keep the build environment out of the binary:

- `--remap-path-prefix=<checkout>=/build/nightfall` and
  `--remap-path-prefix=<cargo home>=/cargo` (panic messages and debug info
  otherwise embed your paths),
- `SOURCE_DATE_EPOCH` = the commit's timestamp,
- `CARGO_INCREMENTAL=0`.

It then compares SHA-256 per binary and writes
`output/release-compare/<timestamp>/report.md` and `SHA256SUMS-a.txt`.

**To compare with someone else:** both run the script on the same commit and
the same target (e.g. `aarch64-apple-darwin`, same macOS SDK) and exchange
`SHA256SUMS-a.txt`. Identical lines mean identical bytes.

### Result on 30 September 2026

Commit `c47487d`, rustc 1.98.0, Darwin arm64, two checkout paths:
**all three binaries byte-identical** (`nightfalld a22482f9…`,
`nightfall-wallet bd3b60dc…`, `nightfall-nodekit b4dae5dc…`).

That is the necessary first step — the same machine does not leak its
paths or time into the bytes — and nothing more. **No one outside the project
has reproduced any Nightfall build.** Different machines can still diverge
(SDK version, linker, CPU-specific codegen settings); only an outside
comparison settles it.

### What cannot be reproduced this way (yet)

- **Published 1.0.x binaries.** `scripts/build-macos-dmgs.sh` and the GitHub
  release workflow build without path remapping or `SOURCE_DATE_EPOCH`, so
  their checksums are not expected to match this recipe. Adopting these
  settings in the release scripts is a release-process change for the
  maintainer; it was deliberately not made tonight (no release changes
  during the local run).
- **The macOS app bundle and DMG.** The recorded local macOS build is ad-hoc signed and **not notarised**.
  Bundle/signature reproducibility is outside this CLI comparison; do not infer
  matching app bundles or DMGs from it.
- **Windows builds**, cross-compiled with MinGW: not tested.

## 3. Operator checklist for an update

1. Download binaries and the `SHA256SUMS` list; run step 1.
2. Optionally rebuild with step 2 and compare with a peer operator.
3. Follow `docs/NODE-KIT.md` §3 (keep the previous binary, restart,
   `doctor`, `status`) and §4 to roll back.


### Runner isolation, 30 September 23:45 UTC (local)

The runner explicitly separates CARGO_TARGET_DIR for each archive, overrides
RUSTFLAGS with path remapping, and clears encoded flags, compiler wrappers and
compiler/target overrides that could defeat the comparison. It resolves the
requested revision to a commit and reports that uncommitted changes are excluded.
Both full SHA256 manifests and the archived Cargo.lock checksum are recorded.
Failed builds and unequal outputs retain temporary evidence and a FAILED marker.

`python3 scripts/check-build-comparison.py` uses synthetic tools to verify
independent targets despite an inherited shared-target setting, removal of
conflicting settings, failures and unequal-output retention. This tests the
runner, not Rust binaries. No new real-build comparison or outside reproduction
is claimed by this change. Cargo registry/config and system SDK/linker remain
host inputs: this is not a hermetic build environment.
