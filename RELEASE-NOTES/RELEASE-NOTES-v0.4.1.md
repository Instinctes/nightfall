Networking fixes found by standing up the first public seed node, plus a
rewritten README. Protocol v6 is unchanged — **v0.4.0 and v0.4.1 are on the
same chain and peer with each other.**

```
genesis   b69d9c81892266a7b89b2e759f9cfd4d9344230b084545d4f92d648ab9eb11a1
seed      seed.nightfallcoin.org:17891
```

## A node now sends blocks, instead of only asking for them

A node caught up by dialling out and requesting blocks. When it found a peer
that was *behind*, it noted the gap and did nothing.

That works while everyone is reachable, and fails the moment one side is behind
NAT — which is most home connections. The peer that needs blocks cannot fetch
them, and the peer that has them never offers. The first seed node hit this
within minutes of going live: it had a peer, it reported healthy, it learned a
dial-back address it could never reach, and it sat at height 0 while the miner
it was connected to kept mining. Nothing in either log said anything was wrong.

A node that finds a peer behind it now feeds them over the connection already
open, capped at 256 blocks per round. Blocks arrive through the peer's normal
inbound path, so validation is unchanged.

## Peers survive a restart

Known addresses were held only in memory. The wallet tells someone with no
peers to add one; they add it, they quit, and the next launch knows nobody and
mines alone — the exact failure this network has already paid for twice, with a
UI that walks people into it.

Addresses are now written to `peers.json` in the data directory and reloaded at
startup alongside the compiled-in seed.

## Honest sync status

The dashboard showed a green "In sync" next to a warning reading "not connected
to anyone". It was reporting whether scanning had finished, which says nothing
about agreeing with a network when there is no network. With no peers it now
reads `No peers · local block N`.

## Fork choice compares work before length

A longer but lighter chain was rejected as `ReorgTooDeep` — reading as "this
looks like an attack" when the truth was "we compared the work and yours is
lighter". The depth bound is a denial-of-service limit, not a rule about which
chain is correct, and it now applies only to candidates claiming to be heavier.

---

## Downloads

| File | Platform |
|------|----------|
| `NIGHTFALLCOIN-Core-0.4.1-macOS-arm64.dmg` | Apple Silicon, macOS 12.5+ |
| `NIGHTFALLCOIN-Core-0.4.1-macOS-intel.dmg` | Intel Macs, macOS 12.5+ |
| `nightfall-core-0.4.1-windows-x64.exe` | Windows 10+ — the wallet |
| `nightfalld-0.4.1-windows-x64.exe` | Windows — headless node |
| `nightfall-wallet-0.4.1-windows-x64.exe` | Windows — command-line wallet |

Verify against `SHA256SUMS-0.4.1.txt` and `SHA256SUMS-0.4.1-windows.txt`.

```bash
shasum -a 256 -c SHA256SUMS-0.4.1.txt                          # macOS / Linux
certutil -hashfile nightfall-core-0.4.1-windows-x64.exe SHA256  # Windows
```

**Not code-signed.** macOS: right-click → **Open** → **Open**, once.
Windows: **More info → Run anyway**. The Windows binaries come from
[a published workflow](https://github.com/Instinctes/nightfall/blob/main/.github/workflows/release.yml)
on GitHub's runners rather than from a laptop.

## First run

1. Open the wallet. It finds the network by itself — verified from a clean data directory: 499 blocks in seven seconds.
2. **Settings → Backup.** Write the 24 words on paper *before* you mine anything. There is no reset and no support desk.
3. Press **Start mining**.

Mined coins show as *unlocking* for 1,440 blocks (~6 h). They are already
yours; the delay protects against reorganisations.

> If the wallet warns that it is mining with **no peers**, stop and fix that
> first. Two miners who never meet build two separate chains, and everything
> mined on the lighter one is lost when they finally connect.

## Honest status

Pre-launch software, not reviewed by anyone outside the project. The
cryptography was written and reviewed by the same party — a conflict of
interest that 134 passing tests do not resolve. There is one seed node, so
network discovery currently has a single point of failure for *discovery*
(no seed can forge or hide a block). Graph privacy is partial and there is no
network-layer privacy.

Full detail in [the README](https://github.com/Instinctes/nightfall#honest-status)
and [docs/AUDIT-2026-08-12.md](https://github.com/Instinctes/nightfall/blob/main/docs/AUDIT-2026-08-12.md).

Do not put value on this that you cannot afford to lose entirely.

---

**Website:** https://nightfallcoin.org
