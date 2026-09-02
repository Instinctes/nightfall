//! A validation record has to mean "I checked this", not "a file says so".
//!
//! The chain file carries no signature, and it does not need one: a node
//! re-derives every block hash and re-checks every proof of work on load.
//! That full check is skipped only when the datadir's own record says this
//! exact file was already verified here — otherwise starting a node would
//! mean re-verifying the whole chain every time.
//!
//! The record used to be nothing but a tip hash and a byte count. Both
//! travel with the file. So shipping `blocks.bin` next to its
//! `chain-meta.json` handed the recipient a chain that was never checked,
//! and the failure was silent: the node started, reported the height, and
//! looked entirely healthy.
//!
//! That matters most for the thing this test was written alongside — a
//! downloadable chain archive, so newcomers do not sync from genesis. An
//! archive is only safe if unpacking it cannot switch the checking off.

use nightfall_crypto::WalletKeys;
use nightfall_storage::ChainStore;
use nightfall_types::NetworkId;

/// A datadir with real blocks in it.
///
/// A genesis-only chain writes a zero-byte `blocks.bin`, and
/// `is_own_file_trusted` refuses that outright — there is nothing to have
/// verified. So every case here needs a chain that actually exists.
fn seeded(dir: &std::path::Path) -> ChainStore {
    let store = ChainStore::new(dir);
    let mut chain = store.load_or_new(NetworkId::Devnet).unwrap();
    let miner = WalletKeys::generate().address();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    for i in 0..3u64 {
        chain.mine_block(&miner, vec![], now + i * 15).unwrap();
    }
    store.save(&chain).unwrap();
    store
}

fn tmpdir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "nf-vr-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Our own datadir, untouched, still counts as verified.
///
/// Without this the fix would be "re-verify always", which is safe and
/// useless: every start of a real node would re-check the whole chain.
#[test]
fn our_own_record_is_still_trusted() {
    let dir = tmpdir("own");
    let store = seeded(&dir);

    assert!(
        store.is_own_file_trusted(),
        "a datadir this node just wrote must not need re-verifying"
    );

    // Reopening the same directory is the same installation.
    let again = ChainStore::new(&dir);
    assert!(
        again.is_own_file_trusted(),
        "the id lives in the directory, so a restart keeps it"
    );

    let _ = std::fs::remove_dir_all(dir);
}

/// The hole this closes: a record copied in from somewhere else.
///
/// This is exactly the shape of a chain archive that ships its metadata —
/// and of any "just copy my datadir" advice in a forum thread.
#[test]
fn a_record_from_another_installation_is_not_ours() {
    let source = tmpdir("source");
    let victim = tmpdir("victim");

    let src = seeded(&source);
    assert!(src.is_own_file_trusted(), "setup: the source trusts itself");

    // Copy the chain *and* its validation record, the way an archive would.
    for name in ["blocks.bin", "blocks.jsonl", "chain-meta.json"] {
        let from = source.join(name);
        if from.exists() {
            std::fs::copy(&from, victim.join(name)).unwrap();
        }
    }

    let dst = ChainStore::new(&victim);
    assert!(
        !dst.is_own_file_trusted(),
        "a validation record written by another installation must not switch \
         off proof-of-work checking here — this is how a tampered archive \
         would be swallowed whole"
    );

    let _ = std::fs::remove_dir_all(source);
    let _ = std::fs::remove_dir_all(victim);
}

/// Copying the id as well must not help.
///
/// Someone assembling an archive by hand might sweep up every file. The
/// point of the id is that it identifies *a* datadir, so two directories
/// holding the same id are the same datadir by definition — and if a
/// publisher ships theirs, they have published a directory, not a chain.
/// Nothing here can stop that; what this test pins is that the id is the
/// only thing standing in the way, so the archive script has one clear rule
/// to follow: ship `blocks.bin`, nothing else.
#[test]
fn the_id_is_the_only_thing_that_makes_a_record_ours() {
    let source = tmpdir("id-source");
    let victim = tmpdir("id-victim");

    let _src = seeded(&source);

    for name in ["blocks.bin", "blocks.jsonl", "chain-meta.json"] {
        let from = source.join(name);
        if from.exists() {
            std::fs::copy(&from, victim.join(name)).unwrap();
        }
    }
    let dst = ChainStore::new(&victim);
    assert!(!dst.is_own_file_trusted(), "without the id: refused");

    std::fs::copy(source.join("install-id"), victim.join("install-id")).unwrap();
    let dst = ChainStore::new(&victim);
    assert!(
        dst.is_own_file_trusted(),
        "with the id copied too, the record is by definition ours — which is \
         why the archive must never contain it"
    );

    let _ = std::fs::remove_dir_all(source);
    let _ = std::fs::remove_dir_all(victim);
}

/// A record with no id at all — every datadir written before this field
/// existed — must be re-verified rather than trusted.
#[test]
fn an_old_record_without_an_id_is_re_verified() {
    let dir = tmpdir("legacy");
    let store = seeded(&dir);
    assert!(store.is_own_file_trusted());

    // Strip the field, as an older release would have written it.
    let path = dir.join("chain-meta.json");
    let raw = std::fs::read_to_string(&path).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&raw).unwrap();
    json.as_object_mut().unwrap().remove("validated_by");
    std::fs::write(&path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();

    let store = ChainStore::new(&dir);
    assert!(
        !store.is_own_file_trusted(),
        "an upgrade re-verifies once. That is a slow start, not a wrong one."
    );

    let _ = std::fs::remove_dir_all(dir);
}
