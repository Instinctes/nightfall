# NIGHTFALLCOIN — release builds

Protocol **v5 (Nightproof-β)** · built 2026-08-12

Mainnet genesis: `c8614333c0f86a4824df212474632f4b9feecf9bf0593841199d894127f2f9a6`

> Nodes only peer with a matching genesis hash. If yours differs, you are on a
> different chain.

| Download | Target | Minimum OS |
|----------|--------|------------|
| `NIGHTFALLCOIN-Core-macOS-arm64.dmg` | Apple Silicon (M1–M4) | macOS 12.5 |
| `NIGHTFALLCOIN-Core-macOS-intel.dmg` | Intel Macs | macOS 12.5 |
| `windows-x64/` | Windows 64-bit | Windows 10 |

Each package contains three programs:

| File | What it is |
|------|-----------|
| `nightfall-core` | **Desktop wallet.** Runs a full node inside itself — mine, send, receive, all in one window. Start here. |
| `nightfalld` | Headless full node, for servers and always-on miners. |
| `nightfall-wallet` | Command-line wallet, talks to a running `nightfalld`. |

On macOS the CLI tools ride along inside the app bundle, at
`NIGHTFALLCOIN Core.app/Contents/MacOS/`.

---

## macOS

Open the `.dmg` and drag **NIGHTFALLCOIN Core** onto Applications.

The app is **not code-signed or notarised**, so a normal double-click is
blocked. On first launch: **right-click the app → Open → Open**. Only needed
once.

Not sure which Mac you have? Apple menu → About This Mac. "Apple M…" takes the
arm64 build, "Intel" takes the intel one. The Intel build also runs on Apple
Silicon through Rosetta, but mines noticeably slower — use the native one.

## Windows

No installer and no DLLs to copy: the binaries are statically linked.

```
nightfall-core.exe --network mainnet
```

SmartScreen will warn about an unknown publisher — the binaries are unsigned.
Choose **More info → Run anyway** if you trust the source.

---

## First run

1. Launch the wallet.
2. Go to **Settings → Backup** and write the recovery seed down on paper.
   Losing it loses the coins, permanently. There is no reset and no support
   desk.
3. Press **Start mining** in the top right.
4. **Receive** shows your `nf1…` address and a QR code. Share it to get paid.

Mining uses every CPU core but one. Set `NF_MINING_THREADS=4` to limit it.
Each thread needs 32 MiB of memory — that is what makes the proof of work
ASIC-resistant.

Mined coins show as **unlocking** for 1,440 blocks (about 6 hours) before they
can be spent. That delay protects against chain reorganisations; the coins are
already yours.

The wallet scans continuously in the background. There is nothing to press to
stay in sync.

### Data location

| OS | Path |
|----|------|
| macOS | `~/Library/Application Support/nightfall/mainnet/` |
| Windows | `%APPDATA%\nightfall\mainnet\` |

`core.seed` in that folder *is* the wallet. On macOS it is created with `0600`
permissions. Back it up somewhere offline.

---

## Connecting to other nodes

The network has no seed nodes yet, so the first connection has to be made by
hand. In the wallet go to **Network → Add a peer** and enter `host:port`, for
example `1.2.3.4:17891`.

From there it is automatic: nodes exchange the addresses they know, so a single
working connection is enough to find the rest of the network. Each node also
advertises its own listening port during the handshake, so the link works in
both directions — blocks you mine propagate outward, not just inward.

To let others reach you, forward TCP port **17891** in your router.

You can also pre-seed a connection before launch:

```bash
SEED_NODE=1.2.3.4:17891 ./nightfall-core --network mainnet
```

### What happens when two people mine at once

Both nodes mine on the tip they know. When one finds a block it pushes it to
every peer, and the heavier chain — measured in **cumulative proof of work**,
not block count — wins. A node on the lighter fork downloads the competing
chain in full, verifies every block, and reorganises onto it.

Double spending is impossible within a chain: an output leaves the UTXO set the
moment it is spent, and the supply invariant is re-checked after every block.

The one thing to be careful about: **if two miners never connect, they build
two separate chains from the same genesis.** Both look perfectly valid on their
own machine. When they finally connect, the lighter chain is discarded and
everything mined on it is gone. Connect first, mine second.

Reorgs deeper than 500 blocks are refused outright, so two chains that drift
that far apart can never be merged at all.

---

## Verifying the supply yourself

Every node can prove the whole money supply in one pass:

```bash
./nightfalld --network mainnet status | grep supply_proof
# supply_proof... OK — Σ UTXO − Σ excess == circulating·G
```

The wallet shows the same check in the sidebar. If it ever reads `FAILED`, stop
the node and do not relay — under v5 that should be impossible.

---

## Before you trust this with anything

This is **pre-launch software that has not been audited by anyone outside the
project.** Read the honest limitations in [`../README.md`](../README.md) and the
audit of the previous protocol version in
[`../docs/AUDIT-2026-08-12.md`](../docs/AUDIT-2026-08-12.md).

Specifically: the transaction graph is obscured by block-level aggregation but
not erased, there is no network-layer privacy, and no third party has reviewed
the cryptography.

## Reproducing these builds

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin x86_64-pc-windows-gnu
brew install mingw-w64   # for the Windows target

./scripts/build-macos-dmgs.sh
cargo build --release --target x86_64-pc-windows-gnu \
    -p nightfall-core -p nightfall-node -p nightfall-wallet
```
