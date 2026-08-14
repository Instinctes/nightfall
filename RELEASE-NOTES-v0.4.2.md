Three networking defects, found by running two miners and a public seed node
against each other, plus mining that refuses to run on a stale chain and Intel
builds that reach back to Catalina.

Protocol v6 is unchanged — **0.4.x releases share one chain and peer with each
other.**

```
genesis   b69d9c81892266a7b89b2e759f9cfd4d9344230b084545d4f92d648ab9eb11a1
seed      seed.nightfallcoin.org:17891
```

---

## A node behind NAT could not win a fork

Three faults stacked on top of one another, each hiding the next. Together they
produced a fork that survived indefinitely while one side carried **53 blocks of
work against 2** — and lost, by sitting still.

**Blocks that fork earlier than the tip were discarded.** `apply_block` only
extends the current tip, which is correct for what it is. What was missing is
the case one step out: a block whose parent we hold, but which branches before
our tip. Those are now collected and offered to `maybe_reorg_to` — the same
cumulative-work rule, the same full rebuild from genesis, the same validation.
Nothing is short-circuited; the existing check is simply given something to look
at.

**Pushes started at the wrong place.** A node that found a peer behind it pushed
from just below the peer's tip. On a fork, every one of those blocks has a
parent the peer has never seen, so nothing could attach and nothing could be
evaluated. The pushing side now locates the last height both sides agree on
first, by walking backwards in doubling steps.

**Branch assembly could never grow past one block.** Only the *first* block of a
branch has a parent in our own chain; every one after it descends from a block
that exists solely in the buffer. Requiring a known parent before storing meant
the run stopped at one — and one block never outweighs anything.

Verified live: a seed node holding the lighter branch logged
`reorged onto a heavier branch` and converged.

## Mining waits until the chain is current

Every block mined while behind extends a tip the network has already left. It
cannot be accepted by anyone; it only deepens a fork. And it happens at the
worst possible moment — right after the wallet opens, before the first sync
lands, which is exactly when someone presses **Start mining**.

Mining now holds until the chain is level with the best height any peer has
reported, and the dashboard says **"Catching up — N blocks behind"** with the
reason. Without that the wallet looks broken: the button reads *Stop mining* and
the hashrate sits at zero.

Mining with **no** peers stays allowed. A network of one has nothing to be
behind, and refusing would make a first node impossible; that case keeps its own
loud warning.

A peer claiming a height it does not have can pause your mining but cannot make
you accept anything invalid, and the claim expires after two minutes so a peer
that disappears cannot stall you. Pausing is the safe direction.

## Intel builds go back to macOS Catalina

The Intel build targeted macOS 12.5 for no technical reason — nothing here needs
anything newer, the GUI is OpenGL rather than Metal. It now targets **10.15**,
and the build script verifies with `vtool` what the binary actually declares
rather than trusting the flag it was given.

Apple Silicon stays at 11.0: ARM Macs did not exist before Big Sur, so a lower
figure would be a claim about machines that never shipped.

---

## Downloads

| File | Platform |
|------|----------|
| `NIGHTFALLCOIN-Core-0.4.2-macOS-arm64.dmg` | Apple Silicon, **macOS 11+** |
| `NIGHTFALLCOIN-Core-0.4.2-macOS-intel.dmg` | Intel, **macOS 10.15 Catalina+** |
| `nightfall-core-0.4.2-windows-x64.exe` | Windows 10+ — the wallet |
| `nightfalld-0.4.2-windows-x64.exe` | Windows — headless node |
| `nightfall-wallet-0.4.2-windows-x64.exe` | Windows — command-line wallet |

Verify against `SHA256SUMS-0.4.2.txt` and `SHA256SUMS-0.4.2-windows.txt`:

```bash
shasum -a 256 -c SHA256SUMS-0.4.2.txt                           # macOS / Linux
certutil -hashfile nightfall-core-0.4.2-windows-x64.exe SHA256   # Windows
```

**Not code-signed.** macOS: right-click → **Open** → **Open**, once.
Windows: **More info → Run anyway**. The Windows binaries are built by
[a published workflow](https://github.com/Instinctes/nightfall/blob/main/.github/workflows/release.yml)
on GitHub's runners rather than on a laptop.

## First run

1. Open the wallet. It finds the network by itself — measured from an empty data directory: 499 blocks in seven seconds.
2. **Settings → Backup.** Write the 24 words on paper *before* you mine anything. There is no reset and no support desk.
3. Press **Start mining**. If the chain is still catching up, mining begins on its own once it is level.

Mined coins show as *unlocking* for 1,440 blocks (~6 h). They are already yours;
the delay protects against reorganisations.

## Honest status

Pre-launch software, not reviewed by anyone outside the project. The
cryptography was written and reviewed by the same party — a conflict of interest
that 134 passing tests do not resolve. There is one seed node, so *discovery*
has a single point of failure (no seed can forge or hide a block). Graph privacy
is partial and there is no network-layer privacy.

Every fault fixed in this release was found by running real nodes against each
other, not by reading the code. Assume there are more.

Do not put value on this that you cannot afford to lose entirely.

---

**Website:** https://nightfallcoin.org
