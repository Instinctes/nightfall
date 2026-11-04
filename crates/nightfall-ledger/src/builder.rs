//! Wallet-side transaction construction.

use curve25519_dalek::scalar::Scalar;
use nightfall_crypto::{
    build_kernel, create_output, create_output_with_features, generator_g, sig, Address,
    Commitment, KernelFeature, WalletKeys,
};

use crate::tx::{Input, Transaction, TxError, MAX_INPUTS, MAX_OUTPUTS};

pub const TX_VERSION: u32 = 2;

/// An owned UTXO with everything needed to spend it.
#[derive(Clone, Debug)]
pub struct Spendable {
    pub commit: Commitment,
    pub value: u64,
    pub blind: Scalar,
    /// Secret key of the output's one-time key `Ko`.
    pub spend_secret: Scalar,
}

/// Where a slice of value is going.
#[derive(Clone, Debug)]
pub struct Payment {
    pub to: Address,
    pub amount: u64,
    pub memo: String,
}

/// Build the block subsidy transaction.
///
/// `Σ outputs − reward·G = blind·H`, so the excess secret is simply the
/// output's blinding factor.
pub fn build_coinbase(
    miner: &Address,
    reward_darks: u64,
    height: u64,
    ctx: &[u8],
) -> Result<Transaction, TxError> {
    let (output, secrets) = create_output_with_features(
        miner,
        reward_darks,
        "",
        ctx,
        nightfall_crypto::OutputFeature::Coinbase,
    )
    .map_err(|_| TxError::MalformedCommitment)?;

    let mut tx = Transaction {
        version: TX_VERSION,
        inputs: vec![],
        outputs: vec![output],
        kernels: vec![],
        stealth_offset: None,
        kernel_offset: None,
    };

    // Kernel excess = Σ out_blinds (no inputs). No kernel offset, ever: the
    // binding rules require the coinbase kernel's excess to open the coinbase
    // output exactly, which is what pins the subsidy to the flagged output.
    // `lock_height` carries the block height, which pins a coinbase kernel to
    // exactly one height and stops it being replayed at another.
    let excess_secret = secrets.blind;
    let mut kernel = build_kernel(
        KernelFeature::Coinbase,
        0,
        reward_darks,
        height,
        &excess_secret,
    );
    // Key balance with no inputs: −Ke = X + s·G  ⇒  x = −r − s.
    let s = nightfall_crypto::random_scalar();
    kernel.attach_stealth(&(-secrets.ephemeral - s));
    tx.kernels = vec![kernel];
    tx.stealth_offset = Some(s.to_bytes());
    Ok(tx)
}

/// Options for building a transfer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BuildRules {
    /// Add a random kernel offset. Only valid once the next block is at or
    /// above the binding fork; a wallet that does not know the height leaves
    /// it off, and the transaction is then valid on both sides of the fork.
    pub kernel_offset: bool,
}

impl BuildRules {
    /// The rules for a transaction aimed at block `next_height` on `network`.
    pub fn for_height(network: nightfall_types::NetworkId, next_height: u64) -> Self {
        Self {
            kernel_offset: next_height >= network.binding_fork_height(),
        }
    }
}

/// Build a transfer. Change is returned to `change_to`.
///
/// Fee is burned in full: it appears only as the public `fee·G` term in the
/// balance equation and is credited to nobody.
///
/// Always carries the binding fields and never a kernel offset, so the result
/// is valid on both sides of the binding fork. See [`build_transfer_with`].
pub fn build_transfer(
    owner: &WalletKeys,
    spendables: &[Spendable],
    payments: &[Payment],
    fee_darks: u64,
    change_to: &Address,
    lock_height: u64,
    ctx: &[u8],
) -> Result<Transaction, BuildError> {
    build_transfer_with(
        owner,
        spendables,
        payments,
        fee_darks,
        change_to,
        lock_height,
        ctx,
        BuildRules::default(),
    )
}

