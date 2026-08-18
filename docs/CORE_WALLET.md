# NIGHTFALLCOIN Core Wallet

User-friendly desktop app: **mine**, **receive**, **send** — no terminal required after build.

## Build

```bash
cd nightfall
cargo build --release -p nightfall-core
```

Binary: `target/release/nightfall-core`

## Start

```bash
# Mainnet (default)
./target/release/nightfall-core --network mainnet

# Devnet (faster mining, for testing)
./target/release/nightfall-core --network devnet

# Join someone’s node while mining
SEED_NODE=1.2.3.4:17891 ./target/release/nightfall-core --network mainnet
```

Or: `./scripts/start-core-wallet.sh mainnet`

## What you see

| Tab | Use |
|-----|-----|
| **Home** | Balance, Sync, quick actions |
| **Receive** | Payment ID + Copy button |
| **Send** | Paste Payment ID, amount in NIGHT, Send |
| **Mine & Network** | Mining ON/OFF, peers, blocks, tip |

Top right: **Start / Stop Mining**.

## Files (important)

Under your data folder (macOS mainnet example):

`~/Library/Application Support/nightfall/mainnet/`

| File | Meaning |
|------|---------|
| `core.seed` | **Master key — backup offline** |
| `core.addr.json` | Payment ID / address |
| `core.notes.json` | Local note cache |
| `chain.json` | Blockchain |

## Tips

1. Leave the app open while mining.  
2. After receiving coins: wait for a block → **Sync**.  
3. Fee is **0.01 NIGHT** (burned).  
4. Open **firewall port 17891** (mainnet) so others can connect.  
5. Share only **Payment ID**, never `core.seed`.

## View key and receipts

A view key is `nfview1…`. It finds every payment and opens amounts and
memos. It cannot spend. Settings → **Show view key**. CLI:
`nightfall-wallet --network mainnet export-view-key`.

To prove **one** payment without handing over the view key, copy a
**Receipt** from the Activity list, or:

```bash
nightfall-wallet --network mainnet export-receipt --txid <prefix> > payment.json
nightfall-wallet verify-receipt --file payment.json
```

Longer write-up: <https://nightfallcoin.org/view-key/>.

## CLI still available

Advanced users: `nightfalld` + `nightfall-wallet` (see MAINNET.md).
