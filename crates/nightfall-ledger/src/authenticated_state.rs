//! Experimental authenticated UTXO state.
//!
//! IMPORTANT:
//! This module is deliberately NOT consensus-active.
//! Protocol v8's existing UTXO root remains authoritative.
//!
//! Goals of this first stage:
//! - incremental O(log keyspace) state updates
//! - authenticate every consensus-relevant UTXO field
//! - deterministic membership proofs
//! - deterministic rollback by deleting/reinserting leaves
//! - no genesis change
//! - no remint
//! - no change to existing ownership

use crate::{UtxoEntry, UtxoSet};
use nightfall_crypto::hash_multi;
use nightfall_types::Hash256;
use std::collections::BTreeMap;

const TREE_DEPTH: usize = 256;

const SMT_EMPTY_LEAF_DOMAIN: &[u8] = b"nightfall:utxo-smt:empty-leaf:v1";

const SMT_LEAF_DOMAIN: &[u8] = b"nightfall:utxo-smt:leaf:v1";

const SMT_NODE_DOMAIN: &[u8] = b"nightfall:utxo-smt:node:v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct NodeKey {
    depth: u16,
    prefix: [u8; 32],
}

/// Sparse Merkle membership proof.
///
/// Siblings are stored bottom-up:
/// siblings[0] is the sibling of the leaf,
/// siblings[255] is the sibling immediately below the root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UtxoMembershipProof {
    pub siblings: Vec<Hash256>,
}

/// Experimental incremental authenticated UTXO tree.
///
/// This is intentionally kept separate from `LedgerState`.
/// Nothing in protocol v8 reads this root yet.
#[derive(Clone, Debug)]
pub struct AuthenticatedUtxoTree {
    entries: BTreeMap<[u8; 32], UtxoEntry>,

    // Only non-default tree nodes are stored.
    nodes: BTreeMap<NodeKey, Hash256>,

    // empty[d] is the canonical empty subtree hash at depth d.
    empty: [Hash256; TREE_DEPTH + 1],
}

impl Default for AuthenticatedUtxoTree {
    fn default() -> Self {
        Self::new()
    }
}

impl AuthenticatedUtxoTree {
    pub fn new() -> Self {
        let empty = build_empty_hashes();

        Self {
            entries: BTreeMap::new(),
            nodes: BTreeMap::new(),
            empty,
        }
    }

