# NIGHTFALLCOIN 1.0.5 — quiet recovery and a single mining reward

Core now keeps one startup screen visible while it verifies saved blocks,
downloads missing history and catches the wallet up to the node. It opens the
wallet after the node and saved scan agree for two seconds. Connection details
remain accessible when peers or archive history are unavailable; the screen
does not claim that an offline mainnet node is synchronized.

Normal background scans no longer replace the whole window with a waiting
panel. Trial decryption runs on an isolated wallet copy, then publishes a saved
batch only if the live wallet has not changed. Payments and invoice edits made
in the meantime win over stale scan work. Node status snapshots also run off
the UI thread, and routine background checks no longer flash the status light.

## Wallet fixes

- A changed scan anchor is reconciled automatically against the node's validated
  chain. This intentionally replaces 1.0.3's manual reconciliation policy.
  Pending inputs, invoices and recovery records are retained. Previously
  confirmed payments revived by a reorg are withheld from automatic relay.
- Activity offers an explicit, reviewed, one-shot retry of a withheld saved
  transaction. It creates no replacement payment, does not release reservations
  or enable automatic rebroadcast, and refuses withdrawn swap-lock payments.
- A pruned or unavailable chain is rejected before a manual rescan resets state.
  Rescans retain local invoices and the offline-signing journal.
- Air stores a request's exact signed answer with its payment reservation.
  Repeating it, including after restarting, returns the same transaction.
  Reusing its nonce with changed terms is rejected. A failed relay can be retried
  without consuming the request first.
- Encrypted wallet setup is enabled on Windows as well as Unix platforms.

## Quiet background work and mining rewards

Routine chain changes, reconciliation and scan retries no longer open warnings,
dialogs or toasts. State checks and internal logs remain, and spending stays
blocked whenever the wallet cannot establish a valid scan. Previously saved
payments can be inspected in Activity without an automatic prompt.

Each newly observed live mining reward gets one small, centered animation.
Mining no longer also flashes the balance or triggers the old incoming/output
toasts. Initial catch-up and reconciled history stay quiet; an output seen again
after a reorg does not trigger another reward during the same session.

Mining → **Reward-Sound** controls a soft synthesized bell, enabled by default.
The choice survives restarts. Audio is generated locally (about 46 kB), with no
downloaded sound assets; playback runs off the UI thread. Linux uses an installed
PipeWire, PulseAudio or ALSA player (`pw-play`, `paplay` or `aplay`).

## Web wallet

The completed glass-style web wallet lives at https://wallet.nightfallcoin.org/.
It stores an encrypted vault on the device, supports encrypted backups, and
requires an anchored mainnet scan before spending. Existing browser wallets
move through the separate, read-only exporter at
https://nightfallcoin.org/wallet/migrate/. Original browser data is retained.
The home-screen icon and standalone manifest are included for iPhone and other
supporting browsers. The browser wallet trusts its configured node's chain data;
it does not independently verify proof of work.

## Upgrade

Same chain, genesis, protocol v8 and wire v6. No chain reset is required. Quit
Core fully before replacing the application, and keep an encrypted backup.
Using Air in 1.0.5 adds an optional encrypted signing journal that older wallet
versions cannot read. Use 1.0.5 or later for snapshots containing that journal.

Downloads include macOS arm64/Intel DMGs, Windows/Linux Core, node and CLI wallet
binaries, plus platform SHA256 lists. macOS bundles are ad-hoc signed; they are
not Developer-ID signed or notarized. Windows binaries have no publisher
certificate.

Validation covers workspace tests and Clippy, an isolated Devnet lifecycle with
automatic reconciliation and stale-scan conflicts, durable Air retries, native
layout/render checks, and the browser wallet's real-WASM persistence tests.
This is not a claim that every possible defect has been ruled out.
