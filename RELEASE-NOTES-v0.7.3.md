**The seed is a doorbell, not the network.** Same chain, genesis `061a052d…`,
protocol v8, wire v6. 0.7.0–0.7.2 still peer. 0.7.3 is the build that finds
the mesh when the seed is full, and that hangs up once you have the tip.

## What was going wrong

Every wallet dialled one machine. That process held 128 sessions and then
dropped the next TCP before Hello. A new miner sat on genesis — one block —
and looked dead. Raising the cap and kicking people off were patches on
that one box. A million miners do not fit on a 1 GiB VPS. They should
never have to.

## The protocol

A node that finished a handshake and accepts inbound is a listener. The
seed publishes only those addresses — plus the compiled-in seed names —
over `GetPeers` and over `GET https://nightfallcoin.org/peers`. Private
ranges, `.onion`, and anyone we have not ourselves reached are omitted.

A fresh node fetches that list on start (mainnet, unless
`NIGHTFALL_PEERS_URL=off`). It dials listeners, not just
`seed.nightfallcoin.org`. Port 443 works when 17891 is filtered.

An inbound peer that is at our tip is introduced: they get the list, then
the socket closes after half a minute. If the room is full, a caught-up
inbound gives up their seat before Hello is refused. Someone still
catching up is not evicted.

Phone and browser wallets are unchanged. They already talk HTTP to the
seed. This path is for full nodes.

## Downloads

`NIGHTFALLCOIN-Core-0.7.3-macOS-arm64.dmg` · Apple Silicon, macOS 11+
`NIGHTFALLCOIN-Core-0.7.3-macOS-intel.dmg` · Intel, macOS 10.15 Catalina+
`nightfall-core-0.7.3-windows-x64.exe` · Windows 10+ — the wallet
`nightfalld-0.7.3-windows-x64.exe` · Windows — headless node
`nightfall-wallet-0.7.3-windows-x64.exe` · Windows — command-line wallet

Verify against `SHA256SUMS-0.7.3.txt` and `SHA256SUMS-0.7.3-windows.txt`.
Android / iOS stay on the 0.7.0 sideload builds.
