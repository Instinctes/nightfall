# NIGHTFALL Dev 0.9.4-dev.1

Development candidate for macOS Apple Silicon. Devnet/Bitcoin regtest only for
local acceptance testing. **Not a mainnet release or a security certification.**
Production downloads and the production Core application are unchanged.

## Core interface

- Webwallet-aligned dark surfaces, restrained gradients and readable secondary text.
- Responsive balance/network/mining metrics; consistent two-column spacing.
- Clear introductions and independent scroll positions for all eight pages.
- Labeled navigation and primary buttons, visible keyboard focus, genuinely disabled buttons.
- Payment review displays the full recipient address and disables the underlying form.
  Escape closes the review without sending.
- Activity filtering has an actionable empty state and a clear-filter button.
- Receiving and network connection states use more accurate wording.
- Leaving Settings hides revealed keys. Configuration presence is not presented as
  a successful Bitcoin connection check.
- Active swaps are listed before offer creation, with per-trade errors, pending
  transactions, confirmation depths and an explicit NIGHT recovery phase.

## Swap execution and recovery

- Status records use the actual handshake ID and agreed amounts.
- The GUI uses a serial background worker for chain checks and broadcasts.
- Save exact transaction bytes and pending intentions durably before broadcasting.
- Retry randomized NIGHT claims with identical bytes across restarts.
- Observe agreed Bitcoin transaction IDs even after transactions leave the mempool.
- Recover the peer's scalar from the correct signature in the actual redeem/refund witness.
- Require real confirmation depth, not mere mempool presence, for settlement.
- Remember potential redeem-secret exposure before handing bytes to the network;
  never replace an exposed redeem with a locally initiated cancel after a restart.
- Refresh Bitcoin depth before processing a NIGHT lock found after a restart.
- Check the signed Bitcoin exit transactions, lock depth, remaining window and
  unspent output before locking NIGHT. Withhold unconfirmed NIGHT-lock retries
  once that safe window is gone.
- Recover Alice's NIGHT after Bob's confirmed Bitcoin refund; show it as unfinished
  until the NIGHT recovery confirms.
- Block rescanning/pruning/resyncing that would compromise active swaps.
- Bound imported packet size and reject overflowing timelock arithmetic.

## Validation and limits

The Core-worker integration test starts isolated Bitcoin Core regtest and NIGHT
devnet nodes, uses actual on-chain transactions, reloads sessions between steps,
and exercises both successful settlement and cancel/refund/NIGHT recovery.
The layout test renders all eight pages at 620, 884 and 1180 pixels of content
width. Separate regressions cover disabled controls, column bounds and a 4.5:1
contrast floor for normal text tokens on the base surfaces.

Native macOS capture initially failed (ScreenCaptureKit -3811), then recovered.
All eight installed Dev pages were visually reviewed in the empty/disconnected
devnet state. Workspace tests passed: 427 passed, 10 ignored; the isolated
Core-worker integration and 5,000-tick soak were run separately and passed.
This does not cover every populated history, operating system or scaling mode.

**Known protocol limit:** there is no independent timed NIGHT refund. Alice's
recovery requires Bob to publish his Bitcoin refund. If he does not, NIGHT may
remain locked permanently even after punishment. Low devnet depths deliberately
do not offer mainnet reorg protection. Independent cryptographic review and
long-duration public-network acceptance are still required. Keep the mainnet gate closed.

Back up the per-swap `.secret` files as well as the wallet seed. The wallet seed
alone cannot reconstruct swap secrets. Keep the wallet and both nodes online
until settlement or recovery finishes.

## Build

`bash scripts/build-macos-dev-app.sh` produces a fresh app bundle, versioned ZIP
and SHA256SUMS in a unique `target/macos-dev-*` directory. It does not stop,
delete or replace any installed application. The wrapper always starts devnet.
The bundle is ad-hoc signed for local integrity checks, not Apple-notarized.
