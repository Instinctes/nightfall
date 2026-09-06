# NIGHTFALLCOIN 0.9.5 — mainnet wallets

The regular mainnet Core release for macOS Apple Silicon, Intel macOS,
Windows x64 and Linux x64, plus the responsive mobile/desktop web wallet.
This replaces 0.9.2 as the current wallet download. Same chain, protocol v8,
wire v6 and `n8` data format. No reset, migration or seed-node upgrade.

## A shared wallet interface

Soft violet surfaces, rounded cards, subtle shadows and pastel pink/cyan
accents now connect Core and the web wallet. Dark lettering on bright balance
cards and action buttons preserves readability. Desktop navigation uses an
icon sidebar; the responsive web wallet keeps bottom navigation on phones.

All eight Core pages were reviewed: Dashboard, Send, Receive, Activity,
Mining, Network, Swap and Settings. Improvements include responsive metrics,
independent scroll positions, clear connection states, keyboard focus and
genuinely disabled actions. Receive uses a square QR with a four-module quiet
zone. Payment review shows the full address and blocks the underlying form;
leaving Settings hides revealed keys.

The browser wallet carries the design through balances, activity, payments,
receive addresses, contacts and settings. Invalid/incomplete or unaffordable
payments cannot open review; the confirmation shows the complete address.
Node failures use readable connection errors. HTML escaping covers attribute
quotes. No third-party scripts, fonts, analytics or account services were added.

## Mainnet use and the swap boundary

Core starts on **mainnet** by default. Mainnet mining, sending and receiving
continue as before. Existing wallet data is retained. Back up your recovery
phrase and quit Core fully before replacing the application.

**Atomic swaps are still disabled on mainnet.** The experimental devnet/testnet
implementation includes background monitoring, durable exact-transaction retries,
confirmed-chain observation, corrected signature-secret extraction and NIGHT
recovery after Bob's Bitcoin refund. These do not constitute a mainnet safety
approval. There is no independent timed NIGHT refund: if Bob never refunds,
Alice's NIGHT may remain locked permanently, even after punishment.
Use test coins only, keep both nodes and Core online, and back up per-swap
`.secret` files as well as the seed. Independent cryptographic review and
long-duration public-network acceptance are outstanding.

The web wallet remains a mainnet light wallet with its existing node-trust and
browser-storage limitations. It receives the design update, **not atomic swaps**.

## Download and verify

- `NIGHTFALLCOIN-Core-0.9.5-macOS-arm64.dmg` — macOS 11+
- `NIGHTFALLCOIN-Core-0.9.5-macOS-intel.dmg` — macOS 10.15+
- `nightfall-core-0.9.5-windows-x64.exe`
- `nightfall-core-0.9.5-linux-x64` — GUI dependencies required
- `nightfalld` and `nightfall-wallet` CLI binaries for Windows/Linux;
  macOS includes them inside the app bundle.

Verify against `SHA256SUMS-0.9.5.txt`, `SHA256SUMS-0.9.5-windows.txt` or
`SHA256SUMS-0.9.5-linux.txt` before running downloads. macOS apps are ad-hoc
signed, not Developer-ID signed or notarized; Windows builds have no publisher
signing certificate. A checksum verifies file integrity, not software safety.

## Validation scope

Local gates include strict workspace Clippy, workspace tests, Core eight-page
layout tests at three content widths, normal-text and gradient contrast tests
and disabled-control regressions. Windows/Linux release jobs run workspace
tests before building. macOS app integrity and DMG checks precede upload.

The shared Core UI was visually checked in the installed test candidate on macOS.
Web wallet navigation/layout was checked at 320, 390, 768, 860, 1024 and 1440 px
using a public test phrase on localhost, with no real funds. These are bounded
checks, not every OS, screen reader, populated history or physical mobile device.
The isolated real-bitcoind/NIGHT Core-worker integration covers settlement and
cancel/refund/NIGHT recovery with session reloads between ticks. A separate
5,000-tick simulation passed on the preceding candidate. No claim of 100% safety.

0.9.4 remained an unpublished draft. This release adds explicit floating-point
types and removes unnecessary test clones to pass the newer CI compiler without
changing transaction rules or enabling mainnet swaps.
