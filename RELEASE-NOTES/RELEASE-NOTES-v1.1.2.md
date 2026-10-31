# NIGHTFALLCOIN 1.1.2 — steadier sync, safer storage

1.1.2 is a maintenance release on the 1.1.x line. It fixes initial sync when
a peer sends short block pages, lets one peer no longer hold back fork
recovery from another, and makes block storage survive a power cut. The
consensus rules are exactly those of 1.1.1: the upgrade still activates at
**block 500,000**, and supply, emission, block reward, block time and proof
of work are unchanged.

Nodes on 1.1.1 stay fully compatible. Nodes still on 1.0.x must upgrade to
1.1.1 or later before block 500,000.

## Fixed

- **Initial sync no longer leaves holes after a short block page — #79.**
  A node serving blocks trims a page to the P2P size limit, so a reply can
  hold fewer than 128 blocks. The downloading node used to move its request
  cursor on by a full page anyway and then wait for timeouts to fill the gap.
  It now asks for the exact next height, and fork downloads keep paging
  until the peer's advertised height. Blocks are larger after block 500,000,
  so this case becomes more common there.
- **Fork recovery is tracked per peer — #9.** The pause between reorg
  fetches used to be shared by the whole node, so one peer could hold back
  recovery from another peer offering a heavier chain. Each peer now has its
  own pause, in a bounded table.
- **Block storage survives a power cut — #18.** `blocks.bin` is synced to
  disk before `chain-meta.json` is written, and the metadata is replaced
  atomically. If the computer stops in the middle of an append, the node
  rolls back to the last confirmed state on the next start, but only when the
  data up to that point still matches its own validation record. A data
  folder that belongs to another network is refused before anything in it is
  changed.
- **Stricter public light API — #86.** The read-only `GET` and `OPTIONS`
  requests must not carry a body or ambiguous framing, and the rate limiter
  no longer scans its whole table on every request.
- **Node Kit** explains a node that accepts the connection but closes it
  without a reply, instead of reporting a bare I/O error.

## Tests

- A Devnet chain is mined across the activation height, replayed through
  both validation paths, and blocks without the new fields or with a
  non-canonical kernel offset are rejected — #87.
- Property tests for Schnorr signatures: every bit of `r` and `s`,
  non-canonical `s`, identity `r` — #30.
- Conformance vectors from each cryptographic primitive's own specification
  — #36.
- A real TCP test for short block pages (#79) and a three-node fork
  convergence test (#9).

## Thanks

Thank you **@nightfall-wizard** for
[#79](https://github.com/Instinctes/nightfall/pull/79),
[#9](https://github.com/Instinctes/nightfall/pull/9),
[#18](https://github.com/Instinctes/nightfall/pull/18),
[#86](https://github.com/Instinctes/nightfall/pull/86),
[#87](https://github.com/Instinctes/nightfall/pull/87),
[#30](https://github.com/Instinctes/nightfall/pull/30) and
[#36](https://github.com/Instinctes/nightfall/pull/36).

## Upgrading

Close Core completely before replacing the app or binary. Keep your 24 words
and an encrypted wallet backup, and never delete your data folder to update.
Node operators and pools: 1.1.2 is recommended before block 500,000; 1.1.1 is
the minimum.

The wallet itself is unchanged from 1.1.1. The web wallet stays on its tested
1.1.0 channel; this release does not relabel an unchanged browser module as a
new build.

## Build and verification

Official macOS arm64/Intel, Windows x64 and Linux x64 release artifacts are
built by GitHub Actions with `--locked`, gated by platform tests, and published
with SHA256 manifests and GitHub build provenance. Verify with
`scripts/check_release_provenance.py` or `gh attestation verify <file> --repo
Instinctes/nightfall`. Provenance is not Apple notarisation, Windows
Authenticode signing or an independent security audit.
