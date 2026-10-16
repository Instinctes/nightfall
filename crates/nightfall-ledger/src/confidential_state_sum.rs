//! Experimental, non-consensus Merkle-sum commitment evidence.
//!
//! The existing protocol-v8 UTXO root does NOT commit to this construction.
//! No caller may treat these proofs as mainnet-authenticated until an audited,
//! consensus-anchored root and independently verified state transition exist.
//! This module does not change UTXO validation, consensus, state or wallets.

use curve25519_dalek::ristretto::RistrettoPoint;
use curve25519_dalek::traits::Identity;
use nightfall_crypto::{commit_public_value, hash_multi, Commitment};
use nightfall_types::{Hash256, MAX_SUPPLY_DARKS};

const LEAF_DOMAIN: &[u8] = b"nightfall:research:sum:leaf:v1";
const INNER_DOMAIN: &[u8] = b"nightfall:research:sum:inner:v1";
const EMPTY_DOMAIN: &[u8] = b"nightfall:research:sum:empty:v1";
const ROOT_DOMAIN: &[u8] = b"nightfall:research:sum:root:v1";
/// Research-only allocation ceiling; not a mainnet UTXO capacity rule.
pub const MAX_RESEARCH_LEAVES: usize = 131_072;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateLeaf {
    pub commitment: Commitment,
    pub output_pk: [u8; 32],
    pub created_height: u64,
    pub coinbase: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SumNode {
    pub hash: Hash256,
    pub sum: Commitment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SumRoot {
    pub hash: Hash256,
    pub sum: Commitment,
    pub leaf_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MembershipProof {
    pub leaf_index: u64,
    pub leaf_count: u64,
    /// Bottom-up path; includes canonical empty siblings on odd levels.
    pub siblings: Vec<SumNode>,
}

fn empty_node() -> SumNode {
    SumNode {
        hash: hash_multi(EMPTY_DOMAIN, &[]),
        sum: Commitment::from_point(RistrettoPoint::identity()),
    }
}

fn leaf_node(leaf: &StateLeaf) -> Result<SumNode, &'static str> {
    if leaf.commitment.point().is_none() {
        return Err("invalid leaf commitment encoding");
    }
    let height = leaf.created_height.to_le_bytes();
    let coinbase = [u8::from(leaf.coinbase)];
    Ok(SumNode {
        hash: hash_multi(
            LEAF_DOMAIN,
            &[&leaf.commitment.0, &leaf.output_pk, &height, &coinbase],
        ),
        sum: leaf.commitment,
    })
}

fn parent(left: SumNode, right: SumNode) -> Result<SumNode, &'static str> {
    let a = left.sum.point().ok_or("invalid left sum encoding")?;
    let b = right.sum.point().ok_or("invalid right sum encoding")?;
    let sum = Commitment::from_point(a + b);
    Ok(SumNode {
        hash: hash_multi(
            INNER_DOMAIN,
            &[&left.hash.0, &right.hash.0, &left.sum.0, &right.sum.0],
        ),
        sum,
    })
}

fn root_from_node(node: SumNode, leaf_count: usize) -> SumRoot {
    let count = (leaf_count as u64).to_le_bytes();
    SumRoot {
        hash: hash_multi(ROOT_DOMAIN, &[&count, &node.hash.0, &node.sum.0]),
        sum: node.sum,
        leaf_count: leaf_count as u64,
    }
}

/// Deterministic research tree over a *caller-supplied* set.
/// Neither the caller's leaf list nor this root is authenticated by v8 consensus.
pub struct ResearchTree {
    leaves: Vec<StateLeaf>,
    levels: Vec<Vec<SumNode>>,
}

impl ResearchTree {
    pub fn build(mut leaves: Vec<StateLeaf>) -> Result<Self, &'static str> {
        if leaves.len() > MAX_RESEARCH_LEAVES {
            return Err("too many research leaves");
        }
        leaves.sort_unstable_by_key(|item| item.commitment.0);
        for pair in leaves.windows(2) {
            if pair[0].commitment == pair[1].commitment {
                return Err("duplicate commitment");
            }
        }
        let mut first = Vec::with_capacity(leaves.len().max(1));
        for leaf in &leaves {
            first.push(leaf_node(leaf)?);
        }
        if first.is_empty() {
            first.push(empty_node());
        }
        let mut levels = vec![first];
        while levels.last().expect("nonempty").len() > 1 {
            let current = levels.last().expect("nonempty");
            let mut next = Vec::with_capacity(current.len().div_ceil(2));
            for pair in current.chunks(2) {
                next.push(parent(pair[0], *pair.get(1).unwrap_or(&empty_node()))?);
            }
            levels.push(next);
        }
        Ok(Self { leaves, levels })
    }

    pub fn root(&self) -> SumRoot {
        let node = self.levels.last().expect("nonempty levels")[0];
        root_from_node(node, self.leaves.len())
    }

    pub fn prove(&self, commitment: Commitment) -> Option<(StateLeaf, MembershipProof)> {
        let index = self
            .leaves
            .binary_search_by_key(&commitment.0, |x| x.commitment.0)
            .ok()?;
        let mut offset = index;
        let mut siblings = Vec::with_capacity(self.levels.len().saturating_sub(1));
        for level in self.levels.iter().take(self.levels.len() - 1) {
            let sibling = offset ^ 1;
            siblings.push(level.get(sibling).copied().unwrap_or_else(empty_node));
            offset /= 2;
        }
        Some((
            self.leaves[index].clone(),
            MembershipProof {
                leaf_index: index as u64,
                leaf_count: self.leaves.len() as u64,
                siblings,
            },
        ))
    }
}

