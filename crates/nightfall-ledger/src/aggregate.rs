//! Block-level aggregation.
//!
//! A block does not store transactions. It stores one flat, canonically sorted
//! set of inputs, one of outputs and one of kernels. Every transaction that
//! went into it dissolves.
//!
//! # What this achieves
//!
//! An observer sees *n* inputs, *m* outputs and *k* kernels with **no
//! indication of which input paid which output**. The anonymity set for a
//! payment becomes every transaction in the block, rather than the payment
//! standing alone as its own visible object. This is CoinJoin at the block
//! level, applied to every block automatically and without coordination.
//!
//! Sorting is what does the work. Inputs and outputs are ordered by
//! commitment, kernels by identifier — all of which are effectively random
//! values, so position carries no information about origin. Leaving the
//! original order intact would have made grouping trivially recoverable.
//!
//! # What this does not achieve
//!
//! **Cut-through is not applied**, and cannot be under this design. In plain
//! Mimblewimble an output created and spent inside the same block can be
//! deleted along with the input that spends it, erasing the intermediate step
//! entirely. That works because ownership there is proven solely by knowing a
//! blinding factor, which the kernel already covers.
//!
//! Nightfall additionally signs each input with the one-time key `Ko` of the
//! output being spent. That signature is what makes *non-interactive* payments
//! safe — without it the sender, who knows the output's blinding factor, could
//! sweep back what they sent. Deleting the input would delete that
//! authorisation, so cut-through and one-sided payments are mutually
//! exclusive. The `docs/DECISIONS.md` lock chose one-sided payments, because a
//! wallet where both parties must be online at once is not a wallet most
//! people can use.
//!
//! The consequence, stated plainly: **spent outputs remain visible on chain,
//! so the transaction graph is obscured but not erased.** `docs/ATTRIBUTES.md`
//! claims a full-set anonymity class; aggregation narrows that gap without
//! closing it. Closing it needs a proof system that can authorise a spend
//! without publishing a per-input signature.

use nightfall_crypto::{domain, hash_multi, Commitment, KernelFeature, Output, TxKernel};
use nightfall_types::Hash256;
use serde::{Deserialize, Serialize};

use crate::tx::{Input, Transaction};

/// The aggregated contents of a block.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockBody {
    pub inputs: Vec<Input>,
    pub outputs: Vec<Output>,
    pub kernels: Vec<TxKernel>,
}

impl BlockBody {
    /// Merge transactions into one aggregate and sort it canonically.
    ///
    /// The coinbase transaction is folded in exactly like any other; it is
    /// identified afterwards only by its kernel feature, not by position.
    pub fn aggregate(txs: &[Transaction]) -> Self {
        let mut body = Self::default();
        for tx in txs {
            body.inputs.extend(tx.inputs.iter().cloned());
            body.outputs.extend(tx.outputs.iter().cloned());
            body.kernels.extend(tx.kernels.iter().cloned());
        }
        body.canonicalise();
        body
    }

    /// Sort every set into the one order the network agrees on.
    ///
    /// Both a privacy measure (position reveals nothing about which
    /// transaction an object came from) and a consensus one (every node
    /// computes the same body hash).
    pub fn canonicalise(&mut self) {
        self.inputs.sort_by_key(|i| i.commit.0);
        self.outputs.sort_by_key(|o| o.commit.0);
        self.kernels.sort_by_key(|k| k.id().0);
    }

    pub fn is_canonical(&self) -> bool {
        let mut copy = self.clone();
        copy.canonicalise();
        copy == *self
    }

    pub fn total_fee(&self) -> u64 {
        self.kernels.iter().map(|k| k.fee_darks).sum()
    }

    pub fn total_reward(&self) -> u64 {
        self.kernels.iter().map(|k| k.reward_darks).sum()
    }

    pub fn coinbase_kernels(&self) -> usize {
        self.kernels
            .iter()
            .filter(|k| k.feature == KernelFeature::Coinbase)
            .count()
    }

    /// Outputs created by the coinbase.
    ///
    /// A block has exactly one coinbase kernel and the subsidy goes to exactly
    /// one output. Since aggregation destroys ordering, the coinbase output is
    /// identified by the balance equation rather than by position: it is the
    /// output that is not accounted for by any input. Rather than guess, the
    /// builder marks it explicitly — see [`BlockBody::coinbase_commit`].
    pub fn coinbase_commit(&self, coinbase_tx: &Transaction) -> Option<Commitment> {
        coinbase_tx.outputs.first().map(|o| o.commit)
    }

