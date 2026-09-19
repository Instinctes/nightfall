# NIGHTFALLCOIN 1.0.2 — Dashboard cards the right height

**This is the 1.0 release to install.** 1.0.0 and 1.0.1 were published and
superseded the same day, before any announcement and before the website offered
them. Everything in
[1.0.0](RELEASE-NOTES-v1.0.0.md) and [1.0.1](RELEASE-NOTES-v1.0.1.md) is here:
the encrypted Vault, Recovery Studio, encrypted backups, invoices and receipts,
Air offline signing, the upgrade fix for wallets with no scan anchor, and the
withdrawal of atomic swap. Same chain, protocol v8, wire v6, `n8` format.

## The Dashboard cards ran off the bottom of the window

"This node" and "Hashrate" hold four numbers each and were drawn as tall as the
window, with a field of empty glass beneath them.

Two faults, and the second one made the first permanent.

Cards that sit side by side are padded to a common height so their bottom edges
line up. That height was then measured **after** the padding — so the
measurement only ever confirmed the padding that produced it. A row that became
too tall once stayed too tall for good, and because the value lives in the
interface's own memory, it survived every later frame.

It became too tall in the first place because each row was identified by
counting the widgets drawn before it. A banner above the cards — "wallet scan
incomplete", a node error, the catching-up notice — renumbers everything below,
so the metrics row could pick up the height stored by the Activity row, which is
tall by nature. Rows are now named by what they hold rather than by where they
happen to fall, and the height reported upwards is the card's own content.

Pinned by a regression that settles a row on tall content, shrinks it, and
requires the row to shrink with it — on the old code it reports the row stuck at
408 points — and then makes a banner appear and requires the row to keep its own
height instead of inheriting its neighbour's.

## Not a bug: "the chain changed below the scan position"

If the wallet tells you the chain changed beneath it and blocks sending, that is
the scan anchor working, not a fault. The node saw a competing branch at a
height the wallet had already scanned.

The wallet does not follow that quietly and will not be changed to. A branch of
equal height is a decision with money attached — re-scanning onto whichever one
a node currently prefers is how a wallet ends up somewhere its owner never
chose. So it stops, says which block it expected and which it found, refuses to
spend, and waits for a person. If the node settles back onto the chain the
wallet knows, scanning resumes by itself. If it does not, the resolution is a
rescan from Settings, after an encrypted backup.

(This was briefly "fixed" during development by reconciling automatically. The
isolated node test refused it, correctly, and the change was reverted.)

## Download and verify

- `NIGHTFALLCOIN-Core-1.0.2-macOS-arm64.dmg` — macOS 11+
- `NIGHTFALLCOIN-Core-1.0.2-macOS-intel.dmg` — macOS 10.15+
- `nightfall-core-1.0.2-windows-x64.exe`
- `nightfall-core-1.0.2-linux-x64` — GUI dependencies required
- `nightfalld` and `nightfall-wallet` CLI binaries for Windows/Linux;
  macOS includes them inside the app bundle.

Verify against `SHA256SUMS-1.0.2.txt`, `SHA256SUMS-1.0.2-windows.txt` or
`SHA256SUMS-1.0.2-linux.txt` before running downloads. macOS apps are ad-hoc
signed, not Developer-ID signed or notarized; Windows builds have no publisher
signing certificate. A checksum verifies file integrity, not software safety.

**Back up your recovery phrase and quit Core fully before replacing the
application.**

## Validation scope

Full workspace build with zero warnings; complete suite green at 438 tests,
including the isolated node lifecycle test that covers scanning, the scan
anchor, competing branches and the payment path end to end.

The 1.0 in the version number is a statement about scope and stability of
interface, not a safety certificate. Independent review remains outstanding and
is named as such on the site.
