**Update if you mine on a laptop.** Same chain, same genesis
`061a052d…`, protocol v8, wire v6. 0.7.0 still peers. 0.7.1 is the
build that can rejoin after the lid closes.

## A closed laptop never caught up

Close the MacBook for a few hours and the seed keeps mining. Open it
again and Core sat on its old tip. New blocks did not attach. Mining
looked fine and produced coins the network would never take. The only
way back was deleting `blocks.jsonl` and replaying from genesis.

Two things stacked:

1. After sleep the process woke with a stale peer height, sockets
   already dead, and started mining on the old tip. One local block
   is a fork.
2. Catch-up then asked whether the seed's *whole chain* was more than
   500 blocks longer than ours. After ~2 hours it was. The node
   reported `ReorgTooDeep` and stopped trying — even when the two
   histories still shared almost every block.

## The fix

Depth is how many of *our* blocks we would abandon, not how far ahead
the other side is. A one-block rewind with hundreds of new blocks
behind it is catch-up. Sync starts at the next height we do not have
and, on a real fork, pulls from the common ancestor.

Mining waits for a peer to confirm the tip after startup or a clock
jump (the lid). An empty node may still mine — a network of one has
to start. After ten minutes isolated, mining resumes so a machine
with no peers is not bricked.

A coinbase mined onto the stranded fork is not on the canonical chain
and does not come back. Everything else does. You do not need to
delete `blocks.jsonl`.

Phone and browser wallets are unchanged. They talk to the seed over
the light API and never ran this path.

## Downloads

`NIGHTFALLCOIN-Core-0.7.1-macOS-arm64.dmg` · Apple Silicon, macOS 11+
`NIGHTFALLCOIN-Core-0.7.1-macOS-intel.dmg` · Intel, macOS 10.15 Catalina+
`nightfall-core-0.7.1-windows-x64.exe` · Windows 10+ — the wallet
`nightfalld-0.7.1-windows-x64.exe` · Windows — headless node
`nightfall-wallet-0.7.1-windows-x64.exe` · Windows — command-line wallet

Verify against `SHA256SUMS-0.7.1.txt` and `SHA256SUMS-0.7.1-windows.txt`.
Android / iOS stay on the 0.7.0 sideload builds.