/// Check a member of an explicitly supplied expected root.
/// If expected_root came from an untrusted server, the result is only
/// self-consistency, NOT independently validated blockchain membership.
pub fn verify_member(leaf: &StateLeaf, proof: &MembershipProof, expected_root: &SumRoot) -> bool {
    if proof.leaf_count == 0
        || proof.leaf_count != expected_root.leaf_count
        || proof.leaf_count > MAX_RESEARCH_LEAVES as u64
        || proof.leaf_index >= proof.leaf_count
    {
        return false;
    }
    let mut width = proof.leaf_count as usize;
    let mut depth = 0;
    while width > 1 {
        depth += 1;
        width = width.div_ceil(2);
    }
    if proof.siblings.len() != depth {
        return false;
    }
    let Ok(mut node) = leaf_node(leaf) else {
        return false;
    };
    let mut offset = proof.leaf_index as usize;
    let mut level_width = proof.leaf_count as usize;
    for sibling in &proof.siblings {
        // A missing right sibling has one precise canonical representation.
        if offset ^ 1 >= level_width && *sibling != empty_node() {
            return false;
        }
        node = match if offset & 1 == 0 {
            parent(node, *sibling)
        } else {
            parent(*sibling, node)
        } {
            Ok(node) => node,
            Err(_) => return false,
        };
        offset /= 2;
        level_width = level_width.div_ceil(2);
    }
    root_from_node(node, proof.leaf_count as usize) == *expected_root
}

