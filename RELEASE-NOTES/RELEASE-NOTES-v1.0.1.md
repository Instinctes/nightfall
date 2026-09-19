# NIGHTFALLCOIN 1.0.1 — the 1.0 release, with the upgrade path fixed

**Use this instead of 1.0.0.** 1.0.0 was published and withdrawn within hours,
before it was announced and before it appeared on the website. If you installed
it, replace it with this build. Nothing was lost — see below.

Everything in [1.0.0](RELEASE-NOTES-v1.0.0.md) is here: the encrypted Vault,
Recovery Studio, encrypted backups, invoices and receipts, Air offline signing,
the rebuilt window, and the withdrawal of atomic swap. Same chain, protocol v8,
wire v6, `n8` data format. No reset, no migration, no seed-node upgrade.

## What 1.0.0 got wrong

**An upgraded wallet stopped scanning, silently.**

1.0 introduced a scan anchor: the hash of the block a scan ended on, stored so
the wallet can tell whether the chain moved underneath it. Wallets written by
0.9.5 have no such field, because it did not exist yet — which is every wallet
in existence.

`scan_blocks` refuses an incremental page from a wallet with no anchor, and
that refusal is correct: a page that begins at the scan position carries no
evidence at all about the history below it, so accepting one would let a wallet
adopt a chain it was never on. What was wrong is what happened next. The
application kept asking for the same incremental page every thirty seconds,
kept being refused, and reported nothing worse than "Catching up". The scan
position never moved again.

Symptom: the balance and Activity stay correct but frozen at the moment of the
upgrade, the node keeps advancing normally, and the wallet line reads
`Wallet scan N / M — coins in the last … blocks are still being looked for`
with N never changing.

**No coins were ever at risk and nothing was written.** The refusal happened
before any wallet save, which is also why every affected wallet file is still
byte-for-byte what 0.9.5 left behind.

## The fix

A wallet with no anchor is now asked for the one range that can give it one: a
single canonical pass from its birth height. That is the repair the wallet
already implements and the only one it treats as sound — absence of a payment
means absence only when you have looked at the whole history — and it writes
the anchor on its way out. Every page after it is an ordinary one.

The pass reads the chain once, so the first launch after upgrading takes longer
than usual and uses more memory while it runs. After that, scanning is exactly
as before. On a pruned node the pass is impossible and the wallet now says so
instead of stalling.

Two regressions pin it: one that the range offered to an anchorless wallet is
the canonical one and that every other wallet keeps paging, and one that walks
a real wallet through the whole path — history without an anchor, an
incremental page still refused, the canonical pass accepted, the anchor
present, the balance identical to a wallet that scanned the same chain from
scratch, and ordinary incremental scanning working afterwards.

## Also fixed

Two failures that only appeared off macOS, found by CI on the 1.0.0 tag:

- `feedback.rs` kept an import and a guard outside the macOS-only block, so on
  Linux the block compiled away and left an unused import above a function
  whose entire body was a bare `return`. Clippy rejects both.
- A vault storage test read its own directory while the store was still open.
  Byte-range locks are advisory on Unix and mandatory on Windows, so the read
  failed there and nowhere else.

Neither changed shipped behaviour; both would have turned CI red on every
future push.

## If you installed 1.0.0

Replace the application and open it. The canonical pass runs once and the scan
catches up. There is nothing to restore and no rescan to request — and if you
had already gone back to 0.9.5, that also worked, because 1.0.0 never wrote to
the wallet file.

## Download and verify

- `NIGHTFALLCOIN-Core-1.0.1-macOS-arm64.dmg` — macOS 11+
- `NIGHTFALLCOIN-Core-1.0.1-macOS-intel.dmg` — macOS 10.15+
- `nightfall-core-1.0.1-windows-x64.exe`
- `nightfall-core-1.0.1-linux-x64` — GUI dependencies required
- `nightfalld` and `nightfall-wallet` CLI binaries for Windows/Linux;
  macOS includes them inside the app bundle.

Verify against `SHA256SUMS-1.0.1.txt`, `SHA256SUMS-1.0.1-windows.txt` or
`SHA256SUMS-1.0.1-linux.txt` before running downloads. macOS apps are ad-hoc
signed, not Developer-ID signed or notarized; Windows builds have no publisher
signing certificate. A checksum verifies file integrity, not software safety.

**Back up your recovery phrase and quit Core fully before replacing the
application.**

## Validation scope

Full workspace build with zero warnings; complete suite green at 437 tests.
The two new regressions above; ledger, consensus, storage and wallet
regressions; Core page-layout tests at three content widths; contrast tests
measured against the composited background; crash-injection tests on the vault
persistence adapter on Unix and Windows; an isolated node lifecycle test
covering the payment path end to end.

The upgrade path itself was also exercised against a real mainnet wallet that
had been left frozen by 1.0.0, rather than only in a fixture.

These are bounded checks, not an independent security audit. The 1.0 in the
version number is a statement about scope and stability of interface, not a
safety certificate. Independent review remains outstanding and is named as such
on the site.
