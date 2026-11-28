//! Non-consensus fingerprint of the complete materialised ledger state.
//!
//! IMPORTANT:
//! This is NOT a block commitment and MUST NOT participate in V8 consensus.
//! Its purpose is storage/snapshot integrity. In particular, V8's historical
//! `utxo_root` does not commit every piece of local UTXO metadata.

use crate::LedgerState;
use nightfall_crypto::hash_multi;
use nightfall_types::Hash256;

const STATE_FINGERPRINT_DOMAIN: &[u8] = b"nightfall:local-chainstate-fingerprint:v1";

pub fn state_fingerprint(state: &LedgerState) -> Hash256 {
    let mut leaves: Vec<Hash256> = Vec::with_capacity(state.utxos.entries.len());

    for (commit, entry) in &state.utxos.entries {
        let coinbase = [u8::from(entry.is_coinbase)];

        leaves.push(hash_multi(
            b"nightfall:local-chainstate-utxo:v1",
            &[
                commit,
                &entry.output_pk,
                &entry.height.to_le_bytes(),
                &coinbase,
            ],
        ));
    }

    let refs: Vec<&[u8]> = leaves.iter().map(|h| h.0.as_slice()).collect();

    let utxo_metadata_root = hash_multi(b"nightfall:local-chainstate-utxos:v1", &refs);

    hash_multi(
        STATE_FINGERPRINT_DOMAIN,
        &[
            &state.height.0.to_le_bytes(),
            &state.utxo_root().0,
            &utxo_metadata_root.0,
            &state.kernel_sum().0,
            &state.kernels.count.to_le_bytes(),
            &state.supply.total_minted_darks.to_le_bytes(),
            &state.supply.total_burned_darks.to_le_bytes(),
            &state.tx_count.to_le_bytes(),
            &state.coinbase_maturity.to_le_bytes(),
        ],
    )
}
