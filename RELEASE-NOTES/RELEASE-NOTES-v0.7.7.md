**Same chain.** Genesis `061a052d…`, protocol v8, wire v6. 0.7.0–0.7.6
still peer.

## What you can see now

The wallet build sits next to **CORE WALLET**, not under MAINNET v8.

Dashboard adds four numbers the GUI already had the ingredients for:

- **Last block** — age of the tip. Orange if it is older than a minute.
- **Network hash** — `difficulty ÷ 15 s`. An estimate, not a miner count.
- **Your share** — this machine against that estimate, while mining.
- **Next unlock** — the soonest immature coinbase and how long it has left.

If the wallet scanner is behind the tip, a line says so. Network lists
peer versions from completed handshakes. Mining splits **this session**
(blocks found since start) from **lifetime mined** (confirmed coinbases
in this wallet).

`status` also reports `tip_time`.

Phone and browser wallets stay 0.7.0.

## Downloads

`NIGHTFALLCOIN-Core-0.7.7-macOS-arm64.dmg`
`NIGHTFALLCOIN-Core-0.7.7-macOS-intel.dmg`
`nightfall-core-0.7.7-windows-x64.exe`
`nightfalld-0.7.7-windows-x64.exe`
`nightfall-wallet-0.7.7-windows-x64.exe`
`nightfall-core-0.7.7-linux-x64`
`nightfalld-0.7.7-linux-x64`
`nightfall-wallet-0.7.7-linux-x64`

Verify `SHA256SUMS-0.7.7.txt`, `SHA256SUMS-0.7.7-windows.txt`,
`SHA256SUMS-0.7.7-linux.txt`.