    /// Build the authenticated state from today's canonical UTXO set.
    ///
    /// This is the bridge needed for a future shadow-mode comparison.
    /// It does NOT replace `UtxoSet::root()`.
    pub fn from_utxo_set(set: &UtxoSet) -> Self {
        let mut tree = Self::new();

        for (commit, entry) in &set.entries {
            tree.insert(*commit, entry.clone());
        }

        tree
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn contains_key(&self, key: &[u8; 32]) -> bool {
        self.entries.contains_key(key)
    }

    pub fn root(&self) -> Hash256 {
        self.node_or_empty(0, [0u8; 32])
    }

    /// Insert or replace one UTXO and update only its 256-node path.
    pub fn insert(&mut self, key: [u8; 32], entry: UtxoEntry) -> Option<UtxoEntry> {
        let old = self.entries.insert(key, entry.clone());

        let leaf = leaf_hash(&key, &entry);
        self.set_node(TREE_DEPTH, key, leaf);
        self.recompute_path(key);

        old
    }

    /// Delete one UTXO and update only its path back to the root.
    pub fn remove(&mut self, key: &[u8; 32]) -> Option<UtxoEntry> {
        let old = self.entries.remove(key)?;

        self.set_node(TREE_DEPTH, *key, self.empty[TREE_DEPTH]);

        self.recompute_path(*key);

        Some(old)
    }

    pub fn prove(&self, key: &[u8; 32]) -> Option<UtxoMembershipProof> {
        self.entries.get(key)?;

        let mut siblings = Vec::with_capacity(TREE_DEPTH);

        // Walk leaf -> root.
        for parent_depth in (0..TREE_DEPTH).rev() {
            let mut sibling = prefix_at(key, parent_depth);

            let our_bit = bit_at(key, parent_depth);

            // Sibling takes the opposite branch.
            set_bit(&mut sibling, parent_depth, !our_bit);

            let sibling_hash = self.node_or_empty(parent_depth + 1, sibling);

            siblings.push(sibling_hash);
        }

        Some(UtxoMembershipProof { siblings })
    }

    fn recompute_path(&mut self, key: [u8; 32]) {
        for parent_depth in (0..TREE_DEPTH).rev() {
            let parent_prefix = prefix_at(&key, parent_depth);

            let mut left_prefix = parent_prefix;
            let mut right_prefix = parent_prefix;

            set_bit(&mut left_prefix, parent_depth, false);

            set_bit(&mut right_prefix, parent_depth, true);

            let child_depth = parent_depth + 1;

            let left = self.node_or_empty(child_depth, left_prefix);

            let right = self.node_or_empty(child_depth, right_prefix);

            let parent_hash = node_hash(left, right);

            self.set_node(parent_depth, parent_prefix, parent_hash);
        }
    }

    fn node_or_empty(&self, depth: usize, prefix: [u8; 32]) -> Hash256 {
        self.nodes
            .get(&NodeKey {
                depth: depth as u16,
                prefix,
            })
            .copied()
            .unwrap_or(self.empty[depth])
    }

    fn set_node(&mut self, depth: usize, prefix: [u8; 32], hash: Hash256) {
        let key = NodeKey {
            depth: depth as u16,
            prefix,
        };

        if hash == self.empty[depth] {
            self.nodes.remove(&key);
        } else {
            self.nodes.insert(key, hash);
        }
    }
}

/// Verify a membership proof without access to the whole UTXO set.
pub fn verify_utxo_membership(
    root: Hash256,
    key: &[u8; 32],
    entry: &UtxoEntry,
    proof: &UtxoMembershipProof,
) -> bool {
    if proof.siblings.len() != TREE_DEPTH {
        return false;
    }

    let mut current = leaf_hash(key, entry);

    for (i, sibling) in proof.siblings.iter().enumerate() {
        let parent_depth = TREE_DEPTH - 1 - i;

        if bit_at(key, parent_depth) {
            current = node_hash(*sibling, current);
        } else {
            current = node_hash(current, *sibling);
        }
    }

    current == root
}

/// Authenticate all state that changes spend semantics.
///
/// Note especially `is_coinbase`: the current v8 `UtxoSet::root()`
/// does not include this flag in its leaf hash.  This experimental
/// construction deliberately does.
fn leaf_hash(key: &[u8; 32], entry: &UtxoEntry) -> Hash256 {
    let height = entry.height.to_le_bytes();
    let coinbase = [u8::from(entry.is_coinbase)];

    hash_multi(
        SMT_LEAF_DOMAIN,
        &[key, &entry.output_pk, &height, &coinbase],
    )
}

fn node_hash(left: Hash256, right: Hash256) -> Hash256 {
    hash_multi(SMT_NODE_DOMAIN, &[&left.0, &right.0])
}

fn build_empty_hashes() -> [Hash256; TREE_DEPTH + 1] {
    let mut empty = [Hash256::ZERO; TREE_DEPTH + 1];

    empty[TREE_DEPTH] = hash_multi(SMT_EMPTY_LEAF_DOMAIN, &[]);

    for depth in (0..TREE_DEPTH).rev() {
        empty[depth] = node_hash(empty[depth + 1], empty[depth + 1]);
    }

    empty
}

fn bit_at(key: &[u8; 32], bit: usize) -> bool {
    debug_assert!(bit < TREE_DEPTH);

    let byte = bit / 8;
    let shift = 7 - (bit % 8);

    ((key[byte] >> shift) & 1) != 0
}

fn set_bit(key: &mut [u8; 32], bit: usize, value: bool) {
    debug_assert!(bit < TREE_DEPTH);

    let byte = bit / 8;
    let shift = 7 - (bit % 8);
    let mask = 1u8 << shift;

    if value {
        key[byte] |= mask;
    } else {
        key[byte] &= !mask;
    }
}

/// Return exactly the first `depth` bits of a key.
/// Every bit below the prefix is zeroed.
fn prefix_at(key: &[u8; 32], depth: usize) -> [u8; 32] {
    debug_assert!(depth <= TREE_DEPTH);

    if depth == TREE_DEPTH {
        return *key;
    }

    let mut out = *key;

    let whole_bytes = depth / 8;
    let remainder = depth % 8;

    if remainder == 0 {
        for byte in &mut out[whole_bytes..] {
            *byte = 0;
        }
    } else {
        let keep_mask = 0xffu8 << (8 - remainder);

        out[whole_bytes] &= keep_mask;

        for byte in &mut out[(whole_bytes + 1)..] {
            *byte = 0;
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(marker: u8, height: u64, coinbase: bool) -> UtxoEntry {
        UtxoEntry {
            output_pk: [marker; 32],
            height,
            is_coinbase: coinbase,
        }
    }

    #[test]
    fn empty_root_is_deterministic() {
        let a = AuthenticatedUtxoTree::new();
        let b = AuthenticatedUtxoTree::new();

        assert_eq!(a.root(), b.root());
        assert_ne!(a.root(), Hash256::ZERO);
    }

    #[test]
    fn insertion_order_does_not_change_root() {
        let k1 = [0x11; 32];
        let k2 = [0x82; 32];
        let k3 = [0xf3; 32];

        let e1 = entry(1, 10, false);
        let e2 = entry(2, 20, true);
        let e3 = entry(3, 30, false);

        let mut a = AuthenticatedUtxoTree::new();
        a.insert(k1, e1.clone());
        a.insert(k2, e2.clone());
        a.insert(k3, e3.clone());

        let mut b = AuthenticatedUtxoTree::new();
        b.insert(k3, e3);
        b.insert(k1, e1);
        b.insert(k2, e2);

        assert_eq!(a.root(), b.root());
    }

    #[test]
    fn insert_then_remove_restores_exact_root() {
        let mut tree = AuthenticatedUtxoTree::new();
        let empty = tree.root();

        let key = [0xa5; 32];

        tree.insert(key, entry(7, 42, false));

        assert_ne!(tree.root(), empty);

        tree.remove(&key).unwrap();

        assert_eq!(tree.root(), empty);
        assert!(tree.is_empty());
    }

    #[test]
    fn all_spend_semantics_are_authenticated() {
        let key = [0x55; 32];

        let mut base = AuthenticatedUtxoTree::new();

        base.insert(key, entry(1, 100, false));

        let root = base.root();

        let mut changed_pk = AuthenticatedUtxoTree::new();

        changed_pk.insert(key, entry(2, 100, false));

        assert_ne!(root, changed_pk.root());

        let mut changed_height = AuthenticatedUtxoTree::new();

        changed_height.insert(key, entry(1, 101, false));

        assert_ne!(root, changed_height.root());

        let mut changed_coinbase = AuthenticatedUtxoTree::new();

        changed_coinbase.insert(key, entry(1, 100, true));

        assert_ne!(root, changed_coinbase.root());
    }

    #[test]
    fn valid_membership_proof_verifies() {
        let k1 = [0x01; 32];
        let k2 = [0x80; 32];

        let e1 = entry(1, 12, false);
        let e2 = entry(2, 25, true);

        let mut tree = AuthenticatedUtxoTree::new();

        tree.insert(k1, e1.clone());
        tree.insert(k2, e2);

        let proof = tree.prove(&k1).unwrap();

        assert!(verify_utxo_membership(tree.root(), &k1, &e1, &proof,));
    }

    #[test]
    fn modified_entry_fails_membership_proof() {
        let key = [0x33; 32];

        let original = entry(9, 1000, false);

        let mut tree = AuthenticatedUtxoTree::new();

        tree.insert(key, original.clone());

        let proof = tree.prove(&key).unwrap();

        let modified = entry(9, 1000, true);

        assert!(!verify_utxo_membership(
            tree.root(),
            &key,
            &modified,
            &proof,
        ));
    }

    #[test]
    fn replacement_changes_root_and_is_reversible() {
        let key = [0x42; 32];

        let first = entry(4, 400, false);

        let second = entry(5, 500, true);

        let mut tree = AuthenticatedUtxoTree::new();

        tree.insert(key, first.clone());

        let first_root = tree.root();

        tree.insert(key, second);

        assert_ne!(first_root, tree.root());

        tree.insert(key, first);

        assert_eq!(first_root, tree.root());
    }
}

// ============================================================================
// STAGE 2: ATOMIC SHADOW TRANSITIONS
// ============================================================================
//
// A shadow transition is applied only after the canonical ledger has accepted
// a block.  The shadow tree never decides whether a block is valid.
//
// Invariants:
//
// 1. Every canonical spend must remove exactly one existing shadow leaf.
// 2. Every canonical output must create exactly one new shadow leaf.
// 3. After the transition, the complete leaf map must equal UtxoSet::entries.
// 4. An incremental root must equal a root rebuilt from the canonical UTXO set.
// 5. Any mismatch leaves the original shadow tree byte-for-byte unchanged.
//
// This is deliberately NOT consensus-active.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShadowTransitionReport {
    pub previous_root: Hash256,
    pub new_root: Hash256,
    pub spent: usize,
    pub created: usize,
    pub resulting_utxos: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShadowStateError {
    /// Canonical ledger says an output was spent, but shadow state
    /// does not contain it.
    MissingSpentUtxo { commitment: [u8; 32] },

    /// Canonical block attempts to create a commitment which is still
    /// present in the staged shadow state.
    DuplicateCreatedUtxo { commitment: [u8; 32] },

    /// Incrementally updated shadow leaves differ from canonical UTXO state.
    CanonicalStateMismatch {
        shadow_len: usize,
        canonical_len: usize,
        first_mismatch: Option<[u8; 32]>,
    },

    /// Leaves are identical but incremental tree construction produced
    /// a different root from a clean deterministic rebuild.
    RebuildRootMismatch {
        incremental: Hash256,
        rebuilt: Hash256,
    },
}

impl std::fmt::Display for ShadowStateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingSpentUtxo { commitment } => {
                write!(
                    f,
                    "shadow state is missing spent UTXO {}",
                    hex32(commitment)
                )
            }

            Self::DuplicateCreatedUtxo { commitment } => {
                write!(
                    f,
                    "shadow transition would replace existing UTXO {}",
                    hex32(commitment)
                )
            }

            Self::CanonicalStateMismatch {
                shadow_len,
                canonical_len,
                first_mismatch,
            } => {
                write!(
                    f,
                    "shadow/canonical UTXO mismatch: shadow={}, canonical={}",
                    shadow_len, canonical_len
                )?;

                if let Some(key) = first_mismatch {
                    write!(f, ", first differing commitment={}", hex32(key))?;
                }

                Ok(())
            }

            Self::RebuildRootMismatch {
                incremental,
                rebuilt,
            } => {
                write!(
                    f,
                    "incremental authenticated-state root {:?} \
                     differs from deterministic rebuild {:?}",
                    incremental, rebuilt
                )
            }
        }
    }
}

