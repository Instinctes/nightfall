# NIGHTFALLCOIN Core Wallet 1.0.4

A mainnet full node, miner and wallet in one desktop application. Download
[v1.0.4](https://github.com/Instinctes/nightfall/releases/tag/v1.0.4) or build:

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
| Mining | Start/stop mining and set CPU threads |
| Network | Peers, connectivity, Tor and relay information |
| Settings | Backups, view key, address book and maintenance |

Core scans automatically. Mined coins unlock after 1,440 blocks; the standard
wallet payment fee is 0.001 NIGHT, burned while subsidy remains. An outgoing
connection is sufficient to sync; inbound mainnet peers use TCP 17891.

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
