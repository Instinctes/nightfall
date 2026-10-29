# NIGHTFALLCOIN 1.1.0 — mainnet wallets

Current release: [v1.1.0](https://github.com/Instinctes/nightfall/releases/tag/v1.1.0).
Protocol v8, wire v6, mainnet genesis `061a052d…`. Same chain and data format.
**Required upgrade:** 1.1.0 activates a consensus upgrade at block 500,000.
Nodes, miners and pools must run 1.1.0 before that height; nothing has to be
migrated.

| Platform | Core download | Minimum |
|---|---|---|
| Apple Silicon | `NIGHTFALLCOIN-Core-1.1.0-macOS-arm64.dmg` | macOS 11 |
| Intel Mac | `NIGHTFALLCOIN-Core-1.1.0-macOS-intel.dmg` | macOS 10.15 |
| Windows x64 | `nightfall-core-1.1.0-windows-x64.exe` | Windows 10 |
| Linux x64 | `nightfall-core-1.1.0-linux-x64` | GTK/X11 libraries required |
| Mobile/browser | [Web wallet](https://nightfallcoin.org/wallet/) | Modern browser |

Check the matching `SHA256SUMS-1.1.0.txt`, `SHA256SUMS-1.1.0-windows.txt`
or `SHA256SUMS-1.1.0-linux.txt` before running a download. macOS bundles are
ad-hoc signed for integrity checks, not Developer-ID signed or notarized.
Windows binaries do not have a publisher signing certificate.

Since 1.0.7 every file — both DMGs included — is built by
GitHub Actions and carries a signed build provenance, so you can check which
workflow built it from which tag:
`gh attestation verify <file> --repo Instinctes/nightfall`. See
[docs/RELEASE-PROVENANCE.md](../docs/RELEASE-PROVENANCE.md). That proves where a
file came from; it does not remove the first-launch warning.

Core starts on mainnet by default and includes a full node, miner and wallet.
The macOS app bundles include the `nightfalld` and `nightfall-wallet` CLI tools;
Windows/Linux CLI tools are separate release assets, as are the Node Kit
(`nightfall-nodekit`), the Devnet till and the receipt evidence tool. Close Core completely
before replacing its app/binary. Back up your 24 words first; never delete
your data directory to update.

Data is under `nightfall/mainnet/n8/` in the platform's application-data folder.
Keep `core.seed` offline-backed-up. Receive shows your `nf1…` address and QR;
Send shows a confirmation before broadcast. Coinbase maturity is 1,440 blocks.

**Atomic swap has been withdrawn** and is not part of any wallet — desktop,
mobile or browser. A NIGHT lock cannot refund itself on a timer, so the feature
carried a permanent-loss case no implementation could remove. See
[docs/SWAP-WITHDRAWN.md](../docs/SWAP-WITHDRAWN.md). No consensus rule changed.
