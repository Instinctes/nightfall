**Same chain.** Genesis `061a052d…`, protocol v8, wire v6. 0.7.0–0.7.3
still peer. This build is the one that tells you why PEERS is 0, starts
answering before the chain file is in memory, and catches up from more
than one listener at a time.

## What changed

The seed used to bind P2P, RPC and the phone API only after it had
replayed every block on disk. Eleven thousand blocks is five minutes.
Cloudflare returned 520, phones showed nothing, and a wallet that
restarted looked dead. RPC and the light API now bind immediately and
report the last saved tip with `loading: true`. P2P waits until the
real chain is in memory, so the seed does not advertise genesis.

Catch-up asked every peer for the same 128-block page. Two live
listeners now fetch different slices; pages that arrive early sit in a
buffer until the hole fills.

A node with PEERS=0 used to say only that. Core now says why: still
loading, last dial failed, Windows is blocking 17891, or the compiled
seed is full (0.7.3+ then asks `https://nightfallcoin.org/peers`).
Mining alone stays a hard warning. The dashboard estimates time to
the next block from your hashrate × current difficulty.

## Linux

A headless node, not a seed:

```
curl -fsSL https://nightfallcoin.org/scripts/install-node-linux.sh -o install-node.sh
less install-node.sh
sudo bash install-node.sh
```

`--mine` if you want it to hold a key. The public doorbell is still
`install-seed-node-linux.sh`. Both refuse to start unless genesis is
`061a052d49607ff8f4b306c75d622ebd230cff4ec3a45a6dffc2f7738d4b20de`.

Prebuilt `nightfalld` / `nightfall-wallet` / `nightfall-core` for
Linux x64 come off the same GitHub Actions run as the Windows
binaries. Verify `SHA256SUMS-0.7.4-linux.txt`.

## Snapshot

```
nightfalld --network mainnet export-snapshot --out /tmp/nf-snap
nightfalld --network mainnet --datadir /new import-snapshot --from /tmp/nf-snap
```

The importer re-checks proof of work and the supply invariant. This is
not a trust shortcut, and the file is not hosted on the website.

## Also

Numbers-only page: https://nightfallcoin.org/network/ — height,
difficulty, supply, listener count. No addresses.
How to build and check a SHA-256: https://nightfallcoin.org/build/
`nfview1` sees, cannot spend; a receipt proves one payment:
https://nightfallcoin.org/view-key/

## Downloads

`NIGHTFALLCOIN-Core-0.7.4-macOS-arm64.dmg` · Apple Silicon, macOS 11+
`NIGHTFALLCOIN-Core-0.7.4-macOS-intel.dmg` · Intel, macOS 10.15 Catalina+
`nightfall-core-0.7.4-windows-x64.exe` · Windows 10+ — the wallet
`nightfalld-0.7.4-windows-x64.exe` · Windows — headless node
`nightfall-wallet-0.7.4-windows-x64.exe` · Windows — command-line wallet
`nightfall-core-0.7.4-linux-x64` · Linux x86-64 — Core (needs GTK)
`nightfalld-0.7.4-linux-x64` · Linux x86-64 — headless node
`nightfall-wallet-0.7.4-linux-x64` · Linux x86-64 — command-line wallet

Verify against `SHA256SUMS-0.7.4.txt`, `SHA256SUMS-0.7.4-windows.txt`
and `SHA256SUMS-0.7.4-linux.txt`. Android / iOS stay on the 0.7.0
sideload builds.
