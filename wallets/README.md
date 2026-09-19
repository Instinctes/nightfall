# NIGHTFALLCOIN 1.0.3 — mainnet wallets

Current release: [v1.0.3](https://github.com/Instinctes/nightfall/releases/tag/v1.0.3).
Protocol v8, wire v6, mainnet genesis `061a052d…`. Same chain and data format as
0.9.2. No reset, migration or seed service upgrade is required.

| Platform | Core download | Minimum |
|---|---|---|
| Apple Silicon | `NIGHTFALLCOIN-Core-1.0.3-macOS-arm64.dmg` | macOS 11 |
| Intel Mac | `NIGHTFALLCOIN-Core-1.0.3-macOS-intel.dmg` | macOS 10.15 |
| Windows x64 | `nightfall-core-1.0.3-windows-x64.exe` | Windows 10 |
| Linux x64 | `nightfall-core-1.0.3-linux-x64` | GTK/X11 libraries required |
| Mobile/browser | [Web wallet](https://nightfallcoin.org/wallet/) | Modern browser |

Check the matching `SHA256SUMS-1.0.3.txt`, `SHA256SUMS-1.0.3-windows.txt`
or `SHA256SUMS-1.0.3-linux.txt` before running a download. macOS bundles are
ad-hoc signed for integrity checks, not Developer-ID signed or notarized.
Windows binaries do not have a publisher signing certificate.

Core starts on mainnet by default and includes a full node, miner and wallet.
The macOS app bundles include the `nightfalld` and `nightfall-wallet` CLI tools;
Windows/Linux CLI tools are separate release assets. Close Core completely
before replacing its app/binary. Back up your 24 words first; never delete
your data directory to update.

Data is under `nightfall/mainnet/n8/` in the platform's application-data folder.
Keep `core.seed` offline-backed-up. Receive shows your `nf1…` address and QR;
Send shows a confirmation before broadcast. Coinbase maturity is 1,440 blocks.

**Atomic swap has been withdrawn** and is not part of any wallet — desktop,
mobile or browser. A NIGHT lock cannot refund itself on a timer, so the feature
carried a permanent-loss case no implementation could remove. See
[docs/SWAP-WITHDRAWN.md](../docs/SWAP-WITHDRAWN.md). No consensus rule changed.
