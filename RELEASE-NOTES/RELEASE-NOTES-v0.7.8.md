**Same chain.** Genesis `061a052d…`, protocol v8, wire v6. 0.7.0–0.7.7
still peer.

## The wallet stopped lying about forks

A node on a competing tip used to say **1 block behind**. That number was
a mining hold, not a distance. Stuck for days looked like one block late,
and upgrading did nothing once the fork was past 500 blocks.

0.7.8 says what is actually happening:

- **Catching up — N blocks behind** when you really are late.
- **On a competing tip** while a reorg is running.
- **Stuck on a dead branch** when the fork is deeper than the reorg
  limit. Settings → **Resync chain, keep wallet** moves `blocks.jsonl`
  aside and downloads the live chain. Keys stay. Coinbase on the
  abandoned tip does not come back.

Network copies the full tip hash. **Same chain as the seed?** asks
`nightfallcoin.org/network.json` (the site sees that you checked).
`/network.json` now includes `tip`.

`status` reports `stalled_on_fork`, `reorg_in_flight`,
`best_peer_height`, `fork_rewind`, `hashrate`, `mining_threads`.

## 24 words, restore, first run

Core used to show a hex seed. Phone and browser already used BIP-39.
Same 32 bytes, same 24 words. Settings → **Reveal 24 words**. New
installs pick create or restore before the node starts. Existing
wallets get a reminder until you tick that you wrote them down.

These words are not a Bitcoin seed.

## The rest of the desktop

- Mining **CPU threads** on the Mining page (32 MiB RAM each). Lands on
  the next template. `NF_MINING_THREADS` still wins if set.
- **Close to tray** — mining keeps running. Windows Show / Quit.
- **Address book** for `nf1` labels you send to. Local only.
- About no longer claims proof of work is not memory-hard. It is
  Argon2id 32 MiB.

Phone and browser wallets stay 0.7.0.

## Downloads

`NIGHTFALLCOIN-Core-0.7.8-macOS-arm64.dmg`
`NIGHTFALLCOIN-Core-0.7.8-macOS-intel.dmg`
`nightfall-core-0.7.8-windows-x64.exe`
`nightfalld-0.7.8-windows-x64.exe`
`nightfall-wallet-0.7.8-windows-x64.exe`
`nightfall-core-0.7.8-linux-x64`
`nightfalld-0.7.8-linux-x64`
`nightfall-wallet-0.7.8-linux-x64`

Verify `SHA256SUMS-0.7.8.txt`, `SHA256SUMS-0.7.8-windows.txt`,
`SHA256SUMS-0.7.8-linux.txt`.
