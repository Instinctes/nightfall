# NIGHTFALLCOIN 1.1.3 — a new wallet is ready in minutes

1.1.3 is the first release after the binding rules activated at block
500,000. Its main change is how fast a new node gets onto the chain: the
published chain archive can now be fetched from inside Core, and the node
checks it on every core instead of one. Supply, emission, block reward, block
time and proof of work are unchanged, and nodes on 1.1.1 and 1.1.2 stay fully
compatible.

## Faster first start

- **Fast start in Core.** A wallet that is far behind offers to download the
  chain archive that `nightfallcoin.org/chain/` already publishes, as one file
  from GitHub, instead of fetching half a million blocks from peers page by
  page. The download resumes where it stopped and is checked against its
  SHA-256 before use. It is only offered on mainnet, only when asked for, and
  never when the node is set to keep its traffic behind a proxy or under
  privacy lockdown: it is a direct HTTPS download, and the screen says so.
- **Nothing in the archive is trusted.** The node treats it exactly like a
  chain file copied in by hand. It replays the blocks up to this build's
  newest checkpoint by linkage and body hash, compares the state they produce
  with the pinned block, and validates every block after the checkpoint in
  full. It switches to the archive only when it carries more work than the
  chain the node already has, and writes it to the data folder with its own
  validation record.
- **Proof of work is checked on all cores.** Nighthash-v2 costs about 11 ms per
  block and depends only on the header, so blocks from peers, from a chain
  file and from a reorg are now hashed in parallel before they are applied in
  order. Every other rule still runs in the same order as before, and the
  first invalid block is reported at the same index with the same reason.
- **The UTXO root is hashed on all cores.** The root over 215,000 unspent
  outputs is computed for every validated block. It is now split into
  independent subtrees; the result is byte-for-byte the same root.
- **New checkpoint at 505,000**, 5,000 blocks after activation. Cross-checked
  between an operator machine's chain file and the seed that serves the
  website's chain view: block hash, parent and UTXO root agree.

Measured on an Apple M3 Pro (11 cores) with the published 505,591-block
archive: the archive loads and validates in **23 seconds**. Without the
checkpoint (`NIGHTFALL_NO_ASSUME_VALID=1`) the same file validates at about
164 blocks per second on this build; before it, proof of work alone was about
84 minutes on one core.

## Also in this release

- **Progress shows the real total.** A chain file dropped into a data folder
  that had already been started showed "1,153 / 1 blocks". The total now
  comes from the file itself.
- **Your address while catching up.** The waiting screen shows the receive
  address of an unlocked wallet. A new wallet can give it out before the
  chain has caught up.
- **Clean shutdown — #7.** `nightfalld` stops on SIGINT, SIGTERM and Windows
  console events; Core stops the node when its window closes. Workers are
  joined with a 15-second bound, and the chain is written only when it was
  fully loaded.
- **Fork choice by work for shorter branches — #10.** A peer whose tip is
  lower than ours is no longer assumed to be behind: if its tip is not one of
  our blocks, its branch is fetched and judged by cumulative work. A pruned
  node refuses a reorg below its horizon instead of trying it.
- **Removed peers stay removed — #8.** A peer taken out of the list is not
  dialled again from peer exchange or the directory until it is added back.

## Tests

- Parallel proof-of-work checks agree with the sequential check for every
  thread count; an invalid block is rejected at its own index; a verdict for
  another block or other parameters is ignored.
- The split UTXO root equals the plain definition for every size around the
  split boundaries and through the real entry point.
- The archive download: complete, wrong checksum, interrupted and resumed,
  server without range support, oversized response, cancelled, reused, and a
  manifest that tries to point anywhere but an archive file.
- A running node continues from an archive and trusts it on the next start;
  an archive that is not ahead changes nothing; an archive with one invalid
  block is refused as a whole.
- Every state of the fast-start panel fits every supported window width.
- From #7, #10 and #8: daemon stop with the real binary, restart after a reorg,
  a real two-node shorter-but-heavier test, and multi-node convergence with
  peer removal.

## Thanks

Thank you **@nightfall-wizard** for
[#7](https://github.com/Instinctes/nightfall/pull/7),
[#10](https://github.com/Instinctes/nightfall/pull/10) and
[#8](https://github.com/Instinctes/nightfall/pull/8).

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
