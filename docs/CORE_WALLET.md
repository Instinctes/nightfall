# NIGHTFALLCOIN Core Wallet 1.0.5

A mainnet full node, miner and wallet in one desktop application. Download
[v1.0.5](https://github.com/Instinctes/nightfall/releases/tag/v1.0.5) or build:

```bash
cargo build --locked --release -p nightfall-core
./target/release/nightfall-core --network mainnet
```

Verify the platform checksum list before installation. Quit Core completely
before replacing its application; closing to the tray can leave it running.
Keep your data directory and back up the 24 recovery words. No reset or data
migration is required from 0.9.2.

## Pages

| Page | Use |
|---|---|
| Dashboard | Balance, recent movements and node health |
| Send | Recipient address, NIGHT amount, memo and full-address review |
| Receive | Share an nf1 address or QR code |
| Activity | Payments, mining rewards, confirmations and receipts |
| Mining | Start/stop mining, CPU threads and persistent Reward-Sound setting |
| Network | Peers, connectivity, Tor and relay information |
| Settings | Backups, view key, address book and maintenance |

Core scans automatically. Mined coins unlock after 1,440 blocks; the standard
wallet payment fee is 0.001 NIGHT, burned while subsidy remains. An outgoing
connection is sufficient to sync; inbound mainnet peers use TCP 17891.

At startup one progress screen stays visible until the node and wallet agree.
Later scans run in the background without replacing the page. A changed scan
anchor is repaired automatically; no repeated rescan confirmation is needed.
If history is pruned or peers are unavailable, use Connection details to inspect
the cause. Core cannot reconstruct missing archive blocks from wallet files.

Routine reorgs and scan retries stay silent in the interface while internal logs
and spending checks remain active. A new live mining reward has one centered
animation, without the old balance flash and duplicate toast. Catch-up history
stays quiet. Mining → **Reward-Sound** enables or disables the synthesized bell;
it is on by default and the selection survives restarts. Linux playback needs
`pw-play`, `paplay` or `aplay` installed.

Payments made pending by a reorg or imported backup stay withheld. Activity
allows reviewing and retrying the exact saved transaction once, without signing
a second payment or enabling automatic relay. A rescan does not cancel it.
Air remembers the exact signed answer to each request inside the wallet;
snapshots with this journal require version 1.0.5 or newer.

## Wallet data

The active chain writes to `nightfall/mainnet/n8/` under the platform data root.
On macOS this is `~/Library/Application Support/nightfall/mainnet/n8/`.
`core.seed` is key material: back it up offline and never share it. Keep other
wallet state files during updates. Blockchain storage uses `blocks.bin`.
Maintenance operations may require a rescan; do not delete data to upgrade.

A view key (`nfview1…`) can read amounts and memos but cannot spend. Use
Settings → Show view key, or `nightfall-wallet --network mainnet export-view-key`.
To prove only one payment, prefer its Receipt in Activity. Never share a seed
phrase in a bug report.

## Atomic swap

Withdrawn before 1.0.4 and removed from the wallet. There is no Swap page and
no bitcoind configuration. The reasoning is in
[SWAP-WITHDRAWN.md](SWAP-WITHDRAWN.md). A wallet file written by an
experimental build still opens normally; rescan and backup restore refuse while
it carries swap journal records, which is intentional.