impl std::error::Error for ShadowStateError {}

impl AuthenticatedUtxoTree {
    /// Exact leaf-level equality with Nightfall's canonical UTXO state.
    ///
    /// This intentionally compares the complete map rather than only a hash.
    /// During shadow deployment we want failures to be maximally observable.
    pub fn matches_utxo_set(&self, canonical: &UtxoSet) -> bool {
        self.entries == canonical.entries
    }

    /// Apply the state delta represented by an already-accepted block.
    ///
    /// IMPORTANT:
    ///
    /// This function performs NO consensus validation.  It assumes `body`
    /// has already been accepted by Nightfall's canonical LedgerState.
    ///
    /// `canonical_after` must be the canonical UTXO set AFTER that block was
    /// committed.
    ///
    /// The entire operation is transactional: all work happens on a clone.
    /// `self` changes only after every cross-check succeeds.
    pub fn apply_committed_block_shadow(
        &mut self,
        body: &crate::BlockBody,
        height: nightfall_types::Height,
        canonical_after: &UtxoSet,
    ) -> Result<ShadowTransitionReport, ShadowStateError> {
        let spent: Vec<[u8; 32]> = body.inputs.iter().map(|i| i.commit.0).collect();

        let created: Vec<([u8; 32], UtxoEntry)> = body
            .outputs
            .iter()
            .map(|out| {
                (
                    out.commit.0,
                    UtxoEntry {
                        output_pk: out.output_pk,
                        height: height.0,
                        is_coinbase: out.features.is_coinbase(),
                    },
                )
            })
            .collect();

        self.apply_delta_checked(&spent, &created, canonical_after)
    }

