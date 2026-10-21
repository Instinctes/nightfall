# NIGHTFALLCOIN 1.0.10 — traceable Linux Node Kit installs

1.0.10 is a small maintenance release on the stable 1.0.x line. It changes how
the official Linux Node Kit installers build a node and adds an optional
pre-install check for servers. Wallet, node and miner behaviour is the same as
1.0.9. Same chain and genesis, protocol v8 and wire v6; no consensus, emission,
reward or transaction-validity rule changes. The larger 1.1.0 roadmap feature
set and the experimental GPU miner are not included.

## Included community contributions

- **Locked installer builds with source fingerprints — #60.** Both Linux
  installers (`install-node-linux.sh`, `install-seed-node-linux.sh`) now build
  with `cargo build --locked`, and print the exact checked-out Git commit and the
  SHA-256 of `Cargo.lock` before compiling. The install stops if either cannot
  be determined. A CI regression with mutation cases guards these lines. The
  fingerprint is diagnostic evidence for support and comparison, not a digital
  signature or an independent audit.
- **Read-only Linux host preflight — #61.** `scripts/node_kit_preflight.py
  --role node|seed` checks a server before installation: Linux with a running
  systemd, CPU architecture, `bash`/`systemctl`/`sudo`, Git, a Rust toolchain or
  a package manager to bootstrap one, free temporary build space, and whether a
  Node Kit installation already exists. It makes no network requests and
  changes nothing. `READY` is a deliberately narrow claim, not a guarantee of a
  successful install. See [docs/NODE-KIT-PREFLIGHT.md](../docs/NODE-KIT-PREFLIGHT.md).

## Thanks

Thank you **@nightfall-wizard** for PRs [#60](https://github.com/Instinctes/nightfall/pull/60)
and [#61](https://github.com/Instinctes/nightfall/pull/61), including their
regression tests. Recovery proposals #42 and #56 and the build-comparison
proposal #62 are still under review and are **not shipped in 1.0.10**.

## Upgrading

Desktop users can update at their own pace; nothing in 1.0.9 needs replacing
urgently. Close Core completely before replacing the app or binary, keep your
24 words and an encrypted wallet backup, and never delete your data folder to
update. Node operators need do nothing; seeds and nodes on 1.0.7 or 1.0.9 stay
fully compatible.

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
