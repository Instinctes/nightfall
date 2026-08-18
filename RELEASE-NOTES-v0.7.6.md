**Same chain.** Genesis `061a052d…`, protocol v8, wire v6. 0.7.0–0.7.5
still peer. 0.7.6 is the build that leaves a one-block fork in seconds,
and that prints its own version under MAINNET v8.

## A four-block fork took ten minutes

You mine (or accept) one block the seed does not have. GetBlocks from
the seed do not connect. Mining stops — correctly — until the reorg
lands. 0.7.2 already measured rewind as one block and skipped
re-hashing the shared prefix. It still ran range proofs, signatures
and a ledger clone on every shared height. Thirteen thousand blocks is
about ten minutes. The seed keeps moving. The wallet says “1 block
behind” the whole time.

Shared history is already ours. A reorg now replays that prefix the
same way a trusted restart does — linkage and the UTXO set, then one
supply check — and only fully verifies the suffix. After the prefix,
the rebuilt roots must still match the last trusted header. A four-block
fork against a seed a hundred blocks ahead is seconds.

The sidebar shows the wallet build under the network badge, so a
screenshot tells you whether the miner is on this fix.

`status` now includes `mining`, `hashes_total` and `blocks_found`.
Headless `nightfalld` miners were scraping nothing official; the GUI
already had those numbers.

Phone and browser wallets are unchanged. They talk to the seed over
the light API and never ran this path.

## Downloads

`NIGHTFALLCOIN-Core-0.7.6-macOS-arm64.dmg` · Apple Silicon, macOS 11+
`NIGHTFALLCOIN-Core-0.7.6-macOS-intel.dmg` · Intel, macOS 10.15 Catalina+
`nightfall-core-0.7.6-windows-x64.exe` · Windows 10+ — the wallet
`nightfalld-0.7.6-windows-x64.exe` · Windows — headless node
`nightfall-wallet-0.7.6-windows-x64.exe` · Windows — command-line wallet
`nightfall-core-0.7.6-linux-x64` · Linux x86-64 — Core (needs GTK)
`nightfalld-0.7.6-linux-x64` · Linux x86-64 — headless node
`nightfall-wallet-0.7.6-linux-x64` · Linux x86-64 — command-line wallet

Verify `SHA256SUMS-0.7.6.txt`, `SHA256SUMS-0.7.6-windows.txt`,
`SHA256SUMS-0.7.6-linux.txt`. Phone wallets stay 0.7.0.