    /// Internal transition primitive.
    ///
    /// Kept separate from BlockBody decoding so the state machine itself can
    /// be stress-tested with arbitrary deterministic deltas.
    fn apply_delta_checked(
        &mut self,
        spent: &[[u8; 32]],
        created: &[([u8; 32], UtxoEntry)],
        canonical_after: &UtxoSet,
    ) -> Result<ShadowTransitionReport, ShadowStateError> {
        let previous_root = self.root();

        // Transactional delta journal.
        //
        // Only UTXOs touched by this transition are recorded.
        // The complete authenticated tree is no longer cloned.
        let mut undo = Vec::<ShadowUndo>::with_capacity(spent.len() + created.len());

        for commitment in spent {
            let Some(previous) = self.remove(commitment) else {
                rollback_shadow(self, &mut undo);

                return Err(ShadowStateError::MissingSpentUtxo {
                    commitment: *commitment,
                });
            };

            undo.push(ShadowUndo::Restore {
                commitment: *commitment,
                entry: previous,
            });
        }

        for (commitment, entry) in created {
            if self.contains_key(commitment) {
                rollback_shadow(self, &mut undo);

                return Err(ShadowStateError::DuplicateCreatedUtxo {
                    commitment: *commitment,
                });
            }

            self.insert(*commitment, entry.clone());

            undo.push(ShadowUndo::Remove {
                commitment: *commitment,
            });
        }

        // Development shadow invariant:
        // the complete authenticated leaf map must match
        // Nightfall's already-accepted canonical UTXO state.
        if self.entries != canonical_after.entries {
            let error = ShadowStateError::CanonicalStateMismatch {
                shadow_len: self.entries.len(),
                canonical_len: canonical_after.entries.len(),
                first_mismatch: first_mismatch_key(&self.entries, &canonical_after.entries),
            };

            rollback_shadow(self, &mut undo);

            return Err(error);
        }

        // Independent reconstruction oracle.
        //
        // This remains deliberately expensive during
        // development. Before production integration it
        // will become a periodic deep audit rather than
        // a per-block hot-path operation.
        let rebuilt = AuthenticatedUtxoTree::from_utxo_set(canonical_after);

        let incremental_root = self.root();

        let rebuilt_root = rebuilt.root();

        if incremental_root != rebuilt_root {
            let error = ShadowStateError::RebuildRootMismatch {
                incremental: incremental_root,
                rebuilt: rebuilt_root,
            };

            rollback_shadow(self, &mut undo);

            return Err(error);
        }

        Ok(ShadowTransitionReport {
            previous_root,
            new_root: incremental_root,
            spent: spent.len(),
            created: created.len(),
            resulting_utxos: self.len(),
        })
    }
}

