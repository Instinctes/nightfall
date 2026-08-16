# Nightfall 0.7.0 — new genesis

This is a **chain reset**. Coins mined on `c8614333…` (0.6.x) do not exist
here. The archive is [docs/HISTORY.md](docs/HISTORY.md).

- Protocol v8 · wire v6 · magic `NFL2`
- Genesis `061a052d49607ff8f4b306c75d622ebd230cff4ec3a45a6dffc2f7738d4b20de`
- 6 NIGHT per block, halving every 7,500,000 blocks, 90 M cap
- ~3.6 years to 50 %, ~23.5 years to 89 M
- Fees burn during the subsidy; after it ends they go to the miner
- Tor on by default; clearnet if Tor is down
- Datadir `nightfall/<network>/n8/`

Build 0.7.0. 0.6.x cannot peer.

Browser wallet: https://nightfallcoin.org/wallet/ (PWA, seed stays in
the browser). iOS IPA: `NIGHTFALLCOIN-0.7.0-ios-arm64.ipa` (unsigned,
sideload).