/// [`build_transfer`] with explicit [`BuildRules`].
#[allow(clippy::too_many_arguments)]
pub fn build_transfer_with(
    owner: &WalletKeys,
    spendables: &[Spendable],
    payments: &[Payment],
    fee_darks: u64,
    change_to: &Address,
    lock_height: u64,
    ctx: &[u8],
    rules: BuildRules,
) -> Result<Transaction, BuildError> {
    if spendables.is_empty() {
        return Err(BuildError::NoInputs);
    }
    if spendables.len() > MAX_INPUTS {
        return Err(BuildError::Tx(TxError::TooManyInputs));
    }
    if payments.len().saturating_add(1) > MAX_OUTPUTS {
        return Err(BuildError::Tx(TxError::TooManyOutputs));
    }
    let in_sum: u64 = spendables.iter().map(|s| s.value).sum();
    let pay_sum: u64 = payments.iter().map(|p| p.amount).sum();
    let needed = pay_sum
        .checked_add(fee_darks)
        .ok_or(BuildError::AmountOverflow)?;
    if in_sum < needed {
        return Err(BuildError::InsufficientFunds {
            have: in_sum,
            need: needed,
        });
    }
    let change = in_sum - needed;

    let mut outputs = Vec::new();
    let mut out_blind_sum = Scalar::ZERO;
    let mut out_ephemeral_sum = Scalar::ZERO;

    for p in payments {
        let (o, s) =
            create_output(&p.to, p.amount, &p.memo, ctx).map_err(|_| BuildError::OutputFailed)?;
        out_blind_sum += s.blind;
        out_ephemeral_sum += s.ephemeral;
        outputs.push(o);
    }

    // Always emit a change output, even for zero change: a transaction with a
    // single output leaks that the whole input was consumed.
    let (change_out, change_secrets) =
        create_output(change_to, change, "", ctx).map_err(|_| BuildError::OutputFailed)?;
    out_blind_sum += change_secrets.blind;
    out_ephemeral_sum += change_secrets.ephemeral;
    outputs.push(change_out);

    let in_blind_sum: Scalar = spendables.iter().fold(Scalar::ZERO, |acc, s| acc + s.blind);

    // Σout − Σin + fee·G = excess·H + k·H  ⇒  excess = Σ out_blinds − Σ in_blinds − k
    let kernel_offset = if rules.kernel_offset {
        Some(nightfall_crypto::random_scalar())
    } else {
        None
    };
    let excess_secret = out_blind_sum - in_blind_sum - kernel_offset.unwrap_or(Scalar::ZERO);

    let mut inputs = Vec::with_capacity(spendables.len());
    let mut in_key_sum = Scalar::ZERO;
    for s in spendables {
        // Sanity: the secret must actually open the output key we are spending.
        debug_assert_eq!(
            Commitment::new(s.value, &s.blind),
            s.commit,
            "spendable blind does not open its commitment"
        );
        let msg = Transaction::input_message(&s.commit);
        let output_pk = (generator_g() * s.spend_secret).compress().to_bytes();
        let ki = nightfall_crypto::derive_input_secret(&s.spend_secret, &s.commit);
        let (input_key, key_sig) =
            nightfall_crypto::sign_input_key(&ki, &s.spend_secret, &s.commit, &output_pk);
        in_key_sum += ki;
        inputs.push(Input {
            commit: s.commit,
            sig: sig::sign(&s.spend_secret, &generator_g(), &msg),
            input_key: Some(input_key),
            key_sig: Some(key_sig),
        });
    }

    let mut kernel = build_kernel(
        KernelFeature::Plain,
        fee_darks,
        0,
        lock_height,
        &excess_secret,
    );
    // Σ Ki − Σ Ke = X + s·G  ⇒  x = Σ ki − Σ r − s
    let stealth_offset = nightfall_crypto::random_scalar();
    kernel.attach_stealth(&(in_key_sum - out_ephemeral_sum - stealth_offset));

    let tx = Transaction {
        version: TX_VERSION,
        inputs,
        outputs,
        kernels: vec![kernel],
        stealth_offset: Some(stealth_offset.to_bytes()),
        kernel_offset: kernel_offset.map(|k| k.to_bytes()),
    };

    let _ = owner; // owner is implied by the per-output spend secrets
                   // Transaction builders must never return a transaction that the ledger
                   // will reject solely because its shape exceeds consensus limits.
    tx.check_shape()?;
    Ok(tx)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BuildError {
    #[error("no inputs selected")]
    NoInputs,
    #[error("insufficient funds: have {have} darks, need {need}")]
    InsufficientFunds { have: u64, need: u64 },
    #[error("amount overflow")]
    AmountOverflow,
    #[error("could not build output")]
    OutputFailed,
    #[error(transparent)]
    Tx(#[from] TxError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_builder_rejects_more_than_consensus_input_limit() {
        let owner = WalletKeys::from_seed([1u8; 32]);
        let recipient = WalletKeys::from_seed([2u8; 32]).address();

        let spendable = Spendable {
            commit: Commitment::new(1, &Scalar::ONE),
            value: 1,
            blind: Scalar::ONE,
            spend_secret: Scalar::ONE,
        };
        let spendables = vec![spendable; MAX_INPUTS + 1];

        let err = build_transfer(
            &owner,
            &spendables,
            &[Payment {
                to: recipient,
                amount: 1,
                memo: String::new(),
            }],
            0,
            &owner.address(),
            0,
            b"test",
        )
        .unwrap_err();

        assert_eq!(err, BuildError::Tx(TxError::TooManyInputs));
    }
}