#[derive(Clone, Debug)]
enum ShadowUndo {
    Restore {
        commitment: [u8; 32],
        entry: UtxoEntry,
    },
    Remove {
        commitment: [u8; 32],
    },
}

/// Restore a partially mutated authenticated shadow state.
///
/// Operations are reversed in strict LIFO order.
/// Because `insert` and `remove` deterministically rebuild
/// every affected Merkle path, rollback restores both the
/// logical UTXO map and the authenticated node map.
fn rollback_shadow(tree: &mut AuthenticatedUtxoTree, undo: &mut Vec<ShadowUndo>) {
    while let Some(operation) = undo.pop() {
        match operation {
            ShadowUndo::Restore { commitment, entry } => {
                let replaced = tree.insert(commitment, entry);

                debug_assert!(
                    replaced.is_none(),
                    "shadow rollback unexpectedly replaced a UTXO"
                );
            }

            ShadowUndo::Remove { commitment } => {
                let removed = tree.remove(&commitment);

                debug_assert!(removed.is_some(), "shadow rollback lost a created UTXO");
            }
        }
    }
}

fn first_mismatch_key(
    left: &BTreeMap<[u8; 32], UtxoEntry>,
    right: &BTreeMap<[u8; 32], UtxoEntry>,
) -> Option<[u8; 32]> {
    let mut keys = std::collections::BTreeSet::new();

    keys.extend(left.keys().copied());
    keys.extend(right.keys().copied());

    keys.into_iter().find(|key| left.get(key) != right.get(key))
}

fn hex32(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut out = String::with_capacity(64);

    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }

    out
}

#[cfg(test)]
mod shadow_transition_tests {
    use super::*;

    fn e(marker: u8, height: u64, coinbase: bool) -> UtxoEntry {
        UtxoEntry {
            output_pk: [marker; 32],
            height,
            is_coinbase: coinbase,
        }
    }

