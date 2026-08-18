**Recommended update.** Same chain, same genesis `c8614333…`, protocol v7,
wire v5 — 0.6.1 and 0.6.2 peer with each other normally. An old wallet keeps
every coin. What 0.6.2 fixes is staying on the tip.

## Wallets behind NAT were always a few blocks late

A node behind a router can dial out but cannot be dialled. The handshake
already kept inbound sockets open; the client then threw that socket away
and the seed announced the next block by dialling a listen address that
does not exist. Blocks arrived on the next eight-second poll — or not at
all, if the only connection was stuck in `CLOSE_WAIT`.

Mining on that stale tip produced valid coinbases that the heavier chain
orphaned the moment the two sides met. From inside the wallet that reads
as "my newly mined coins keep getting rejected".

## The fix

Outbound connections stay up. A new block is written onto the live socket,
not offered as a fresh TCP handshake to an address nobody can reach. A
wallet that can reach a seed is at most one round-trip behind the finder.

The eight-second full-peer pull is now a safety net: live sessions exchange
`GetStatus`, and blocks are fetched only when the tips actually differ.
The Core wallet scans when the tip moves, not on a timer. Light clients
get the same feed as a stream (`scan_subscribe` / `nightfall-wallet follow`).

0.6.1 still works on this network. It just cannot stay in real time.

## Tests

150 passing. New coverage in `nightfall-node` for the session pool,
including that an inbound announce to the same listen address cannot
overwrite and then delete a live outbound.

## Downloads

`NIGHTFALLCOIN-Core-0.6.2-macOS-arm64.dmg` · Apple Silicon, macOS 11+
`NIGHTFALLCOIN-Core-0.6.2-macOS-intel.dmg` · Intel, macOS 10.15 Catalina+
`nightfall-core-0.6.2-windows-x64.exe` · Windows 10+ — the wallet
`nightfalld-0.6.2-windows-x64.exe` · Windows — headless node
`nightfall-wallet-0.6.2-windows-x64.exe` · Windows — command-line wallet

Verify against `SHA256SUMS-0.6.2.txt` and `SHA256SUMS-0.6.2-windows.txt`.