/// Test the conservation equation for the supplied aggregate commitments.
///
/// This does NOT establish authentic UTXO completeness, kernel validity,
/// supply counters, range proofs, or that a root is committed by chain history.
/// An independent source of all those facts is a prerequisite for a certificate.
pub fn check_reported_conservation(
    root: &SumRoot,
    kernel_excess_sum: Commitment,
    minted_darks: u64,
    burned_darks: u64,
) -> bool {
    if minted_darks > MAX_SUPPLY_DARKS || burned_darks > minted_darks {
        return false;
    }
    let Some(utxo_sum) = root.sum.point() else {
        return false;
    };
    let Some(kernel_sum) = kernel_excess_sum.point() else {
        return false;
    };
    utxo_sum - kernel_sum == commit_public_value(minted_darks - burned_darks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use curve25519_dalek::scalar::Scalar;

    fn leaf(n: u64) -> StateLeaf {
        StateLeaf {
            commitment: Commitment::new(n, &Scalar::from(n + 20)),
            output_pk: [n as u8; 32],
            created_height: n,
            coinbase: n == 0,
        }
    }

    #[test]
    fn exact_all_member_proofs_across_odd_and_even_sets() {
        for count in [1, 2, 3, 4, 5, 7, 8, 9, 17] {
            let input: Vec<_> = (1..=count).map(leaf).rev().collect();
            let tree = ResearchTree::build(input.clone()).unwrap();
            for item in input {
                let (found, proof) = tree.prove(item.commitment).unwrap();
                assert!(verify_member(&found, &proof, &tree.root()));
            }
        }
    }

    #[test]
    fn empty_state_is_not_a_member() {
        let tree = ResearchTree::build(vec![]).unwrap();
        assert_eq!(tree.root().leaf_count, 0);
        assert!(tree.prove(leaf(1).commitment).is_none());
        assert!(check_reported_conservation(
            &tree.root(),
            Commitment::identity(),
            0,
            0
        ));
    }

    #[test]
    fn deterministic_root_independent_of_input_order() {
        let mut list: Vec<_> = (1..=6).map(leaf).collect();
        let one = ResearchTree::build(list.clone()).unwrap().root();
        list.reverse();
        assert_eq!(one, ResearchTree::build(list).unwrap().root());
    }

    #[test]
    fn refuses_duplicate_even_if_other_metadata_differs() {
        let mut b = leaf(1);
        b.output_pk = [255; 32];
        assert!(ResearchTree::build(vec![leaf(1), b]).is_err());
    }

    #[test]
    fn tampering_to_member_or_path_fails_closed() {
        let tree = ResearchTree::build((1..=5).map(leaf).collect()).unwrap();
        let (item, proof) = tree.prove(leaf(3).commitment).unwrap();
        let mut other = item.clone();
        other.created_height += 1;
        assert!(!verify_member(&other, &proof, &tree.root()));
        other = item.clone();
        other.coinbase = !other.coinbase;
        assert!(!verify_member(&other, &proof, &tree.root()));
        other = item.clone();
        other.output_pk[0] ^= 1;
        assert!(!verify_member(&other, &proof, &tree.root()));

        let mut altered = proof.clone();
        altered.siblings[0].hash.0[0] ^= 1;
        assert!(!verify_member(&item, &altered, &tree.root()));
        let mut altered = proof.clone();
        altered.siblings[0].sum = Commitment::identity();
        assert!(!verify_member(&item, &altered, &tree.root()));
        let mut altered = proof.clone();
        altered.leaf_index = 99;
        assert!(!verify_member(&item, &altered, &tree.root()));
        let mut altered = proof.clone();
        altered.siblings.pop();
        assert!(!verify_member(&item, &altered, &tree.root()));
        let mut altered = proof.clone();
        altered.leaf_count += 1;
        assert!(!verify_member(&item, &altered, &tree.root()));
    }

    #[test]
    fn roots_bind_leaf_count_sum_and_metadata() {
        let a = ResearchTree::build(vec![leaf(1), leaf(2)]).unwrap().root();
        let b = ResearchTree::build(vec![leaf(1), leaf(2), leaf(3)])
            .unwrap()
            .root();
        assert_ne!(a.hash, b.hash);
        let mut corrupt = a;
        corrupt.sum = Commitment::identity();
        let tree = ResearchTree::build(vec![leaf(1), leaf(2)]).unwrap();
        let (item, proof) = tree.prove(leaf(1).commitment).unwrap();
        assert!(!verify_member(&item, &proof, &corrupt));
    }

    #[test]
    fn supply_equation_catches_wrong_amounts_and_kernel() {
        let item = StateLeaf {
            commitment: Commitment::new(100, &Scalar::from(17u64)),
            output_pk: [42; 32],
            created_height: 1,
            coinbase: false,
        };
        let root = ResearchTree::build(vec![item]).unwrap().root();
        let kernel = Commitment::from_point(nightfall_crypto::generator_h() * Scalar::from(17u64));
        assert!(check_reported_conservation(&root, kernel, 120, 20));
        assert!(!check_reported_conservation(&root, kernel, 121, 20));
        assert!(!check_reported_conservation(
            &root,
            Commitment::identity(),
            120,
            20
        ));
        assert!(!check_reported_conservation(&root, kernel, 20, 21));
        assert!(!check_reported_conservation(
            &root,
            kernel,
            MAX_SUPPLY_DARKS + 1,
            0
        ));
    }
}