    fn canonical(entries: Vec<([u8; 32], UtxoEntry)>) -> UtxoSet {
        let mut set = UtxoSet::new();

        // Tests in this module care about the canonical leaf map.
        // We deliberately do not depend on commitment decompression here.
        for (key, entry) in entries {
            set.entries.insert(key, entry);
        }

        set
    }

    #[test]
    fn shadow_delta_matches_clean_rebuild() {
        let k1 = [0x10; 32];
        let k2 = [0x20; 32];
        let k3 = [0x30; 32];

        let before = canonical(vec![(k1, e(1, 10, false)), (k2, e(2, 20, true))]);

        let mut shadow = AuthenticatedUtxoTree::from_utxo_set(&before);

        let after = canonical(vec![(k2, e(2, 20, true)), (k3, e(3, 30, false))]);

        let created = vec![(k3, e(3, 30, false))];

        let report = shadow.apply_delta_checked(&[k1], &created, &after).unwrap();

        assert_eq!(report.spent, 1);
        assert_eq!(report.created, 1);
        assert_eq!(report.resulting_utxos, 2);

        assert!(shadow.matches_utxo_set(&after));

        let rebuilt = AuthenticatedUtxoTree::from_utxo_set(&after);

        assert_eq!(shadow.root(), rebuilt.root());
    }

    #[test]
    fn mismatch_is_fully_atomic() {
        let k1 = [0x41; 32];
        let k2 = [0x42; 32];

        let before = canonical(vec![(k1, e(1, 100, false))]);

        let mut shadow = AuthenticatedUtxoTree::from_utxo_set(&before);

        let root_before = shadow.root();
        let entries_before = shadow.entries.clone();

        // Intentionally wrong canonical post-state:
        // the delta creates k2, but canonical state claims k1 survived.
        let wrong_after = canonical(vec![(k1, e(1, 100, false))]);

        let err = shadow
            .apply_delta_checked(&[k1], &[(k2, e(2, 101, false))], &wrong_after)
            .unwrap_err();

        assert!(matches!(
            err,
            ShadowStateError::CanonicalStateMismatch { .. }
        ));

        // Failed shadow transition must have ZERO side effects.
        assert_eq!(shadow.root(), root_before);

        assert_eq!(shadow.entries, entries_before);
    }

    #[test]
    fn missing_spend_is_detected_and_atomic() {
        let k1 = [0x51; 32];
        let nonexistent = [0xff; 32];

        let before = canonical(vec![(k1, e(1, 1, false))]);

        let mut shadow = AuthenticatedUtxoTree::from_utxo_set(&before);

        let root_before = shadow.root();

        let err = shadow
            .apply_delta_checked(&[nonexistent], &[], &before)
            .unwrap_err();

        assert_eq!(
            err,
            ShadowStateError::MissingSpentUtxo {
                commitment: nonexistent,
            }
        );

        assert_eq!(shadow.root(), root_before);
    }

    #[test]
    fn duplicate_creation_is_detected_and_atomic() {
        let key = [0x61; 32];

        let before = canonical(vec![(key, e(1, 1, false))]);

        let mut shadow = AuthenticatedUtxoTree::from_utxo_set(&before);

        let root_before = shadow.root();

        let err = shadow
            .apply_delta_checked(&[], &[(key, e(9, 9, true))], &before)
            .unwrap_err();

        assert_eq!(
            err,
            ShadowStateError::DuplicateCreatedUtxo { commitment: key }
        );

        assert_eq!(shadow.root(), root_before);
    }

    #[test]
    fn coinbase_flag_changes_shadow_root() {
        let key = [0x71; 32];

        let normal = canonical(vec![(key, e(7, 700, false))]);

        let coinbase = canonical(vec![(key, e(7, 700, true))]);

        let a = AuthenticatedUtxoTree::from_utxo_set(&normal);

        let b = AuthenticatedUtxoTree::from_utxo_set(&coinbase);

        assert_ne!(a.root(), b.root());
    }

    #[test]
    fn height_changes_shadow_root() {
        let key = [0x81; 32];

        let a_state = canonical(vec![(key, e(8, 800, false))]);

        let b_state = canonical(vec![(key, e(8, 801, false))]);

        let a = AuthenticatedUtxoTree::from_utxo_set(&a_state);

        let b = AuthenticatedUtxoTree::from_utxo_set(&b_state);

        assert_ne!(a.root(), b.root());
    }