    /// Hash over the whole aggregate. Committed to by the block header.
    pub fn hash(&self) -> Hash256 {
        let mut parts: Vec<Vec<u8>> = Vec::new();
        for i in &self.inputs {
            let mut v = i.commit.0.to_vec();
            v.extend_from_slice(&i.sig.r);
            v.extend_from_slice(&i.sig.s);
            parts.push(v);
        }
        for o in &self.outputs {
            let mut v = o.commitment_bytes();
            v.extend_from_slice(&o.sender_sig.r);
            v.extend_from_slice(&o.sender_sig.s);
            parts.push(v);
        }
        for k in &self.kernels {
            let mut v = k.signing_message();
            v.extend_from_slice(&k.excess_sig.r);
            v.extend_from_slice(&k.excess_sig.s);
            parts.push(v);
        }
        let refs: Vec<&[u8]> = parts.iter().map(|p| p.as_slice()).collect();
        hash_multi(domain::TXBODY, &refs)
    }

    /// Sum of every kernel excess in the block.
    pub fn kernel_excess_sum(&self) -> Option<Commitment> {
        let list: Vec<Commitment> = self.kernels.iter().map(|k| k.excess).collect();
        Commitment::sum(&list).map(Commitment::from_point)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::{build_coinbase, build_transfer, Payment, Spendable};
    use nightfall_crypto::{scan_output, WalletKeys};
    use nightfall_types::{NetworkId, DARKS_PER_NIGHT};

    fn ctx() -> &'static [u8] {
        NetworkId::Devnet.proof_context()
    }

    fn funded(miner: &WalletKeys) -> (Transaction, Vec<Spendable>) {
        let cb = build_coinbase(&miner.address(), 20 * DARKS_PER_NIGHT, 0, ctx()).unwrap();
        let view = miner.view_key();
        let spendables = cb
            .outputs
            .iter()
            .filter_map(|o| {
                scan_output(&view, o).map(|d| Spendable {
                    commit: d.commit,
                    value: d.value,
                    blind: d.blind,
                    spend_secret: d.spend_secret(miner),
                })
            })
            .collect();
        (cb, spendables)
    }

    #[test]
    fn aggregation_merges_and_sorts() {
        let miner = WalletKeys::generate();
        let bob = WalletKeys::generate();
        let (cb, spendables) = funded(&miner);

        let tx = build_transfer(
            &miner,
            &spendables,
            &[Payment {
                to: bob.address(),
                amount: DARKS_PER_NIGHT,
                memo: String::new(),
            }],
            1_000,
            &miner.address(),
            0,
            ctx(),
        )
        .unwrap();

        let body = BlockBody::aggregate(&[cb.clone(), tx.clone()]);

        assert_eq!(body.inputs.len(), tx.inputs.len());
        assert_eq!(body.outputs.len(), cb.outputs.len() + tx.outputs.len());
        assert_eq!(body.kernels.len(), 2);
        assert!(body.is_canonical());
    }

    #[test]
    fn ordering_does_not_leak_transaction_grouping() {
        // Aggregating the same transactions in a different order must produce a
        // byte-identical body. If it did not, the order would carry
        // information about which transaction arrived first.
        let miner = WalletKeys::generate();
        let bob = WalletKeys::generate();
        let (cb, spendables) = funded(&miner);

        let tx = build_transfer(
            &miner,
            &spendables,
            &[Payment {
                to: bob.address(),
                amount: DARKS_PER_NIGHT,
                memo: String::new(),
            }],
            1_000,
            &miner.address(),
            0,
            ctx(),
        )
        .unwrap();

        let a = BlockBody::aggregate(&[cb.clone(), tx.clone()]);
        let b = BlockBody::aggregate(&[tx, cb]);
        assert_eq!(a, b);
        assert_eq!(a.hash(), b.hash());
    }

    #[test]
    fn aggregate_preserves_the_balance_equation() {
        let miner = WalletKeys::generate();
        let bob = WalletKeys::generate();
        let reward = 20 * DARKS_PER_NIGHT;
        let (cb, spendables) = funded(&miner);
        let fee = 1_000u64;

        let tx = build_transfer(
            &miner,
            &spendables,
            &[Payment {
                to: bob.address(),
                amount: DARKS_PER_NIGHT,
                memo: String::new(),
            }],
            fee,
            &miner.address(),
            0,
            ctx(),
        )
        .unwrap();

        let body = BlockBody::aggregate(&[cb, tx]);

        let in_commits: Vec<_> = body.inputs.iter().map(|i| i.commit).collect();
        let out_commits: Vec<_> = body.outputs.iter().map(|o| o.commit).collect();
        let expected = nightfall_crypto::expected_excess(
            &in_commits,
            &out_commits,
            body.total_fee(),
            body.total_reward(),
        )
        .unwrap();

        assert_eq!(
            Commitment::from_point(expected),
            body.kernel_excess_sum().unwrap(),
            "the aggregate must balance exactly as the parts did"
        );
        assert_eq!(body.total_fee(), fee);
        assert_eq!(body.total_reward(), reward);
    }

    #[test]
    fn every_signature_survives_aggregation() {
        let miner = WalletKeys::generate();
        let (cb, _) = funded(&miner);
        let body = BlockBody::aggregate(&[cb]);

        for k in &body.kernels {
            assert!(k.verify_signature(), "kernel signature must survive");
        }
        for o in &body.outputs {
            assert!(o.verify_sender_sig(), "output signature must survive");
        }
    }
}
