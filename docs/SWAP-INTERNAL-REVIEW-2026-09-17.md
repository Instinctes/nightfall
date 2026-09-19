# Internal swap protocol review — 17 September 2026

> **WITHDRAWN — 18 September 2026.** Atomic swap is not part of NIGHTFALLCOIN.
> It is not in wallet 1.0.0 and is not scheduled for a later release. The code
> has been removed from the tree. This document is kept as a historical record
> only; it describes software that no longer exists in the product. See
> [`SWAP-WITHDRAWN.md`](SWAP-WITHDRAWN.md) for the decision and its reasoning.

This is an internal source and regression-test review of the working tree,
not an independent audit, a security certification or a Mainnet release.
It follows `brain.md` BE–BG and the current `WALLET-1.0.0-PLAN.md`.
No live wallet, deployment, release channel or consensus rule was changed.

## Confirmed findings and corrections

All four regressions below were first run against the uncorrected swap
implementation and failed. The original test binary was retained temporarily
at `/private/tmp/nightfall-swap-review-before-fixes-20260917`; this is local
reproduction evidence, not a release artifact.

### C-SWAP-02 — Pending cancel bypassed observed redeem exposure

Location: `crates/nightfall-swap/src/driver.rs`, `Session::tick_inner`, peer
observation and pending-send retry branches.

A session already holding a pending cancel could observe the counterparty's
redeem in the Bitcoin mempool. The peer-observation branch correctly changed
the state to `Redeeming`, but retained the pending cancel. The pending-send
branch then returned its exact bytes for another broadcast without consulting
the state machine. This violated the driver's own rule to stop cancelling
after the redeem secret becomes public and could prolong the cancel/redeem
race from the honest client's side.

Correction: observed redeem exposure clears an older cancel intention. A
pending-only change is also persisted, even when the state already was
`Redeeming`. Existing confirmed cancel/refund/punish evidence still takes
precedence over an unconfirmed competing redeem. Removing a local intention
cannot retract a cancel already distributed to peers.

Regression:
`driver::tests::an_observed_redeem_withdraws_an_older_cancel_intention`
tests legacy and detached execution and checks the legacy durable record.
The original code failed with
`a newly observed public secret must cancel the older cancel intention`.

### C-SWAP-03 — Pending NIGHT claim bypassed joint settlement depth

Location: `driver.rs`, confirmation handling for `StoredSwap::pending`.

Bob can learn Alice's share from a mempool redeem and confirm his NIGHT claim
before Bitcoin has confirmed that redeem. The general peer-observation branch
required both transactions to meet their agreed depths before `Done`; the
pending NIGHT-claim branch instead applied `BobClaimedNight` immediately.
It could mark the swap finished and permit retirement/locking while the
Bitcoin side was still unsettled.

Correction: an already confirmed pending NIGHT claim remains tracked while
the agreed redeem lacks its required Bitcoin depth. This does not block
Alice's separate NIGHT recovery after a confirmed Bitcoin refund.

Regression:
`driver::tests::a_confirmed_night_claim_waits_for_bitcoin_redeem_finality`.
The original code failed with
`NIGHT finality does not finalize a mempool redeem`.
The existing successful-claim test now supplies actual redeem depth instead
of silently assuming settlement from an in-memory `Redeeming` state.

### C-SWAP-04 — Positive abort outputs could still be dust

Location: `crates/nightfall-swap/src/bitcoin_tx.rs`, `spend_lock` and
`spend_cancel`.

The builders accepted any output above zero. A valid non-dust funding and
cancel could therefore lead to a completely signed refund/punish output one
satoshi below the destination script's default dust limit. Such a refund is
not relayed under the default policy. The abort safety path must fail before
the parties fund this graph, rather than first discovering that its signed
exit is dust after the deadline.

Correction: lock outputs and every redeem/cancel/refund/punish output are
checked against `Script::minimal_non_dust()`. The exit helper checks the actual
remaining amount after each fee, including both fees on an abort path. The
exact non-dust boundary is accepted. This check does not guarantee relay under
a node's custom policy or confirmation during fee pressure; preflight and a
real fee replacement protocol are still required.

Regression: `bitcoin_tx::tests::abort_children_must_leave_a_relayable_output`.
The original refund builder failed the assertion
`the refund would leave one satoshi below the script's dust threshold`.

