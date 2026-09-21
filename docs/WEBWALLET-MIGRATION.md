# Browser wallet origin migration — 1.0.4

The production destination is **https://wallet.nightfallcoin.org/**. Browser
storage belongs to an origin. The new origin cannot read the old wallet stored
at `https://nightfallcoin.org`, and the website must not pretend a redirect moves
that data.

Existing 0.9.5 holders use **https://nightfallcoin.org/wallet/migrate/** in their
original browser profile:

1. Close other old-wallet tabs and read the saved wallet. Check its address.
2. Choose a password of at least 12 characters and explicitly create an encrypted
   backup. Encryption takes place locally with `BrowserVault.fromLegacy`.
3. Download the `.nfv` file, open the new wallet, and import the encrypted backup.
   Verify the same address and complete a scan before sending.

The exporter reopens the newly generated ciphertext in a separate vault,
authenticates it with the chosen password and compares the resulting address.
It checks the saved wallet and outbox for concurrent changes before and after
encryption, invalidates a prepared download on storage changes, and checks them
again when the download is clicked. It does not write or delete legacy storage,
upload a seed or wallet, submit a transaction, or claim that a requested download
proves a file was saved to disk.

The legacy keys are `nf-web-wallet-v1` (wallet state) and `nf-web-outbox` (separate
outgoing transactions). Every outbox entry must match a saved outgoing history
entry and its complete transaction. Unresolved history entries must retain input
reservations. A mismatch stops migration; it does not clear, replay or discard
the payment. This matters because 0.9.5 saved its wallet state after submission,
leaving a possible interruption between the outbox and wallet writes. Preserve
the original browser profile and investigate mismatches before another payment.

The original wallet JSON is passed to Rust unchanged. JavaScript only inspects a
copy, never serializes the wallet again, so a legacy `u64` is not rounded during
migration. Outgoing transactions containing integers outside JavaScript's safe
range are refused by the comparison guard. The receiving wallet imports backups
with outgoing-payment quarantine; importing must not authorize a fresh broadcast.

Wallet keys, scan state and recorded payments are in the `.nfv`. The separately
stored legacy address book (`nf-web-book`) is **not** in that format; its labels
remain in the old profile. The original 0.9.5 application and its WASM package are
retained under `/wallet/`. This exporter is a separate application underneath
`/wallet/migrate/`, with its own current WASM copy under `migrate/pkg/`.

Run `node scripts/check-wallet-migration.mjs` for synthetic model checks. These
cover pending-payment mismatches, unsafe numbers, concurrent changes, encrypted
reopen/address verification failures and read-only behavior. They use no real
wallets and do not replace browser/WASM validation or a live deployment check.
