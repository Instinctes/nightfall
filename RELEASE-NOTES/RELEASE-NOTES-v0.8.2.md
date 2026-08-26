**Same chain.** Genesis `061a052d…`, protocol v8, wire v6. No consensus
change; 0.7.x and 0.8.x peer with this normally.

The release that makes a lost payment impossible instead of merely visible.

## A payment that misses now gets sent again

A new transaction is handed to **exactly one** randomly chosen peer, so it
cannot be traced back to the node that made it. That part is deliberate. What
was missing is that nothing ever repeated it. One dropped hop — a peer
restarting, a Tor exit vanishing — and the payment ceased to exist while the
wallet said "pending" for ever.

0.8.1 made that failure visible. This one repairs it.

**The wallet now keeps the transaction** and re-submits it after every sync,
until a block takes it or you abandon it. Core does this through the local
node; the browser wallet keeps a small outbox in the browser and gives up
after a day, because after a day it is not a dropped hop, it is a payment the
network will not take — and retrying for ever would hide that.

Entries written by older versions have no stored transaction and cannot be
re-sent. For those, **Settings → Rescan from genesis** still releases the
reserved coins.

## Nodes now forget transactions nobody mines

The mempool only ever dropped what a block *consumed*. A transaction that
never reached a block was therefore never dropped at all. Measured on mainnet
on 26 August: one seed was holding 60 of them, the other 117 — different sets
of the same corpses, because each had heard different ones.

Left alone that walks to the 10,000-entry cap, and at the cap a node **stops
accepting new transactions**. Quietly. A node that has been up long enough
becomes unable to relay a payment and nothing says so.

Transactions now expire after six hours — 1,440 blocks, the same span as
coinbase maturity. A payment no miner has taken in that time is not going to
be taken. And a full mempool now sweeps before it refuses, so a node that has
filled up with dead entries heals itself instead of going deaf.

The two halves fit together on purpose: nodes forget, and the sender remembers
and repeats. Neither is much use alone.

## Fewer ways to ship a broken screen

Three UI bugs went out in 0.8.1 because a stylesheet rule named markup that
does not exist — a rule for `input[type="text"]` while no field carries a
`type`, a rule for `.words .word` while the phrase grid emits classless spans,
a button base scoped to one container. Each was found by a person looking at a
phone.

`scripts/check-web-wallet.mjs` now runs in CI and finds that class of mistake
in a second: every class used must have a rule, every id the script looks up
must be produced by a template, the four cache busters must agree, and the
wallet's build string must match the workspace version.

The release workflow also refuses to publish a binary whose `--version`
disagrees with the tag. The seeds once ran 0.8.0 code introducing itself as
0.7.8; nothing broke, but the network's version census was wrong and no one
could tell from outside.

## Downloads

`NIGHTFALLCOIN-Core-0.8.2-macOS-arm64.dmg`
`NIGHTFALLCOIN-Core-0.8.2-macOS-intel.dmg`
`nightfall-core-0.8.2-windows-x64.exe`
`nightfalld-0.8.2-windows-x64.exe`
`nightfall-wallet-0.8.2-windows-x64.exe`
`nightfall-core-0.8.2-linux-x64`
`nightfalld-0.8.2-linux-x64`
`nightfall-wallet-0.8.2-linux-x64`

Verify `SHA256SUMS-0.8.2.txt`, `SHA256SUMS-0.8.2-windows.txt`,
`SHA256SUMS-0.8.2-linux.txt`.

212 tests, fmt and clippy clean. Browser wallet is 0.8.2 and needs no install.
