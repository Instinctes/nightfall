//! Genuine offline Mainnet acceptance.
//! Requires an operator-supplied archive, never synthetic proof.

use nightfall_storage::{now_unix, ChainStore};
use nightfall_types::{checkpoint_at, highest_checkpoint_height, NetworkId};
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "requires complete offline mainnet archive"]
fn pr100_real_archive_export_import_catchup_equals_full_archive() {
    let source_dir = PathBuf::from(
        std::env::var("NF_PR100_ARCHIVE_COPY").expect("offline archive copy required"),
    );

    assert!(source_dir.is_dir());

    let source_store = ChainStore::new(&source_dir);
    let checkpoint = highest_checkpoint_height();

    assert!(checkpoint >= 505_000, "compiled checkpoint is too old");

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    let scratch =
        std::env::temp_dir().join(format!("nightfall-pr100-{}-{stamp}", std::process::id()));

    std::fs::create_dir(&scratch).unwrap();

    let _cleanup = Cleanup(scratch.clone());

    let snapshot_file = scratch.join("authenticated-checkpoint.json");

    // Official export API, not a synthetic replacement.
    let start = Instant::now();

    let snapshot = source_store
        .export_checkpoint_snapshot(&snapshot_file, NetworkId::Mainnet)
        .unwrap();

    let export_seconds = start.elapsed().as_secs_f64();

    assert_eq!(snapshot.checkpoint_height, checkpoint);

    drop(snapshot);

    let snapshot_bytes = std::fs::metadata(&snapshot_file).unwrap().len();

    assert!(
        snapshot_bytes > 0 && snapshot_bytes <= 512 * 1024 * 1024,
        "snapshot exceeds import size limit"
    );

    // Genuine import into independent disposable storage.
    let restored_store = ChainStore::new(scratch.join("restored"));

    let start = Instant::now();

    let mut restored = restored_store
        .import_checkpoint_snapshot(&snapshot_file, NetworkId::Mainnet)
        .unwrap();

    let import_seconds = start.elapsed().as_secs_f64();

    assert_eq!(
        restored.tip_hash().to_hex(),
        checkpoint_at(checkpoint).unwrap()
    );

    assert_eq!(restored.first_height, checkpoint + 1);

    // Independently load the complete source archive.
    let start = Instant::now();

    let source = source_store.load_or_new(NetworkId::Mainnet).unwrap();

    let load_seconds = start.elapsed().as_secs_f64();

    assert_eq!(source.first_height, 0, "source must be a complete archive");

    assert!(
        source.tip_height().unwrap().0 > checkpoint,
        "archive has no post-checkpoint blocks"
    );

    assert_eq!(
        source.hash_at(checkpoint).unwrap().to_hex(),
        checkpoint_at(checkpoint).unwrap()
    );

    // Every later block must pass ordinary consensus.
    let start = Instant::now();
    let mut catchup_blocks = 0u64;

    for block in source.blocks.iter().skip((checkpoint + 1) as usize) {
        restored.apply_block(block.clone(), now_unix()).unwrap();

        catchup_blocks += 1;
    }

    assert!(catchup_blocks > 0);

    let catchup_seconds = start.elapsed().as_secs_f64();

    restored.verify_supply().unwrap();

    restored_store.save(&restored).unwrap();

    // Verify actual persisted restart, not just RAM state.
    let again = restored_store.load_or_new(NetworkId::Mainnet).unwrap();

    again.verify_supply().unwrap();

    assert_eq!(again.tip_hash(), source.tip_hash(), "tip hash");

    assert_eq!(
        again.ledger.utxo_root(),
        source.ledger.utxo_root(),
        "UTXO root"
    );

    assert_eq!(
        again.ledger.kernel_sum(),
        source.ledger.kernel_sum(),
        "kernel sum"
    );

    assert_eq!(
        again.ledger.supply.total_minted_darks, source.ledger.supply.total_minted_darks,
        "minted supply"
    );

    assert_eq!(
        again.ledger.supply.total_burned_darks, source.ledger.supply.total_burned_darks,
        "burned supply"
    );

    assert_eq!(again.total_work, source.total_work, "total work");

    assert_eq!(again.block_count(), source.block_count(), "block count");

    assert_eq!(again.first_height, checkpoint + 1);

    let peak_kib = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("VmHWM:"))
                .map(str::to_string)
        })
        .and_then(|line| {
            line.split_whitespace()
                .nth(1)
                .and_then(|n| n.parse::<u64>().ok())
        });

    println!(
        "PR100_REAL_MAINNET_PASS \
         checkpoint={} \
         source_tip={} \
         catchup={} \
         snapshot_bytes={} \
         load_s={:.2} \
         export_s={:.2} \
         import_s={:.2} \
         catchup_s={:.2} \
         peak_ram_kib={:?}",
        checkpoint,
        source.tip_height().unwrap().0,
        catchup_blocks,
        snapshot_bytes,
        load_seconds,
        export_seconds,
        import_seconds,
        catchup_seconds,
        peak_kib,
    );
}
