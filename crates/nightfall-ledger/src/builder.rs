//! Wallet-side transaction construction.

use curve25519_dalek::scalar::Scalar;
use nightfall_crypto::{
    build_kernel, create_output, create_output_with_features, generator_g, sig, Address,
    Commitment, KernelFeature, WalletKeys,
};

use crate::tx::{Input, Transaction, TxError};

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
    };

    // Kernel excess = Σ out_blinds (no inputs).
    // `lock_height` carries the block height, which pins a coinbase kernel to
    // exactly one height and stops it being replayed at another.
    let excess_secret = secrets.blind;
    tx.kernels = vec![build_kernel(
        KernelFeature::Coinbase,
        0,
        reward_darks,
        height,
        &excess_secret,
    )];
    Ok(tx)
}

/// Build a transfer. Change is returned to `change_to`.
///
/// Fee is burned in full: it appears only as the public `fee·G` term in the
/// balance equation and is credited to nobody.
pub fn build_transfer(
    owner: &WalletKeys,
    spendables: &[Spendable],
    payments: &[Payment],
    fee_darks: u64,
    change_to: &Address,
    lock_height: u64,
    ctx: &[u8],
) -> Result<Transaction, BuildError> {
    if spendables.is_empty() {
        return Err(BuildError::NoInputs);
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

    for p in payments {
        let (o, s) =
            create_output(&p.to, p.amount, &p.memo, ctx).map_err(|_| BuildError::OutputFailed)?;
        out_blind_sum += s.blind;
        outputs.push(o);
    }

    // Always emit a change output, even for zero change: a transaction with a
    // single output leaks that the whole input was consumed.
    let (change_out, change_secrets) =
        create_output(change_to, change, "", ctx).map_err(|_| BuildError::OutputFailed)?;
    out_blind_sum += change_secrets.blind;
    outputs.push(change_out);

    let in_blind_sum: Scalar = spendables.iter().fold(Scalar::ZERO, |acc, s| acc + s.blind);

    // Σout − Σin + fee·G = excess·H  ⇒  excess = Σ out_blinds − Σ in_blinds
    let excess_secret = out_blind_sum - in_blind_sum;

    let mut tx = Transaction {
        version: TX_VERSION,
        inputs: spendables
            .iter()
            .map(|s| Input {
                commit: s.commit,
                sig: nightfall_crypto::SchnorrSig {
                    r: [0u8; 32],
                    s: [0u8; 32],
                },
            })
            .collect(),
        outputs,
        kernels: vec![],
    };

    tx.kernels = vec![build_kernel(
        KernelFeature::Plain,
        fee_darks,
        0,
        lock_height,
        &excess_secret,
    )];

    for (i, s) in spendables.iter().enumerate() {
        // Sanity: the secret must actually open the output key we are spending.
        debug_assert_eq!(
            Commitment::new(s.value, &s.blind),
            s.commit,
            "spendable blind does not open its commitment"
        );
        let msg = Transaction::input_message(&s.commit);
        tx.inputs[i].sig = sig::sign(&s.spend_secret, &generator_g(), &msg);
    }

    let _ = owner; // owner is implied by the per-output spend secrets
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
