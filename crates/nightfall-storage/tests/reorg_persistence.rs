//! Persistence must survive a reorg.
//!
//! Regression test for a bug found by running two mining nodes against each
//! other: after adopting a competing chain of equal or greater length, `save()`
//! took the append path and spliced the new fork onto the abandoned one. The
//! resulting file could not be replayed, so the node refused to start with
//! "previous hash does not link to our tip" — a wallet that had mined for hours
//! would simply never come back up.

use nightfall_consensus::Chain;
use nightfall_crypto::WalletKeys;
use nightfall_storage::ChainStore;
use nightfall_types::NetworkId;

const NOW: u64 = 1_800_000_000;

fn tmpdir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "nf-store-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn mine_chain(blocks: u64, seed: [u8; 32]) -> Chain {
    let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();
    let miner = WalletKeys::from_seed(seed).address();
    for i in 0..blocks {
        chain.mine_block(&miner, vec![], NOW + i * 15).unwrap();
    }
    chain
}

#[test]
fn reorg_to_a_longer_chain_is_stored_correctly() {
    let dir = tmpdir("reorg");
    let store = ChainStore::new(&dir);

    // Node mines its own short chain and persists it.
    let ours = mine_chain(3, [1u8; 32]);
    store.save(&ours).unwrap();

    // A competing, heavier chain arrives and is adopted.
    let theirs = mine_chain(5, [2u8; 32]);
    let mut adopted = Chain::new_fair(NetworkId::Devnet).unwrap();
    adopted
        .try_ingest_blocks(theirs.blocks.clone(), NOW + 10_000)
        .unwrap();
    assert_ne!(adopted.tip_hash(), ours.tip_hash(), "setup: chains differ");
    assert!(adopted.block_count() > ours.block_count());

    store.save(&adopted).unwrap();

    // The decisive check: reload from disk.
    let reloaded = store
        .load_or_new(NetworkId::Devnet)
        .expect("a chain saved after a reorg must reload");

    assert_eq!(reloaded.tip_hash(), adopted.tip_hash());
    assert_eq!(reloaded.block_count(), adopted.block_count());
    reloaded.verify_supply().unwrap();

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn reorg_to_an_equal_length_chain_is_stored_correctly() {
    // The nastiest case: same height, different history, so a naive
    // "did the chain shrink?" check sees nothing wrong.
    let dir = tmpdir("reorg-equal");
    let store = ChainStore::new(&dir);

    let ours = mine_chain(4, [3u8; 32]);
    store.save(&ours).unwrap();

    let theirs = mine_chain(4, [4u8; 32]);
    let mut adopted = Chain::new_fair(NetworkId::Devnet).unwrap();
    adopted
        .try_ingest_blocks(theirs.blocks.clone(), NOW + 10_000)
        .unwrap();
    assert_ne!(adopted.tip_hash(), ours.tip_hash());
    assert_eq!(adopted.block_count(), ours.block_count());

    store.save(&adopted).unwrap();

    let reloaded = store.load_or_new(NetworkId::Devnet).unwrap();
    assert_eq!(reloaded.tip_hash(), adopted.tip_hash());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn plain_extension_still_appends() {
    // The append fast path must keep working — rewriting the whole file on
    // every block is what made v4 unusable.
    let dir = tmpdir("extend");
    let store = ChainStore::new(&dir);

    let mut chain = mine_chain(2, [5u8; 32]);
    store.save(&chain).unwrap();
    let miner = WalletKeys::from_seed([5u8; 32]).address();
    chain.mine_block(&miner, vec![], NOW + 100).unwrap();
    store.save(&chain).unwrap();

    let reloaded = store.load_or_new(NetworkId::Devnet).unwrap();
    assert_eq!(reloaded.block_count(), 3);
    assert_eq!(reloaded.tip_hash(), chain.tip_hash());

    std::fs::remove_dir_all(&dir).ok();
}
