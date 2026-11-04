# Checkout — devnet prototype

Status in 1.1.0: **Devnet-only, view-only** Checkout CLI, authenticated API and merchant UI. No mainnet Checkout. The sections below first describe the original 30 September wallet-based implementation; the current view-only architecture and invocation are documented at the end.

A shop has one question after it hands a customer a payment request: *may I
hand over the goods?* The wallet already had a till (`counter.rs`) that
answers "did the right payment arrive?" by matching the invoice reference in
the memo, never by amount. What it did not answer was the second half: a
payment one block deep has *arrived*, but a reorganisation can still remove
it. A till that says "Paid" at that moment invites the merchant to ship.

## What was added

`counter::settlement(status, tip_height, need)` → one of

| Settlement | Meaning | Hand over? |
|---|---|---|
| `NotPaid` | open, expired, underpaid, or only a *candidate* (right amount, no reference) | no |
| `Unconfirmed` | settled only by a payment not yet in a block | no |
| `Confirming { have, need }` | in blocks; the shallowest settling payment has `have` of `need` confirmations | not yet |
| `Confirmed` | every settling payment is at least `need` deep | threshold met; reorg risk remains |
| `ClosedByHand` | the merchant closed it ("paid in cash"); nothing on chain is claimed | merchant's call |

Rules, each with a unit test:

- confirmations = tip − height + 1; `need` is at least 1;
- a split payment is only as final as its **newest** part — losing that part
  makes it an underpayment, which is `NotPaid`;
- a payment reported above the tip (view and tip disagree mid-reorg) is not
  counted as confirmed;
- it is a pure function of what the wallet sees *now*: after a reorg the
  wallet rescans from its birth height, the payment moves, returns to the
  mempool or vanishes, and the answer changes with it. Nothing stale can be
  reported as final.

`DEFAULT_CONFIRMATIONS = 10` is a merchant policy, not a consensus rule. The
protocol permits reorganisations up to `MAX_REORG_DEPTH` (500) blocks; no
smaller number is a guarantee, only a cost to an attacker.

CLI (`nightfall-wallet`, devnet/testnet):

```
nightfall-wallet invoice --reference A-1 --darks 300000000 [--expires-in 900]
    # prints the payment request; the reference travels as memo and id
nightfall-wallet till [--confirmations 10] [--json]
    # syncs from birth height, then: invoice state + settlement per invoice
```

Core's Counter screen shows the same settlement under each settled invoice
("Confirming 2/10 — do not hand over yet", "Confirmed — threshold met; reorg risk remains"), at
the default policy of 10; layout-tested at 620/884/1180 px.

Invoices persist in the wallet database (existing `add_invoice`, unique
references, bounded count).

## Evidence — real processes on an isolated devnet

`node scripts/lab/checkout-devnet.mjs` runs a merchant node, a miner node, a
merchant wallet, a customer wallet and later a rival miner, all on loopback
with temp datadirs. Three consecutive runs on 30 September, darwin/arm64:
**4/4 checks each**.

1. Invoice A-1 paid with the request's memo: till went `not_paid` →
   `confirming:2/3` → `confirmed`, never final early.
2. Invoice B-2 paid and mined (`confirming:2/1000`), then a heavier rival
   branch forked below it replaced the merchant's chain: till reported B-2
   `not_paid` afterwards and had never said final. A-1, below the fork,
   stayed `confirmed`.
3. `till` on mainnet refuses and creates nothing.

## Not built / not claimed

- Local authenticated Devnet API adapter only (below); no public/hosted checkout or automatic broadcast.
- The till still runs inside a wallet that holds the spend key. The counter
  code needs only view-key data, but a separate view-only till process is not
  built.
- No mainnet enablement. No third-party wallet has been tested paying these
  requests.
- A payment request is not proof of payment, and a receipt proves what it
  states (see `verify-receipt`), not that goods were delivered.

The local API now uses `confirmed` rather than `final`. This is an unreleased
prototype change. Confirmation thresholds never guarantee irreversible payment.

## Local authenticated service (1 October, unreleased)

`scripts/checkout/devnet-service.mjs` wraps the existing CLI; it exposes no send,
signing, key export or broadcast command. Only `127.0.0.1` is supported for both
listener and RPC. Browser Origins and foreign Host headers are refused; every
request requires a random 32-byte hexadecimal bearer token. Never put that token
in a URL. This is a local development adapter, not a public merchant endpoint.

Use a NEW disposable Devnet wallet directory, initialize it with the CLI on
Devnet, and add a `.nightfall-checkout-devnet` file containing exactly `devnet`
plus newline as explicit opt-in. Do not mark an existing real wallet directory.
Supply an absolute wallet binary path, absolute data path, and an owner-only
(0600) token file. Generate the token locally with a cryptographic RNG.

```
node scripts/checkout/devnet-service.mjs --wallet-bin /absolute/path/nightfall-wallet \
  --datadir /absolute/path/disposable-devnet-wallet --rpc 127.0.0.1:17883 \
  --token-file /absolute/path/private-token --port 4187 --confirmations 10
```

The RPC port must match your isolated Devnet node. No node is started by this
service. Do not run the example against a production wallet.

- `POST /invoices`, Content-Type `application/json`: fields `reference` (1–48
  ASCII letters/digits/underscore/hyphen, begins alphanumeric), `amount_darks`
  (positive decimal integer STRING), optional `description` (64 UTF-8 bytes),
  optional `expires_in` (1–604800 seconds). Unknown fields are refused.
- `GET /invoices/<reference>`: freshly synced CLI status. Existing wallet DB
  persists invoices; the service does not maintain a second ledger/cache.
- Requests serialize access to the wallet, with a bounded queue. Backend failure
  returns 503, never a cached success or raw wallet diagnostic. Creating an
  invoice may have succeeded before a failure: check its reference before retry.
- `confirmed` means the chosen threshold is met, not irreversible settlement.
  Reorgs can return it to `not_paid`. Late-payment policy remains in invoice
  status; no goods-delivery decision is automated.

`node scripts/check-checkout-service.mjs` exercises the HTTP boundary using a
mock backend. The extended Devnet E2E also checks authenticated HTTP invoice
creation, fresh confirmed status and reorg reversal against real processes:
7/7 passed on 30 September 22:49 UTC, report
`output/lab/checkout-2026-09-30T22-49-22-718Z`. No public deployment,
merchant pilot, view-only wallet process or security audit is claimed.


## 11 October local completion pass

The service CLI now requires `--view-key-file` and the `nightfall-till` binary. This is a separate view-only process: no `Wallet::open`, seed option or spending commands. Owner-only view-key files are read locally; immutable invoice JSON files publish by hard link after fsync. Duplicate references cannot overwrite existing invoices. No observations or cached success are persisted. Current Devnet chain bodies replay through existing full consensus validation, capped at 10,000 blocks; the scanned tip anchor is checked again after replay. Branch changes/failures return unavailable status. Normal chain extension may continue beyond the observed tip; confirmations are computed at the reported observed tip.

The loopback service serves a local merchant UI at `/`: token entry, exact decimal invoice amount, expiry, copy request and fresh status. Bearer tokens stay in memory and clear on tab hiding. Only its own exact loopback origin is accepted; foreign/null origins and DNS aliases remain refused. Core Counter also supports invoice deadlines and a saved confirmation-policy setting. Mainnet remains refused by the till/service; no production pilot is claimed.
