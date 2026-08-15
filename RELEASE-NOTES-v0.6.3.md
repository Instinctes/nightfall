**Update if you are on 0.6.2 and stuck behind the tip.** Same chain, same
genesis `c8614333…`, protocol v7, wire v5. 0.6.1 and 0.6.2 still peer.
0.6.3 is the build that can leave a one-block fork.

## 0.6.2 stayed on the tip — unless it had forked

Live sockets worked. A wallet that was simply late caught the next block.
A wallet that had mined one competing block at its tip did not.

`GetBlocks` started at `tip − 1`, so the first block in the page was one
the node already held. `apply_block` returned `BadHeight` and the rest of
the page was thrown away. Reorg fetch only ran when the peer was at the
*same* height with a different hash. A peer 120 blocks ahead on the real
chain never triggered it.

The node sat on an open connection to the seed, 120 blocks behind, and
looked idle.

## The fix

Ask for the next height we do not have. If that page still does not
connect and the peer is ahead, pull their chain on a fresh socket and
weigh it — the same rule as before, now reachable from a live session.

A coinbase mined onto the stranded block is not on the canonical chain
and does not come back. Everything else does.

## Downloads

`NIGHTFALLCOIN-Core-0.6.3-macOS-arm64.dmg` · Apple Silicon, macOS 11+
`NIGHTFALLCOIN-Core-0.6.3-macOS-intel.dmg` · Intel, macOS 10.15 Catalina+
`nightfall-core-0.6.3-windows-x64.exe` · Windows 10+ — the wallet
`nightfalld-0.6.3-windows-x64.exe` · Windows — headless node
`nightfall-wallet-0.6.3-windows-x64.exe` · Windows — command-line wallet

Verify against `SHA256SUMS-0.6.3.txt` and `SHA256SUMS-0.6.3-windows.txt`.
