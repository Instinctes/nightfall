# NIGHTFALLCOIN 1.1.4 — faster block validation

1.1.4 makes every validated block cheaper to check, which matters most for
a new wallet catching up with Fast start. Consensus rules, supply, emission,
block reward, block time and proof of work are unchanged, and nodes on 1.1.1,
1.1.2 and 1.1.3 stay fully compatible.

## Faster

- **The UTXO root no longer re-hashes every unspent output for every block.**
  The root over ~215,000 unspent outputs is checked for each validated block,
  and recomputing all of their leaf hashes was most of that cost. Each leaf
  now is computed once, when its output enters the set. The root is
  byte-for-byte the same; a set loaded from disk without the cache computes
  the leaves itself.
- **Hashing short inputs takes one call instead of many.** Blake3 hashes the
  same bytes either way, so the result is identical; the Merkle nodes and
  leaves behind the UTXO root are simply cheaper to compute.
- **Difficulty and median-time checks read only the blocks they use.** The
  retarget looks at the last 91 blocks and median-time-past at the last 11,
  but both used to copy the whole header history for every block.
- **New checkpoint at 530,000.** Cross-checked between an operator's chain
  file and the seed that serves the website's chain view: block hash, parent
  and UTXO root agree.

Measured on an Apple M3 Pro (11 cores) with a real 533,143-block chain file
that this node did not validate itself (as after Fast start):

| | 1.1.3 | 1.1.4 |
|---|---|---|
| Blocks validated after the checkpoint | ~45 per second | ~104 per second |
| Same file, same 505,000 checkpoint | 590 s | 279 s |
| Same file with this release's checkpoint (530,000) | — | 42 s |

Every block after the newest checkpoint is still validated in full, so the
first start of a new wallet grows again with the age of the checkpoint. The
checkpoint moves forward with each release.

## Upgrading

Close Core completely before replacing the app or binary. Keep your 24 words
and an encrypted wallet backup, and never delete your data folder to update.
Nothing has to be migrated.

## Build and verification

Official macOS arm64/Intel, Windows x64 and Linux x64 release artifacts are
built by GitHub Actions with `--locked`, gated by platform tests, and published
with SHA256 manifests and GitHub build provenance. Verify with
`scripts/check_release_provenance.py` or `gh attestation verify <file> --repo
Instinctes/nightfall`. Provenance is not Apple notarisation, Windows
Authenticode signing or an independent security audit.
