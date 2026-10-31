//! Real Devnet PoW across the consensus activation.
//! No mainnet, wallet files, RPC or peers are accessed.

use nightfall_consensus::Chain;
use nightfall_crypto::WalletKeys;
use nightfall_types::NetworkId;

#[test]
fn activation_boundary_replay_and_rejection_are_consistent() {
    let network = NetworkId::Devnet;

    assert_eq!(network.binding_fork_height(), 20);
    assert_eq!(NetworkId::Mainnet.binding_fork_height(), 500_000);

    let wallet = WalletKeys::generate();
    let recipient = wallet.address();

    let mut original = Chain::new_fair(network).unwrap();

    let start = 1_800_000_000u64;
    let mut blocks = Vec::new();

    // Mine valid blocks on both sides of activation.
    for height in 0..=21u64 {
        let block = original
            .mine_block(&recipient, Vec::new(), start + height * 15)
            .unwrap();

        assert_eq!(block.header.height.0, height);

        assert_eq!(block.body.has_any_binding(), height >= 20);

        blocks.push(block);
    }

    original.verify_supply().unwrap();

    // Independently verify the entire history.
    let fresh = Chain::rebuild_from_blocks(network, blocks.clone(), start + 2_000).unwrap();

    fresh.verify_supply().unwrap();

    // Simulate replay of previously validated local data.
    let trusted = Chain::rebuild_from_blocks_trusted_prefix(
        network,
        blocks.clone(),
        blocks.len(),
        start + 2_000,
    )
    .unwrap();

    trusted.verify_supply().unwrap();

    // Both replay paths must produce identical state.
    for replayed in [&fresh, &trusted] {
        assert_eq!(replayed.tip_hash(), original.tip_hash());

        assert_eq!(replayed.chain_work(), original.chain_work());

        assert_eq!(replayed.total_minted(), original.total_minted());

        assert_eq!(replayed.ledger.utxo_root(), original.ledger.utxo_root());

        assert_eq!(replayed.ledger.kernel_sum(), original.ledger.kernel_sum());
    }

    // Construct the next valid block template.
    let next = original
        .build_template(&recipient, Vec::new(), start + 22 * 15)
        .unwrap()
        .seal(0);

    let before_tip = original.tip_hash();
    let before_work = original.chain_work();
    let before_minted = original.total_minted();
    let before_utxo = original.ledger.utxo_root();
    let before_kernel = original.ledger.kernel_sum();
    let before_count = original.block_count();

    // Attack 1: remove mandatory post-fork binding.
    let mut missing_binding = next.clone();

    missing_binding.body.strip_binding();

    missing_binding.header.body_root = missing_binding.body.hash();

    assert!(original
        .apply_block_locally_trusted(missing_binding, start + 2_000,)
        .is_err());

    // Attack 2: provide a noncanonical kernel offset.
    let mut bad_offset = next;

    bad_offset.body.kernel_offset = Some([255u8; 32]);

    bad_offset.header.body_root = bad_offset.body.hash();

    assert!(original
        .apply_block_locally_trusted(bad_offset, start + 2_000,)
        .is_err());

    // Failed validation must never advance the chain.
    assert_eq!(original.tip_hash(), before_tip);
    assert_eq!(original.chain_work(), before_work);
    assert_eq!(original.total_minted(), before_minted);

    assert_eq!(original.ledger.utxo_root(), before_utxo);

    assert_eq!(original.ledger.kernel_sum(), before_kernel);

    assert_eq!(original.block_count(), before_count);

    original.verify_supply().unwrap();
}
