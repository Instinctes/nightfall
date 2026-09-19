# NIGHTFALLCOIN 1.0.0 — mainnet wallets

The first 1.0 Core release for macOS Apple Silicon, Intel macOS, Windows x64
and Linux x64. This replaces 0.9.5 as the current download.

**Same chain.** Protocol v8, wire v6, `n8` data format, unchanged emission and
genesis. No reset, no migration, no seed-node upgrade. If you run a node or
mine, nothing is required of you.

## Encrypted Vault

Keys now live in an authenticated encrypted store instead of a plaintext seed
file. One shared vault engine serves the desktop wallet and the browser, with
a versioned envelope, bounded parsing, a fixed KDF cost and authenticated
network context. An unlocked session owns the keys; locking removes the
application's access to them.

Migration from a plaintext wallet is deliberately two-stage and failure-safe:
the encrypted copy is written and verified in full before any legacy secret is
retired, and an interrupted retirement resumes from its surviving files. If the
old and new state disagree, neither original is removed. Encryption at rest is
not protection against hostile code on the machine, a compromised device, every
memory copy, or backups you made earlier.

Automatic locking on loss of window focus and after inactivity. Password
rotation without re-creating the wallet. Old backups keep their old passwords.

## Recovery Studio and encrypted backups

Rehearse your 24 words against the wallet you actually hold: the check derives
the address locally, compares it, and never writes a file, creates a wallet or
sends anything. A different valid phrase is refused by name rather than
silently accepted.

Encrypted `.nfv` backups can be exported and read back for a password,
identity and full-state comparison without overwriting anything. Recovery
offers two explicit modes and defaults to keys-only. A restored state that
could still broadcast an old payment is refused at two independent gates, and
withheld payments are shown in their own notice rather than mixed into the
stuck-payment one.

## The rest of the wallet

Invoices and receipts, the address book, view keys, Air offline signing for a
machine that is never on a network, mining with a CPU-thread control, and a
rebuilt window: a rounded glass plate with the navigation rail standing off its
left edge, working traffic-light buttons, and text measured against the
background it is actually drawn on rather than against a colour that is never
displayed.

## Atomic swap has been withdrawn

The NIGHT ↔ BTC atomic swap was built, reviewed, and removed before this
release. It is not disabled — it is gone, along with its crate, its page, its
bitcoind configuration and 21 third-party packages including all of
`secp256k1`, the `bitcoin` crate family, and a second implementation of the
same curve arithmetic the chain already uses.

Nightfall has no script language, so a NIGHT lock cannot refund itself on a
timer the way a Bitcoin lock can. A counterparty who cancels and walks away
leaves NIGHT locked permanently; a redeem published too late discloses its
secret while the cancel confirms, and one side can end up with both assets.
Neither is reachable by better implementation work. Shipping that unaudited was
not a trade worth making.

No user was ever exposed: swaps were blocked on mainnet in code, in every
published build. No consensus rule changed — there is no fork and no migration.
A wallet file written by an experimental build still opens; rescan and restore
stay blocked while it carries swap records, and an abandoned swap lock is never
rebroadcast on its own.

Reasoning: [`docs/SWAP-WITHDRAWN.md`](../docs/SWAP-WITHDRAWN.md).

## Web wallet — unchanged in this release

**The browser wallet stays where it is, at `nightfallcoin.org/wallet/`.** This
release is desktop Core only. Nothing about your browser wallet changes, and
there is nothing for you to move.

`wallet.nightfallcoin.org` exists and is reserved for it. A browser separates
stored keys by origin and by nothing smaller, so moving the wallet to its own
host is worth doing — but the build prepared there handles keys, backups and
recovery and cannot yet make a payment. Shipping it would trade a wallet that
pays for one that does not, so it waits.

When the move happens it will be announced, and the old address will hand your
wallet across before it stops serving one. Until then, the wallet at
`/wallet/` remains a light wallet that trusts a node for chain data, with
browser storage that can be cleared or lost.

## Download and verify

- `NIGHTFALLCOIN-Core-1.0.0-macOS-arm64.dmg` — macOS 11+
- `NIGHTFALLCOIN-Core-1.0.0-macOS-intel.dmg` — macOS 10.15+
- `nightfall-core-1.0.0-windows-x64.exe`
- `nightfall-core-1.0.0-linux-x64` — GUI dependencies required
- `nightfalld` and `nightfall-wallet` CLI binaries for Windows/Linux;
  macOS includes them inside the app bundle.

Verify against `SHA256SUMS-1.0.0.txt`, `SHA256SUMS-1.0.0-windows.txt` or
`SHA256SUMS-1.0.0-linux.txt` before running downloads. macOS apps are ad-hoc
signed, not Developer-ID signed or notarized; Windows builds have no publisher
signing certificate. A checksum verifies file integrity, not software safety.

**Back up your recovery phrase and quit Core fully before replacing the
application.**

## Validation scope

Full workspace build with zero warnings and the complete test suite green.
Ledger, consensus, storage and wallet regressions; Core page-layout tests at
three content widths; contrast tests measured against the composited
background; crash-injection tests on the vault persistence adapter on Unix and
Windows; an isolated node lifecycle test covering the payment path end to end.

These are bounded checks, not an independent security audit, and not every
operating system, screen reader, populated history or physical device. The
1.0 in the version number is a statement about scope and stability of
interface, not a safety certificate. Independent review remains outstanding and
is named as such on the site.
