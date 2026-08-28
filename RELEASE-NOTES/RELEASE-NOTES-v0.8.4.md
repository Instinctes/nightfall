**Same chain.** Genesis `061a052d…`, protocol v8, wire v6. No consensus
change; 0.7.x and 0.8.x peer with this normally.

**Upgrade if you mine.** This fixes a bug that could stop your miner dead
while telling you it was running.

## One bad transaction could stop a miner completely

Reported on Discord, from a log repeating once a second:

```
WARN template: ledger: duplicate output commitment 02afa428f1…
```

The log line was the small part. Building a block template was
all-or-nothing: if a single transaction from the mempool was unusable, the
whole template was thrown away, the miner slept a second and tried the same
doomed set again. **It hashed nothing** — for as long as that entry stayed,
which without a restart is six hours.

From outside it looked exactly like a wallet that had quietly stopped: the
switch said mining, the rate said 0, and nothing said why. At least one person
ran for hours like that.

Three changes:

**The block gets built without it.** The template builder now drops the
transaction the ledger will not take and mines the block anyway. If nothing
survives the filter it mines an empty block, because a block with only its
coinbase is a perfectly good block and a miner that refuses to produce one is
simply off. The fast path is untouched — filtering only runs after a normal
attempt has already failed.

**The mempool stops offering it.** A transaction refused for a reason that
cannot resolve itself gets dropped instead of being handed to the builder
again next second.

**And the reorg that caused it is fixed.** This is the root cause: adopting a
heavier branch replaced the chain wholesale and never told the mempool.
Anything the new branch had already mined stayed behind, became permanently
invalid, and poisoned every template from then on. Both reorg paths now
reconcile the mempool against the new chain.

## "Mining · 0 H/s" now says why

A switched-on miner that is producing nothing had three different causes and
one appearance. The wallet now says which one:

* *waiting to catch up with the network* — a peer is ahead, and mining on a
  tip they have moved past only deepens a fork
* *the chain will not give this node a block template* — the case above, which
  should no longer happen

`status` carries the same thing as `mining_idle` for headless nodes.

## If you are seeing this right now

Restart the node or Core. The mempool is memory-only, so a restart clears it
and mining resumes immediately. Nothing was at risk in the meantime: the
mempool is local, the chain was unaffected, and no balance ever depended on
it. It was your miner idling.

## Downloads

`NIGHTFALLCOIN-Core-0.8.4-macOS-arm64.dmg`
`NIGHTFALLCOIN-Core-0.8.4-macOS-intel.dmg`
`nightfall-core-0.8.4-windows-x64.exe`
`nightfalld-0.8.4-windows-x64.exe`
`nightfall-wallet-0.8.4-windows-x64.exe`
`nightfall-core-0.8.4-linux-x64`
`nightfalld-0.8.4-linux-x64`
`nightfall-wallet-0.8.4-linux-x64`

Verify `SHA256SUMS-0.8.4.txt`, `SHA256SUMS-0.8.4-windows.txt`,
`SHA256SUMS-0.8.4-linux.txt`.

216 tests, fmt and clippy clean.
