# NIGHTFALLCOIN 1.0.9 — new identity, bounded RPC and safer operator tools

1.0.9 is a focused maintenance release on the stable 1.0.x line. It includes the
new NIGHTFALL logo and app icons for macOS, Windows, Linux and the tray/sidebar.
The larger 1.1.0 roadmap feature set and experimental GPU miner are not included.
Same chain and genesis, protocol v8 and wire v6; no consensus, emission, reward
or transaction-validity rule changes.

## Included community contributions

- **RPC resource containment and wallet compatibility — #51.** At most 32 RPC
  connections and eight scan subscriptions, bounded frames and writes, a
  separate first-byte idle timeout, and a deadline for incomplete frames.
  Long-running wallet follow subscriptions remain open beyond ten minutes.
  The real-client CI regression now runs for changes to relevant node/wallet
  code and shared dependencies. Existing unauthenticated RPC loopback defaults
  remain unchanged; these limits are not authentication.
- **Locked official release inputs — #54.** Tests and binary builds on every
  release platform use Cargo.lock with --locked. A policy regression guards
  against accidentally dropping those flags. This fixes dependency resolution;
  it does not establish bit-for-bit reproducibility.
- **Disposable CLI backup rehearsal — #55.** Six offline Devnet cases check
  seed-file identity restoration and rejection of missing, truncated or
  interrupted state. This checks the CLI seed boundary; it is not a complete
  encrypted-vault, mnemonic or historical-payment recovery guarantee.
- **Isolated temporary Linux build swap — #57, with maintainer hardening.** Both
  installers allocate unique temporary swap files and register cleanup before
  allocation. Existing operator files are preserved. If swapoff fails, the
  still-active temporary swap file remains named for manual recovery, rather
  than being unlinked. A mocked regression covers this failure path as well.

## Thanks

Thank you **@nightfall-wizard** for PRs [#51](https://github.com/Instinctes/nightfall/pull/51),
[#54](https://github.com/Instinctes/nightfall/pull/54),
[#55](https://github.com/Instinctes/nightfall/pull/55) and
[#57](https://github.com/Instinctes/nightfall/pull/57), including the follow-up
changes and regression tests. Contributor commits are retained in the release
history. Thanks also to **@parthod0x** for the detailed headless-recovery proposal
and review discussion. Recovery PRs #42 and #56 are still under review and are
**not shipped in 1.0.9**; they are alternative implementations, not a combined
recovery feature.

## Build and verification

Official macOS arm64/Intel, Windows x64 and Linux x64 release artifacts are built
by GitHub Actions, gated by platform tests, and published with SHA256 manifests
and GitHub build provenance. Verify the supplied hashes and attestation using
`scripts/check_release_provenance.py`. Provenance is not Apple notarisation,
Windows Authenticode signing or an independent security audit.

Keep an encrypted wallet backup before upgrading and close the old wallet
before starting another copy. The existing data format is retained. Atomic
swaps remain withdrawn. The separately hosted webwallet stays on its tested
1.0.7 channel; this desktop/node release does not relabel an unchanged browser
module as a new build.
