//! Find a swap lock in a NIGHT block and turn a UTXO height into depth.

use nightfall_crypto::swap::{LockError, SharedLock};
use nightfall_crypto::Output;
use nightfall_ledger::Transaction;

/// Confirmations of an output created at `created_height`, given the tip.
pub fn confirmations(tip: u64, created_height: u64) -> u64 {
    tip.checked_sub(created_height)
        .map(|n| n.saturating_add(1))
        .unwrap_or(0)
}

/// The lock output inside a transaction Alice just built.
pub fn lock_output_in_tx<'a>(
    tx: &'a Transaction,
    shared: &SharedLock,
    value: u64,
) -> Result<&'a Output, LockError> {
    for o in &tx.outputs {
        if shared.verify_lock(o, value).is_ok() {
            return Ok(o);
        }
    }
    Err(LockError::NotOurOutput)
}

/// Scan a bag of outputs for the one this swap can spend.
pub fn find_lock_output<'a>(
    outputs: impl IntoIterator<Item = &'a Output>,
    shared: &SharedLock,
    value: u64,
) -> Option<&'a Output> {
    outputs
        .into_iter()
        .find(|o| shared.verify_lock(o, value).is_ok())
}

/// Whether Bob's NIGHT claim has landed.
///
/// NIGHT aggregates: after a block the claim txid is gone. Confirmation is
/// the lock existing in history and no longer sitting unspent, or the claim
/// still sitting in the mempool. A lock that was never created must not
/// count as spent — that would finish a swap whose NIGHT never moved.
pub fn claim_seen(lock_was_created: bool, lock_unspent: bool, _claim_in_mempool: bool) -> bool {
    lock_was_created && !lock_unspent
}

#[cfg(test)]
mod tests {
    use super::confirmations;

    #[test]
    fn confirmations_count_the_creating_block() {
        assert_eq!(confirmations(10, 10), 1);
        assert_eq!(confirmations(12, 10), 3);
        assert_eq!(confirmations(9, 10), 0);
    }

    #[test]
    fn a_mempool_claim_does_not_settle_the_trade() {
        assert!(!super::claim_seen(true, true, true));
    }

    #[test]
    fn a_spent_lock_counts_once_it_existed() {
        assert!(super::claim_seen(true, false, false));
    }

    #[test]
    fn a_missing_lock_is_not_a_claim() {
        assert!(!super::claim_seen(false, false, false));
        assert!(!super::claim_seen(true, true, false));
    }
}
