**Update immediately.** Same chain, same genesis `c8614333…`, protocol v7,
wire v5 — 0.6.0 and 0.6.1 peer with each other normally. But a node running
0.6.0 that has diverged from the network cannot rejoin it, and every block it
mines is lost.

## Two nodes, one plainly heavier, permanently unable to reconcile

When a peer's blocks do not connect to ours, the node pulls that peer's chain
and weighs it. The pull was capped at `MAX_REORG_DEPTH * 4` — a flat 2,000
blocks, chosen when the chain was short enough that the difference never
showed.

The chain passed 2,000 blocks today, and the cap became a wall.

A node on 2,048 blocks asked the seed node, which held 2,057, for its chain. It
received the first 2,000 of them — a *prefix* of a longer chain — weighed that
prefix against its own 2,048, correctly concluded it carried less work, and
refused it. Then it did the same on the next round, and the next. The seed was
not offering a worse chain; the laptop was only ever being shown part of a
better one.

```
MacBook  2048 blocks  work 21,533,890
Seed     2057 blocks  work 21,621,404      ← plainly heavier
log:     "peer 209.250.235.133 chain is not heavier"
```

Neither side could move: the shorter node would not adopt what it could see of
the longer chain, and it never offered its own, because the push path asks
whether the peer is *shorter*, not whether it is lighter. Everything mined on
the stranded side was mined onto a branch nothing would ever accept — which is
exactly what "my newly mined coins keep getting rejected" looks like from the
inside.

## The fix

The cap is now `our_len + MAX_REORG_DEPTH`: enough to hold all of the peer's
chain, never more than we would accept anyway. That number is not a new guess —
it is the acceptance rule stated once. `evaluate_reorg` rejects anything longer
than ours plus `MAX_REORG_DEPTH` as too deep, so pulling further is bandwidth a
hostile peer would be glad to make us spend, and pulling less eventually
truncates a legitimate chain.

Any fixed constant has this failure waiting inside it. The only question is
which block reaches it.

## Tests

146 passing, up from 143. Four new ones in `nightfall-node`, pinned to the
numbers this was found at:

- `a_peer_longer_than_two_thousand_blocks_is_still_fetched_whole` — 2,048 vs
  2,057, and it asserts the cap exceeds the old constant, so restoring a fixed
  bound fails the build.
- `we_never_pull_more_than_we_would_accept` — the ceiling stays tied to the
  acceptance rule.
- `a_short_peer_is_not_padded`, `genesis_only_peer_is_one_block`.

`cargo fmt --check` and `clippy --workspace --all-targets -D warnings` clean on
the toolchain CI uses.

## If your node is stuck

Install this build and restart. A node that had diverged will pull the real
chain on its next sync round and rejoin on its own. Your seed phrase, your
wallet file and every coin on the canonical chain are untouched. Coins mined
onto the stranded branch were never accepted by the network and do not come
back — that branch was the bug.

## Downloads

`NIGHTFALLCOIN-Core-0.6.1-macOS-arm64.dmg` · Apple Silicon, macOS 11+
`NIGHTFALLCOIN-Core-0.6.1-macOS-intel.dmg` · Intel, macOS 10.15 Catalina+
`nightfall-core-0.6.1-windows-x64.exe` · Windows 10+ — the wallet
`nightfalld-0.6.1-windows-x64.exe` · Windows — headless node
`nightfall-wallet-0.6.1-windows-x64.exe` · Windows — command-line wallet

Verify against `SHA256SUMS-0.6.1.txt` and `SHA256SUMS-0.6.1-windows.txt`.
