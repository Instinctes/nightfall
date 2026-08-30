//! Claiming a swap lock — the one supported way to spend a shared output.
//!
//! **Experimental. Not wired into any wallet.** See `docs/SWAP-SPEC-DRAFT.md`
//! v0.2.
//!
//! # Where the safety actually lives
//!
//! Spending the shared output needs the spend secret `s_a + s_b + offset` for
//! the input signature and the blinding factor `b_in` for the kernel excess
//! `b_out − b_in`. Both derive from the shared secret `t`.
//!
//! An earlier draft of this module claimed the safety property was *taking
//! `b_in` from `t` rather than from the sealed payload*. A mutation test
//! disproved it: swapping one source for the other passes every test, and it
//! passes because it cannot fail. Once [`SharedLock::verify_lock`] has run,
//! the commitment opens as `expected_value·G + derive_blind(t)·H` and the
//! payload opens as `payload_value·G + payload_blind·H` with
//! `payload_value == expected_value` — and a Pedersen commitment is binding,
//! so the two blinds are the same scalar. There is no input that tells them
//! apart.
//!
//! **The load-bearing step is therefore the verification, not the choice of
//! source.** `claim_spendable` runs it first and treats failure as fatal.
//! Code that skips it and reaches for the payload is where the danger was all
//! along: the paying party chooses that payload, and a claim built from a
//! lying one produces `UnbalancedBlock` — after the counterparty's Bitcoin
//! secret is already public.
//!
//! `lock_blind` is still what this module uses, because deriving from `t` is
//! the shorter path and does not depend on the payload opening at all. That is
//! a robustness preference, not the security argument.

use crate::builder::{build_transfer, BuildError, Payment, Spendable};
use crate::tx::Transaction;
use curve25519_dalek::scalar::Scalar;
use nightfall_crypto::swap::{LockError, SharedLock};
use nightfall_crypto::{Address, Output, WalletKeys};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ClaimError {
    /// The lock did not pass [`SharedLock::verify_lock`]. **Do not claim, and
    /// do not release a swap secret.** Spec v0.2 §8 phase 2a.
    #[error("lock rejected: {0:?}")]
    Lock(LockError),
    /// The ephemeral key is malformed, so no shared secret can be derived.
    #[error("malformed ephemeral key")]
    BadEphemeralKey,
    #[error("build failed: {0}")]
    Build(#[from] BuildError),
}

/// Turn a verified lock into something [`build_transfer`] can spend.
///
/// Verification runs first and failure is fatal by design: a lock that does not
/// verify is one whose payload lies, whose amount is wrong, or which is not
/// ours at all. Recovering coins from such an output is possible but is a
/// deliberate manual act, not something this function should quietly do.
pub fn claim_spendable(
    shared: &SharedLock,
    lock: &Output,
    value: u64,
    a: &Scalar,
    b: &Scalar,
) -> Result<Spendable, ClaimError> {
    shared.verify_lock(lock, value).map_err(ClaimError::Lock)?;

    let blind = shared
        .lock_blind(&lock.ephemeral_pk)
        .ok_or(ClaimError::BadEphemeralKey)?;
    let spend_secret = shared
        .claim_secret(a, b, &lock.ephemeral_pk)
        .ok_or(ClaimError::BadEphemeralKey)?;

    Ok(Spendable {
        commit: lock.commit,
        value,
        blind,
        spend_secret,
    })
}

/// Build the transaction that sweeps a verified lock to `to`.
///
/// `fee_darks` is burned, as every fee on this chain is, so the recipient gets
/// `value − fee_darks`.
#[allow(clippy::too_many_arguments)]
pub fn build_claim(
    shared: &SharedLock,
    lock: &Output,
    value: u64,
    a: &Scalar,
    b: &Scalar,
    to: &Address,
    fee_darks: u64,
    ctx: &[u8],
) -> Result<Transaction, ClaimError> {
    let spendable = claim_spendable(shared, lock, value, a, b)?;
    let amount = value
        .checked_sub(fee_darks)
        .ok_or(BuildError::AmountOverflow)?;

    // `owner` is unused by `build_transfer` — every input carries its own spend
    // secret — but the signature demands one. A throwaway keeps the shared
    // output's secrets out of any wallet identity.
    let nobody = WalletKeys::generate();

    Ok(build_transfer(
        &nobody,
        &[spendable],
        &[Payment {
            to: *to,
            amount,
            memo: String::new(),
        }],
        fee_darks,
        to,
        0,
        ctx,
    )?)
}
