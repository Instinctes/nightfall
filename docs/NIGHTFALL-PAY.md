# Nightfall Pay

Status: shipped in 1.1.0. No protocol change.

## Interoperability

The existing shared Rust `PaymentRequest` is authoritative. Both Core and
BrowserVault use it. Required `network` (mainnet/testnet/devnet); optional
`amount` in whole darks, `memo`, `id`, `expires` (Unix seconds). Duplicate and
unknown fields are refused. Network identity is checked separately before
payment. Expiry is a displayed payee policy, not a consensus rule. Parsing
never signs, reserves or broadcasts a transaction.

The established transport is `nightfall:<address>?network=...&amount=...`.
The new browser-opening transport is exactly:

`https://wallet.nightfallcoin.org/#pay=<percent-encoded entire nightfall URI>`

Only that exact HTTPS origin/path is accepted by the shared parser. Percent
encoding is canonical uppercase hex and encodes everything except ASCII
alphanumerics and `-._~`. Nested wrappers, malformed encoding and additional
fragment fields are refused. Inner payload maximum: 2048 UTF-8 bytes. Envelope
maximum: prefix plus 6144 bytes. An address is not network-tagged; never infer
network from its spelling. Monetary input is never parsed through floating point.

## UI behavior

Core invoice rows offer Copy payment link; pasting the link into Send uses the
same request review card as the original URI. Browser Receive offers Copy
payment link; QR retains the compact scheme URI. Opening a browser share link
removes the fragment from the current history entry, keeps the request only in
memory while locked, then shows Send with validated details after unlock. No
payment occurs until normal explicit review and confirmation after a valid scan.
A reload before unlocking discards the in-memory request; reopen the link.
Automatic OS-level scheme registration is not implemented or claimed.

Fragment payloads are not transmitted in HTTP paths or queries. This is not
end-to-end encryption: recipients, clipboard tools, browser extensions and
scripts loaded by the wallet can see the request. Do not put secrets into a
memo. Never move the payload to server query parameters or analytics.

## Validation

- `cargo +1.98.0 test --offline --locked -p nightfall-wallet payment_request --lib`
- `node scripts/check-pay-link.mjs`
- Core layout regression `the_payment_request_card_fits_supported_content_widths`.
- Web UI uses the existing WASM scheme parser after decoding the envelope;
  rebuilding WASM adds direct Rust envelope acceptance too. No live bundle
  replacement is required to test browser fragment transport locally.
