# Atomic swap — withdrawn from NIGHTFALLCOIN, 18 September 2026

NIGHT ↔ BTC atomic swap is **withdrawn**. It is not in wallet 1.0.0, it is not
scheduled for a later 1.x, and the code is out of the tree. This page is the
record of why, what was removed, and what was deliberately left behind.

The decision is the operator's. It was taken before the 1.0.0 release rather
than after, which is the only point at which it was still free.

## The short version

The swap worked. That was never the question. The question was whether a
two-person project should ship a cross-chain protocol whose worst case is a
user losing coins permanently, with no independent audit, when the protocol
itself cannot be made safe by any amount of implementation work.

The answer is no.

## What could not be fixed

These are properties of the construction, not defects in our code. No further
engineering removes them.

**1. There is no timed NIGHT refund, and there cannot be one.**

Mimblewimble has no script language. A Bitcoin lock can carry `OP_CSV` and
refund itself after a deadline; a NIGHT lock cannot. Alice can only reclaim her
NIGHT *after* Bob publishes his Bitcoin refund, because that publication is
what reveals the share she needs. That is conditional recovery, not a timeout
she can enforce. If Bob cancels and then never refunds, **the NIGHT stays
locked forever.** Nobody can move it — not Alice, not Bob, not us.

**2. §9.2: a late redeem can lose Alice both sides.**

If Alice broadcasts her Bitcoin redeem too close to `H₁` and it does not
confirm, her share `s_a` is already public — in a mempool, in a log, in a
reorged block — while Bob's cancel does confirm. Bob refunds the Bitcoin *and*
claims the NIGHT. He ends up with both. The state machine refuses to redeem
inside the safety margin, and a Bitcoin outage at the wrong moment is exactly
the case the margin cannot cover. See `SWAP-LOSS.md`, the row in bold.

**3. The fee ladder is a protocol change, not a patch.**

Bitcoin fee pressure can leave a correctly signed abort transaction unrelayed.
Fixing that means negotiating a versioned transaction graph, signing *every*
redeem variant under the adaptor before either side funds anything, binding
refund and punish signatures to each cancel's exact txid and residual value,
bounding the cumulative fee budget, and persisting every publishable variant
before any bytes reach a node. `SWAP-INTERNAL-REVIEW-2026-09-17.md` §"Fee-ladder
work remains a protocol change" lists the seven constraints in full. At the end
of all of it, a finite ladder still cannot guarantee inclusion, and §9.2 is
still there.

## What the last review found

The internal review of 17 September 2026 examined an implementation that
already looked finished. It found four confirmed defects in it:

| ID | What it was |
|---|---|
| C-SWAP-02 | A pending cancel survived an observed redeem and could be rebroadcast, prolonging the cancel/redeem race from the honest side |
| C-SWAP-03 | A confirmed NIGHT claim could mark a swap finished while the Bitcoin redeem was still in a mempool |
| C-SWAP-04 | A fully signed refund or punish output could land one satoshi below the dust limit and never relay |
| C-SWAP-05 | A funding amount could panic the transaction builder outright |

All four were corrected and pinned with regressions. That is not the point. The
point is the **rate**: four real defects, two of them loss-bearing, found in a
single pass over code that had already been reviewed. The next pass would have
found more. That is what an unaudited cross-chain protocol looks like, and it
is not a surface a small team should be carrying into its first mainnet
release.

## What was never true

The wallet has always refused to run a swap on mainnet. The gate was closed in
every published build. **No user has ever executed a NIGHT ↔ BTC swap with real
coins through this software, and no user funds are at risk from this removal.**
Testnet and devnet swaps were experimental and are not recoverable through
1.0.0; they were never real money.

## What it would have cost to keep

- **~15,000 lines of Rust** across one dedicated crate and sixteen files in
  the wallet, ledger and crypto crates — the largest single subsystem in the
  project, and the only one whose failure mode is permanent loss.
