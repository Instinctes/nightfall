**Same chain.** Genesis `061a052d…`, protocol v8, wire v6. No consensus
change; 0.7.x and 0.8.x peer with this normally.

A chain view, and the one honest way to build one for a currency that hides
its amounts.

## nightfallcoin.org/chain

A privacy chain cannot have a normal block explorer. There are no addresses on
the chain, the amounts are Pedersen commitments, and Mimblewimble cuts through
and aggregates the transactions before a block is sealed. An explorer that
could show you who paid whom would be a bug report, not a feature.

What a node *can* prove is now on one page, recomputed on every load:

* **The supply proof.** `Σ UTXO − Σ kernel_excess = (minted − burned)·G`, and
  whether this node currently agrees. Hidden amounts mean a private chain has
  to prove its own supply by arithmetic rather than by counting, so this is the
  first question anyone asks and it deserved a page to point at.
* **Circulating, minted, burned** against the 90,000,000 cap.
* **The set the network agrees on** — unspent outputs, kernels, total work and
  the UTXO root. Two honest nodes at the same height print the same root.
* **Block time and difficulty** over the last 60 blocks, against the 15-second
  target, drawn from the headers themselves.
* **Recent blocks** — height, age, difficulty, input/output/kernel *counts* and
  the reward. Counts, not contents.
* **What the network is running**, counted from the handshake of every peer
  this node has met.

And a section on what the page deliberately cannot show, which is most of what
an explorer usually is.

There is no transaction search, on purpose. A kernel lookup would let anyone
holding a transaction id confirm a payment they were not part of, and would
hand the server a record of who asked. Your own history belongs in your wallet;
to prove a payment to somebody else, use a view key.

The page reads one node, and that node is ours. It says so, in those words. If
it and your own node ever disagree, your node is right.

## `get_headers` on the light API

New read-only method: block headers plus input, output and kernel counts for a
height range, capped at 512 and defaulting to the newest. Nothing it returns is
hidden by the protocol in the first place.

`get_blocks` stays off the public endpoint. It ships full bodies with range
proofs — megabytes per call, and a free amplifier for anyone who asks twice.
A test now pins the allow-list so widening it stays a deliberate act.

Pruned nodes answer from their stored headers with the counts left out, so
"not stored" never renders as "zero".

## Downloads

`NIGHTFALLCOIN-Core-0.8.3-macOS-arm64.dmg`
`NIGHTFALLCOIN-Core-0.8.3-macOS-intel.dmg`
`nightfall-core-0.8.3-windows-x64.exe`
`nightfalld-0.8.3-windows-x64.exe`
`nightfall-wallet-0.8.3-windows-x64.exe`
`nightfall-core-0.8.3-linux-x64`
`nightfalld-0.8.3-linux-x64`
`nightfall-wallet-0.8.3-linux-x64`

Verify `SHA256SUMS-0.8.3.txt`, `SHA256SUMS-0.8.3-windows.txt`,
`SHA256SUMS-0.8.3-linux.txt`.

213 tests, fmt and clippy clean. Upgrading is optional: 0.8.2 keeps working,
it simply cannot answer `get_headers` for anyone pointing a chain view at it.
