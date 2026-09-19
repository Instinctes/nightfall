# NIGHTFALLCOIN 1.0.4 — the 1.0 release

**This is the build to install.** 1.0.0 through 1.0.3 were each published and
replaced within hours on 19 September, before the website offered them and
before anything was announced. Everything from those builds is here: the
encrypted Vault, Recovery Studio, encrypted backups, invoices and receipts,
Air offline signing, the Dashboard card fix, the upgrade path for wallets with
no scan anchor, the way out of a changed history, and the withdrawal of atomic
swap. Same chain, protocol v8, wire v6, `n8` format — no reset, no migration,
nothing required of node operators or miners.

## The window flickered while catching up

Two causes, both of them fixed in 1.0.2 and 1.0.3 creating the conditions for
each other.

Paired cards remember the height they were last measured at, so the two cards
in a row can be padded to the same bottom edge. That remembered height was
being applied at *any* width. When the scan banner appeared the page grew
taller, the scrollbar took a few points of width, the text inside the cards
rewrapped to a different natural height, and the padding that followed changed
the page height back — once per frame, visibly. A height measured for one width
is an answer to a different question at another, so it is now discarded when
the width changes rather than reused.

The status line alternated too. A catch-up is a sequence of short scan passes,
so the "currently scanning" flag flips true and false several times a second,
and the readout swapped between "Scanning…" and "Catching up · scanned block N"
fast enough to read as flicker. While the wallet is behind, being behind is the
stable fact worth showing; a pass running is how it stops being behind, not
separate news. "Scanning…" now appears only when the wallet is at the tip and a
pass is genuinely running.

## A test that failed one run in a few

`rejected_block_leaves_the_chain_untouched` built a block, sealed it with nonce
1, and assumed that would miss the proof-of-work target. The miner address is
generated fresh on every run, so the block hash differs every run, and at
devnet difficulty nonce 1 clears the target often enough to be a coin flip.
When it cleared, the block was valid, the chain moved, and the test reported an
unreadable `Hash256 != Hash256` — in CI, while passing locally all morning.

It now searches for a nonce that is known to miss, and asserts the block is
actually rejected before checking what the rejection left behind. A fixture
that is only probably invalid proves nothing about invalid blocks.

## Download and verify

- `NIGHTFALLCOIN-Core-1.0.4-macOS-arm64.dmg` — macOS 11+
- `NIGHTFALLCOIN-Core-1.0.4-macOS-intel.dmg` — macOS 10.15+
- `nightfall-core-1.0.4-windows-x64.exe`
- `nightfall-core-1.0.4-linux-x64` — GUI dependencies required
- `nightfalld` and `nightfall-wallet` CLI binaries for Windows/Linux;
  macOS includes them inside the app bundle.

Verify against `SHA256SUMS-1.0.4.txt`, `SHA256SUMS-1.0.4-windows.txt` or
`SHA256SUMS-1.0.4-linux.txt` before running downloads. macOS apps are ad-hoc
signed, not Developer-ID signed or notarized; Windows builds have no publisher
signing certificate. A checksum verifies file integrity, not software safety.

**Back up your recovery phrase and quit Core fully before replacing the
application.**

## Validation scope

Full workspace build with zero warnings; complete suite green at 438 tests,
including the isolated node lifecycle test covering scanning, the scan anchor,
competing branches, reconciliation and the payment path end to end.

The 1.0 in the version number is a statement about scope and stability of
interface, not a safety certificate. Independent review remains outstanding and
is named as such on the site.