- **21 third-party packages**, including `secp256k1` and `secp256k1-sys` (C),
  the whole `bitcoin` crate family, `ecdsa_fun` and `sigma_fun` — and
  `curve25519-dalek-ng` with `subtle-ng`: a **second, divergent implementation
  of the same curve arithmetic** the chain itself already depends on, linked
  into the same binary. Removing swap removes all of it.
- **A running bitcoind with `-txindex=1`** as a hard requirement on every user
  who wanted to use the feature, plus three externally supplied Bitcoin
  addresses, manual packet copy-paste, and Core kept running for the whole
  window. Realistic reach: almost nobody.
- **An independent audit** we had not commissioned and could not responsibly
  ship without.

## What was removed

Deleted outright: the `nightfall-swap` crate; `app_swap.rs`,
`app_swap_drive.rs`, `app_swap_lock.rs`, `app_swap_night.rs`,
`app_swap_send.rs`, `swap_worker.rs`, `swap_live_tests.rs`, `swap_ui_tests.rs`,
`views_swap.rs`, `widgets_swap.rs`, `wallet_state/swap_checkpoint.rs` in the
Core wallet; `ledger/src/swap.rs` with its two test files; `crypto/src/swap.rs`
and `crypto/src/dleq.rs`.

Also gone: the Swap page and its navigation entry, the Settings card for
bitcoind credentials, the swap worker and its per-frame tick, the vault
lock/prune/resync guards that existed only to protect an unfinished swap, and
every user-facing string that mentioned swaps.

## What was deliberately kept

**`nightfall-wallet/src/swap_journal.rs` and the `swap_journal` field stay.**
This is not an oversight and it must not be "cleaned up" later.

The wallet database carries `#[serde(deny_unknown_fields)]`. The journal field
is written only when it is non-empty (`skip_serializing_if`). Remove the field
and any wallet file that contains journal records becomes **unopenable** —
the wallet would refuse to load rather than ignore the unknown key. The cost of
keeping it is one inert struct. The cost of removing it is someone's wallet.

The guards that read it stay for the same reason, and because they are still
correct:

- Restoring a backup that contains journal records is refused.
- Rescanning while journal records exist is refused, because a rescan clears
  the reservations those records depend on.
- A pending payment whose memo is `swap-lock` is **never** rebroadcast
  automatically. Its deadline passed long ago, the counterparty may already
  have refunded, and nothing watches the other chain any more. Only a person
  who understands what that payment was may republish it.
- The browser wallet refuses a vault carrying swap material, because a browser
  can neither monitor deadlines nor reconcile a second chain.

Every one of these is a no-op on a wallet that never ran an experimental swap,
which is every wallet in existence on mainnet.

`retire_swap_checkpoint` in the journal remains available for a future,
deliberate, operator-driven cleanup of a developer wallet. Nothing calls it.

## Consensus

**No consensus rule, protocol version, genesis parameter or emission schedule
changed.** This was verified before a line was deleted: `nightfall-consensus`
contains zero references to swap, and block aggregation never reaches
`ledger/src/swap.rs`. Removing swap is a wallet change. **There is no fork, no
migration, and no action required from any node operator or miner.**

## Related documents

`SWAP.md`, `SWAP-LOSS.md`, `SWAP-ATTACKS.md`, `SWAP-SPEC-DRAFT.md`,
`SWAP-SPEC-v0.1-withdrawn.md`, `SWAP-VAULT-PROGRESS-2026-09-13.md`,
`INTERNAL-SWAP-REVIEW-2026-09-13.md` and
`SWAP-INTERNAL-REVIEW-2026-09-17.md` are kept as the historical record. They
describe software that is no longer in the product. Each now carries a
withdrawal banner. Do not read them as documentation of a shipping feature.

## If this is revisited

It should not be revisited as an atomic swap. The two constructions worth
examining later, in this order:

1. **A liquidity venue that never holds NIGHT custody on both sides at once** —
   the loss cases above all come from two locks existing simultaneously with
   only one of them able to time out.
2. **A listing on an exchange that already runs the counterparty risk as its
   business.** This is what every comparable privacy chain actually did, and it
   costs the project nothing but a listing process.

Neither is a 1.0.0 problem.