    #[test]
    fn deterministic_transition_sequence_matches_rebuild() {
        let mut canonical = UtxoSet::new();
        let mut shadow = AuthenticatedUtxoTree::new();

        // Keep this bounded: apply_delta_checked already performs an
        // independent full rebuild after every transition.
        //
        // 64 inserts + 22 removals exercise different key prefixes,
        // coinbase metadata, deletion and deterministic rebuilding
        // without turning a unit test into a multi-hour benchmark.
        for n in 0u16..64 {
            let mut key = [0u8; 32];

            key[0..2].copy_from_slice(&n.to_be_bytes());

            key[31] = (n as u8).wrapping_mul(37);

            let entry = e(n as u8, n as u64, n % 17 == 0);

            canonical.entries.insert(key, entry.clone());

            shadow
                .apply_delta_checked(&[], &[(key, entry)], &canonical)
                .unwrap();

            assert!(shadow.matches_utxo_set(&canonical));
        }

        for n in (0u16..64).step_by(3) {
            let mut key = [0u8; 32];

            key[0..2].copy_from_slice(&n.to_be_bytes());

            key[31] = (n as u8).wrapping_mul(37);

            canonical.entries.remove(&key);

            shadow.apply_delta_checked(&[key], &[], &canonical).unwrap();

            assert!(shadow.matches_utxo_set(&canonical));
        }

        let rebuilt = AuthenticatedUtxoTree::from_utxo_set(&canonical);

        assert_eq!(shadow.root(), rebuilt.root());

        assert_eq!(shadow.len(), canonical.entries.len());
    }
}

#[cfg(test)]
mod shadow_undo_tests {
    use super::*;

    fn entry(marker: u8, height: u64, coinbase: bool) -> UtxoEntry {
        UtxoEntry {
            output_pk: [marker; 32],
            height,
            is_coinbase: coinbase,
        }
    }

    #[test]
    fn rollback_after_multiple_mutations_restores_exact_tree() {
        let k1 = [0x91; 32];
        let k2 = [0x92; 32];
        let k3 = [0x93; 32];
        let k4 = [0x94; 32];

        let mut canonical_before = UtxoSet::new();

        canonical_before.entries.insert(k1, entry(1, 100, false));

        canonical_before.entries.insert(k2, entry(2, 101, true));

        canonical_before.entries.insert(k3, entry(3, 102, false));

        let mut shadow = AuthenticatedUtxoTree::from_utxo_set(&canonical_before);

        let root_before = shadow.root();
        let entries_before = shadow.entries.clone();
        let nodes_before = shadow.nodes.clone();

        // Deliberately incorrect canonical post-state.
        //
        // The transition itself performs several valid mutations
        // before the final canonical comparison fails.
        let wrong_after = canonical_before.clone();

        let result =
            shadow.apply_delta_checked(&[k1, k2], &[(k4, entry(4, 103, false))], &wrong_after);

        assert!(matches!(
            result,
            Err(ShadowStateError::CanonicalStateMismatch { .. })
        ));

        // Strong rollback requirement:
        // not merely logical equality, but identical leaf map,
        // identical sparse-node map and identical root.
        assert_eq!(shadow.entries, entries_before);

        assert_eq!(shadow.nodes, nodes_before);

        assert_eq!(shadow.root(), root_before);
    }

    #[test]
    fn rollback_after_duplicate_creation_restores_prior_spends() {
        let spent = [0xa1; 32];
        let duplicate = [0xa2; 32];

        let mut canonical = UtxoSet::new();

        canonical.entries.insert(spent, entry(1, 1, false));

        canonical.entries.insert(duplicate, entry(2, 2, false));

        let mut shadow = AuthenticatedUtxoTree::from_utxo_set(&canonical);

        let root_before = shadow.root();
        let entries_before = shadow.entries.clone();
        let nodes_before = shadow.nodes.clone();

        // The spend succeeds first. Creation then collides.
        // The successful spend must be rolled back.
        let result =
            shadow.apply_delta_checked(&[spent], &[(duplicate, entry(9, 9, true))], &canonical);

        assert_eq!(
            result,
            Err(ShadowStateError::DuplicateCreatedUtxo {
                commitment: duplicate,
            })
        );

        assert_eq!(shadow.entries, entries_before);

        assert_eq!(shadow.nodes, nodes_before);

        assert_eq!(shadow.root(), root_before);
    }
}
