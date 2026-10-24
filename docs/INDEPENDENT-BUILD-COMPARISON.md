# Offline independent-build byte comparison (advisory)

The roadmap calls for independent operators to **build and compare** Nightfall.
`compare_independent_builds.py` addresses a different question from
`scripts/check_release_provenance.py`: the latter validates release digests
and GitHub provenance; this tool compares **two locally produced native release
binaries** with declared identical source inputs and compiler identities.

No new Rust dependencies, network requests, wallet reads, mining operations,
release modifications, consensus changes, or background services are involved.

## Builder A and builder B (each on their own machine)

Start from an independently obtained, clean checkout of **the same reviewed
commit**, with a recorded `Cargo.lock`. Build normally using `--locked`:

```sh
cargo build --release --locked -p nightfall-node
python3 scripts/compare_independent_builds.py record \
  --repo . --binary nightfalld=target/release/nightfalld > "$HOME/nightfall-build-A.json"
```

Builder B performs the same steps and saves `$HOME/nightfall-build-B.json`.
Place receipts **outside the source checkout** so that its dirty-worktree
check remains meaningful. Only exchange the JSON receipts when both operators
consent. No hostname, device ID, absolute path, network address, Git remote,
recovery phrase or private key is recorded. The script fails closed for dirty
worktrees, symlink artifacts and invalid compiler metadata; the local build
is **not** run by the tool.

## Compare offline

```sh
python3 scripts/compare_independent_builds.py compare \
  "$HOME/nightfall-build-A.json" "$HOME/nightfall-build-B.json"
```

- **Exit 0 / MATCHING_BYTES:** receipt hashes and sizes match with declared identical build inputs.
- **Exit 1 / DIFFERENT_BYTES:** build inputs match but at least one binary digest or size differs.
- **Exit 2 / INCOMPARABLE:** source, Cargo.lock, compiler identities, native target, profile or artifact names differ. Do not infer a reproducibility failure.
- **Exit 3 / REJECTED:** malformed, oversized, duplicate-key, symlink, unexpected-field or missing receipts.

All receipt fields are whitelisted. Input JSON is bounded to 8 KiB; duplicates
and unexpected metadata are rejected to prevent accidental publication of
sensitive operator data. The test suite can run offline, including on Termux:

```sh
python3 -m unittest discover -s scripts -p 'test_compare_independent_builds.py' -v
```

## Limits of the evidence

A pair of matching **self-reported** SHA-256 receipts does not verify the actual
binary bytes of a remote builder, the build command or the builder's
independence. It is not a signed attestation, independent audit, guaranteed
bit-for-bit reproducibility, or proof of chain correctness. Compare real
artifacts and independently verify their hashes before escalating the claim.
Native host compiler versions must agree exactly: cross-compilation,
OS differences and reproducibility across different toolchains are out of scope.
No official Nightfall release has been independently reproduced by this
fixture-only contribution.