### C-SWAP-05 — Funding amount addition could panic

Location: `bitcoin_tx.rs`, `TxLock::from_prevout`.

The funding builder used Bitcoin `Amount`'s panicking `+` implementation on
caller-supplied amounts. `value = u64::MAX`, `fee = 1` panicked with
`Amount addition error`. The UI's saturating funding sum did not make the
builder safe and public session/builder APIs could reach it directly.

Correction: reject values outside Bitcoin's monetary range, calculate the
required funding with `checked_add`, and use checked fee subtraction in the
shared exit-value validator. Invalid amounts produce an error.

Regression:
`bitcoin_tx::tests::impossible_funding_values_are_errors_instead_of_panics`.

## Detached execution and cryptography

The detached driver already restored its complete candidate after a failed
tick. An additional regression now observes a public redeem, clears a pending
cancel, and then fails the later lock lookup. It compares the complete record
with its original value, so a host cannot accidentally commit the partial
observation from a failed tick:
`driver::tests::a_detached_partial_lookup_failure_rolls_back_every_observation`.

The DLEQ v2 source was reviewed again: each curve has an independently derived
hash-to-group generator, bit-OR statements bind the same bit on both curves,
and the final equal-discrete-log proofs bind those generators to the public
claims. The 252-bit bound stays below both group orders. The previously
confirmed unequal-secret attack and old-format rejection remain explicit
regressions. This pass found no new confirmed cryptographic flaw and changes
no cryptographic construction. That is not a proof of overall protocol safety.

## Fee-ladder work remains a protocol change

`fees.rs` still validates proposed absolute fees only. `session.rs::fee`
returns the single negotiated `btc_fee_sats`, while the session keeps one
redeem adaptor. `driver.rs` indexes raw and watched transactions by `SendKind`
and `persist.rs::StoredSwap::outgoing` stores one byte string per kind.
There is no signed replacement graph or multi-variant exposure history.

An implementation must address these constraints together:

1. Negotiate a versioned graph committing to network, keys, destination
   scripts, values, CSV delays, all fee choices and cumulative budgets. Do
   not relabel an existing v2 session as a newer protocol.
2. Sign every redeem variant with Bob's adaptor under Alice's encryption
   point before funding. A different fee changes the sighash and requires
   its own signature; the fee selector cannot alter a signed transaction.
3. For each cancel parent, bind refund and punish signatures to that exact
   parent's txid and remaining value. Changing a cancel fee changes both
   the txid and the amount of its child input. Bob needs Alice's refund
   adaptor and Alice needs Bob's punish signature for each admitted branch.
4. Bound `cancel_fee + child_fee` as well as each successful-path fee. Check
   every resulting output for dust. Bound the graph's signature count,
   serialized size and encrypted journal reservation before releasing funds.
5. Specify replacement sequences and validate current node replacement and
   package policy, including incremental relay fees, descendant conflicts and
   fee ceilings. The current redeem uses `Sequence::MAX`; changing only a
   selection helper does not implement replacement behavior.
6. Persist every potentially published redeem variant and its adaptor before
   giving bytes to a node, including preflight calls. Exposure is monotonic
   across timeouts, process loss and mempool eviction. Watch all possible
   competing txids and recover from the adaptor belonging to the observed
   variant. Confirmed chain evidence must select the surviving graph branch.
7. Exercise replacement, pinning, parent replacement, cancellation races,
   restarts and insufficient fee ceilings with the actual Bitcoin interpreter
   and then with public-network timing. A finite ladder cannot guarantee
   inclusion or remove the underlying late-redeem loss scenario.

These corrections do not finish the dedicated swap-backup recovery workflow,
pre-v2 recovery, terminal journal retirement, public-network duration tests,
or the signed fee graph. The permanent NIGHT lock following cancel without
refund is still a protocol limitation, not repaired by Vault persistence.
The implementation therefore does not support an unqualified claim that
Atomic Swaps are completely finished. Mainnet stays gated.

## Verification

The post-fix swap suite is running with Rust 1.98.0, `--offline --locked`,
`--lib --tests`, without doctests. Final counts will be appended after it
finishes. Log: `/private/tmp/nightfall-swap-review-20260917-tests.log`.
Ignored live/regtest and long-duration tests are not passing evidence.
