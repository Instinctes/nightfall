# NIGHTFALLCOIN 1.0.3 — a way out of a changed history

**This is the 1.0 release to install.** 1.0.0 through 1.0.2 were published and
superseded the same day, before any announcement and before the website offered
them. Everything from those builds is here: the encrypted Vault, Recovery
Studio, encrypted backups, invoices and receipts, Air offline signing, the
upgrade fix for wallets with no scan anchor, the Dashboard card fix, and the
withdrawal of atomic swap. Same chain, protocol v8, wire v6, `n8` format.

## The wallet could get stuck with no way forward

When the chain changes below the position a wallet has already scanned — a
competing branch of the same height, which on a fifteen-second chain is weather
rather than catastrophe — the wallet stops, says which block it expected and
which it found, and refuses to spend.

That refusal is correct and stays. A branch of equal height is a decision with
money attached, and a wallet that quietly re-scans onto whichever branch its
node currently prefers has made that decision on its owner's behalf.

What was missing was the other half: any way for the owner to make it. The
banner said "reconcile the wallet before sending" and there was nothing to
press. The documented route, a rescan from Settings, is refused while any
payment is pending — and pending payments are exactly what a reorg calls into
question. A wallet in that state could not scan, could not rescan, and could not
send. It simply stopped, a hundred blocks behind its own node, and said
"catching up".

## What is new

The scan warning now offers **Reconcile with this chain**, and only when the
anchor has actually broken. It explains itself before it acts and asks once more
before it does:

> This reads the whole chain your node follows and makes this wallet agree with
> it: anything that chain does not contain is dropped, and a payment whose
> inputs it no longer spends goes back to unconfirmed. Nothing is broadcast.
> Export an encrypted backup first — this cannot be undone from inside the
> wallet.

It runs the same canonical pass a rescan ends in, on the scan worker rather than
the interface thread, without discarding what is already known and without the
no-pending-payments precondition. Reconciling is not a rebroadcast: nothing is
put back on the wire by it.

Pinned end to end in the isolated node test, which now builds two real competing
branches, confirms the wallet refuses to follow the second one by itself,
reconciles on request, and goes back to ordinary scanning afterwards — and that
the offer is absent when there is nothing to reconcile.

## If your wallet is stuck right now

Install this build, open the **Wallet scan incomplete** banner, export an
encrypted backup from Settings, then press **Reconcile with this chain**. It
reads the whole chain once, so it takes a while; the scan resumes by itself
afterwards.

If the banner does not offer it, the wallet is not in that state and an ordinary
scan will catch up on its own.

## Download and verify

- `NIGHTFALLCOIN-Core-1.0.3-macOS-arm64.dmg` — macOS 11+
- `NIGHTFALLCOIN-Core-1.0.3-macOS-intel.dmg` — macOS 10.15+
- `nightfall-core-1.0.3-windows-x64.exe`
- `nightfall-core-1.0.3-linux-x64` — GUI dependencies required
- `nightfalld` and `nightfall-wallet` CLI binaries for Windows/Linux;
  macOS includes them inside the app bundle.

Verify against `SHA256SUMS-1.0.3.txt`, `SHA256SUMS-1.0.3-windows.txt` or
`SHA256SUMS-1.0.3-linux.txt` before running downloads. macOS apps are ad-hoc
signed, not Developer-ID signed or notarized; Windows builds have no publisher
signing certificate. A checksum verifies file integrity, not software safety.

**Back up your recovery phrase and quit Core fully before replacing the
application.**

## Validation scope

Full workspace build with zero warnings; complete suite green at 438 tests,
including the isolated node lifecycle test that now covers competing branches,
the refusal, and the reconciliation.

The 1.0 in the version number is a statement about scope and stability of
interface, not a safety certificate. Independent review remains outstanding and
is named as such on the site.
