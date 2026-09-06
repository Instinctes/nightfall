# NIGHTFALLCOIN Core Wallet 0.9.5

A mainnet full node, miner and wallet in one desktop application. Download
[v0.9.5](https://github.com/Instinctes/nightfall/releases/tag/v0.9.5) or build:

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
| Swap | Experimental NIGHT/BTC trades; disabled on mainnet |
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
phrase or swap `.secret` file in a bug report.

## Experimental swap boundary

Use explicit `--network devnet` or `--network testnet` only with test coins.
Mainnet swaps stay blocked. There is no independent timed NIGHT refund; if Bob
never publishes his Bitcoin refund, Alice's NIGHT can remain locked permanently.
Back up per-swap secrets as well as the seed, and keep Core and both nodes online
until settlement/recovery finishes. See [operator notes](SWAP.md) and
[loss cases](SWAP-LOSS.md). The web wallet does not implement atomic swaps.
