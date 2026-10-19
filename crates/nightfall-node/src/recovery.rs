//! Narrow, offline-only, journaled chain reset. The caller must hold the datadir lock.
//! Do not call this from RPC, wallets, background tasks or the mining runtime.
use anyhow::{bail, Context, Result};
use nightfall_storage::{ChainStore, Format};
use nightfall_types::NetworkId;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const BIN: &str = "blocks.bin";
const JSON: &str = "blocks.jsonl";
const META: &str = "chain-meta.json";
const HEADERS: &str = "headers.jsonl";
const HORIZON: &str = "utxo-horizon.json";
const JOURNAL: &str = ".stalled-fork-recovery-in-progress";

fn file_state(p: &Path) -> Result<bool> {
    match fs::symlink_metadata(p) {
        Ok(m) if m.file_type().is_file() => Ok(true),
        Ok(_) => bail!("refusing linked/non-regular path: {}", p.display()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}
fn directory(p: &Path) -> Result<()> {
    if !fs::symlink_metadata(p)?.file_type().is_dir() {
        bail!("datadir must be a real directory: {}", p.display());
    }
    Ok(())
}
fn sync_dir(p: &Path) -> Result<()> {
    File::open(p)?.sync_all()?;
    Ok(())
}
fn sync_files(p: &Path, names: &[&str]) -> Result<()> {
    for name in names {
        File::open(p.join(name))?.sync_all()?;
    }
    sync_dir(p)
}
fn unique_dir(parent: &Path, tag: &str) -> Result<PathBuf> {
    for _ in 0..16 {
        let p = parent.join(format!("{tag}-{:016x}", rand::random::<u64>()));
        match fs::create_dir(&p) {
            Ok(()) => return Ok(p),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    bail!("could not allocate unique {tag} directory")
}
fn inject(point: &str) -> Result<()> {
    // Faults are available only in debug builds, never in release binaries.
    #[cfg(debug_assertions)]
    if std::env::var("NIGHTFALL_RECOVERY_TEST_FAILURE")
        .ok()
        .as_deref()
        == Some(point)
    {
        bail!("injected recovery test failure: {point}");
    }
    #[cfg(not(debug_assertions))]
    let _ = point;
    Ok(())
}

pub fn recover(dir: &Path, network: NetworkId) -> Result<PathBuf> {
    directory(dir)?;
    if file_state(&dir.join(JOURNAL))? {
        bail!(
            "unfinished recovery journal {} — inspect and restore backup before retrying",
            dir.join(JOURNAL).display()
        );
    }
    let binary = file_state(&dir.join(BIN))?;
    let json = file_state(&dir.join(JSON))?;
    if binary == json {
        bail!("expected exactly one chain format, not both/neither");
    }
    let block_file = if binary { BIN } else { JSON };
    if !file_state(&dir.join("install-id"))? {
        bail!("install-id missing; refusing an untrusted or ambiguous layout");
    }
    if !file_state(&dir.join(META))? {
        bail!("chain-meta.json missing; refusing ambiguous chain layout");
    }
    if file_state(&dir.join("chain.json"))? {
        bail!("legacy chain.json detected; refusing mixed protocol layout");
    }
    for name in [
        "blocks.bin.tmp",
        "blocks.jsonl.tmp",
        "chain-meta.json.tmp",
        "headers.jsonl.tmp",
        "utxo-horizon.json.tmp",
    ] {
        if file_state(&dir.join(name))? {
            bail!("unfinished persistence file {name}");
        }
    }
    let meta: serde_json::Value = serde_json::from_slice(&fs::read(dir.join(META))?)?;
    let saved_network: NetworkId = serde_json::from_value(meta["network"].clone())
        .context("metadata network missing or invalid")?;
    if saved_network != network {
        bail!("wrong network: metadata says {saved_network}, requested {network}");
    }
    let is_pruned =
        meta["pruned"].as_bool() == Some(true) || meta["first_height"].as_u64().unwrap_or(0) > 0;
    let headers = file_state(&dir.join(HEADERS))?;
    let horizon = file_state(&dir.join(HORIZON))?;
    if (headers || horizon) != is_pruned || headers != horizon {
        bail!("inconsistent pruned/header/horizon metadata; refusing to guess");
    }
    let mut names = vec![block_file, META];
    if is_pruned {
        names.extend([HEADERS, HORIZON]);
    }
    // Verify the actual chain and its genesis before any original file is moved.
    let store = ChainStore::new(dir);
    let original = store
        .load_or_new(network)
        .context("original chain failed validation")?;
    if meta["genesis_hash"].as_str() != Some(original.genesis_hash.to_hex().as_str())
        || meta["validated_tip"].as_str() != Some(original.tip_hash().to_hex().as_str())
        || original.is_pruned() != is_pruned
    {
        bail!("original chain and metadata disagree");
    }
    original.verify_supply()?;
    let expected_format = store.format();
    let staged = unique_dir(dir, ".stalled-recovery-stage")?;
    let staged_store = ChainStore::new(&staged);
    let stage_result = (|| -> Result<()> {
        fs::copy(dir.join("install-id"), staged.join("install-id"))?;
        if store.format() == Format::Json {
            fs::write(staged.join(JSON), [])?;
        }
        inject("stage-save")?;
        let fresh = nightfall_consensus::Chain::new_fair(network)?;
        staged_store.save(&fresh)?;
        inject("stage-save-after")?;
        let again = staged_store.load_or_new(network)?;
        if again.tip_hash() != fresh.tip_hash()
            || again.block_count() != fresh.block_count()
            || !file_state(&staged.join(block_file))?
            || !file_state(&staged.join(META))?
        {
            bail!("staged genesis read-back verification failed");
        }
        again.verify_supply()?;
        sync_files(&staged, &[block_file, META])?;
        Ok(())
    })();
    if let Err(e) = stage_result {
        let _ = fs::remove_dir_all(&staged);
        return Err(e.context("staging failed; original chain untouched"));
    }
    let backup = unique_dir(dir, "stalled-fork-recovery-backup")?;
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join(JOURNAL))?;
    writeln!(
        f,
        "BACKUP={}\nSTAGE={}\nFILES={}",
        backup.display(),
        staged.display(),
        names.join(",")
    )?;
    f.sync_all()?;
    sync_dir(dir)?;
    println!("recovery backup: {}", backup.display());
    let mut moved: Vec<&str> = Vec::new();
    let mut installed: Vec<&str> = Vec::new();
    let outcome = (|| -> Result<()> {
        for (idx, name) in names.iter().enumerate() {
            inject(&format!("backup-{}", idx + 1))?;
            fs::rename(dir.join(name), backup.join(name))
                .with_context(|| format!("backing up {name}"))?;
            moved.push(name);
        }
        sync_dir(&backup)?;
        sync_dir(dir)?;
        for (idx, name) in [block_file, META].iter().enumerate() {
            inject(&format!("install-{}", idx + 1))?;
            fs::rename(staged.join(name), dir.join(name))
                .with_context(|| format!("installing staged {name}"))?;
            installed.push(name);
        }
        sync_dir(dir)?;
        inject("verify")?;
        let checked = store.load_or_new(network)?;
        let fresh = nightfall_consensus::Chain::new_fair(network)?;
        if checked.tip_hash() != fresh.tip_hash()
            || checked.block_count() != fresh.block_count()
            || store.format() != expected_format
        {
            bail!("installed chain did not verify as the fresh genesis in the original format");
        }
        checked.verify_supply()?;
        Ok(())
    })();
    if let Err(e) = outcome {
        let mut rollback_errors = Vec::new();
        for name in installed.iter().rev() {
            if let Err(x) = fs::rename(dir.join(name), staged.join(name)) {
                rollback_errors.push(format!("stage {name}: {x}"));
            }
        }
        for name in moved.iter().rev() {
            if let Err(x) = fs::rename(backup.join(name), dir.join(name)) {
                rollback_errors.push(format!("restore {name}: {x}"));
            }
        }
        if rollback_errors.is_empty() {
            if let Err(x) =
                sync_dir(dir).and_then(|_| fs::remove_file(dir.join(JOURNAL)).map_err(Into::into))
            {
                rollback_errors.push(format!("journal cleanup: {x}"));
            }
        }
        if !rollback_errors.is_empty() {
            bail!("RECOVERY FAILED: {e}; ROLLBACK INCOMPLETE: {}; backup: {}; journal: {}. STOP NODE. See docs/RECOVER-STALLED-FORK.md", rollback_errors.join("; "), backup.display(), dir.join(JOURNAL).display());
        }
        let _ = fs::remove_dir_all(&staged);
        return Err(
            e.context("recovery failed, original chain restored; backup directory retained")
        );
    }
    sync_dir(&backup)?;
    fs::remove_file(dir.join(JOURNAL))?;
    sync_dir(dir)?;
    fs::remove_dir_all(&staged)?;
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nightfall_crypto::WalletKeys;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Tmp(PathBuf);
    impl Tmp {
        fn new() -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "nf-recovery-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            // A stale test directory must never be reused.
            fs::create_dir(&dir).unwrap();
            Self(dir)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn fixture(dir: &Path, fmt: Format, prune: bool) -> (String, Vec<(String, Vec<u8>)>) {
        if fmt == Format::Json {
            fs::write(dir.join(JSON), b"").unwrap();
        }
        let store = ChainStore::new(dir);
        let mut chain = nightfall_consensus::Chain::new_fair(NetworkId::Devnet).unwrap();
        let miner = WalletKeys::from_seed([16; 32]).address();
        if prune {
            for i in 0..12 {
                chain
                    .mine_block(&miner, vec![], nightfall_storage::now_unix() + i * 15)
                    .unwrap();
            }
            chain.prune_keep(4).unwrap();
        } else {
            chain
                .mine_block(&miner, vec![], nightfall_storage::now_unix())
                .unwrap();
        }
        store.save(&chain).unwrap();
        fs::write(dir.join("wallet.seed"), b"DO-NOT-CHANGE-THIS-SEED").unwrap();
        fs::create_dir(dir.join("wallet")).unwrap();
        fs::write(dir.join("wallet").join("key.dat"), b"SEPARATE-KEY-DATA").unwrap();
        let mut names = vec![store.blocks_path(), store.meta_path()];
        if prune {
            names.extend([store.headers_path(), store.horizon_path()]);
        }
        let bytes = names
            .into_iter()
            .map(|p| {
                (
                    p.file_name().unwrap().to_string_lossy().to_string(),
                    fs::read(p).unwrap(),
                )
            })
            .collect();
        (chain.tip_hash().to_hex(), bytes)
    }
    #[test]
    fn recovery_preserves_format_wallet_and_restorable_tip() {
        for fmt in [Format::Binary, Format::Json] {
            for prune in [false, true] {
                let tmp = Tmp::new();
                let (tip, original) = fixture(&tmp.0, fmt, prune);
                let backup = recover(&tmp.0, NetworkId::Devnet).unwrap();
                assert_eq!(ChainStore::new(&tmp.0).format(), fmt);
                assert!(backup.is_dir());
                for (name, contents) in &original {
                    assert_eq!(&fs::read(backup.join(name)).unwrap(), contents);
                }
                assert_eq!(
                    fs::read(tmp.0.join("wallet.seed")).unwrap(),
                    b"DO-NOT-CHANGE-THIS-SEED"
                );
                assert_eq!(
                    fs::read(tmp.0.join("wallet/key.dat")).unwrap(),
                    b"SEPARATE-KEY-DATA"
                );
                let store = ChainStore::new(&tmp.0);
                let fresh = store.load_or_new(NetworkId::Devnet).unwrap();
                assert_eq!(
                    fresh.tip_hash(),
                    nightfall_consensus::Chain::new_fair(NetworkId::Devnet)
                        .unwrap()
                        .tip_hash()
                );
                // Reinstall the saved complete original state, including pruned metadata.
                for (name, _) in &original {
                    let path = tmp.0.join(name);
                    if path.exists() {
                        fs::remove_file(path).unwrap();
                    }
                }
                for (name, _) in &original {
                    fs::copy(backup.join(name), tmp.0.join(name)).unwrap();
                }
                assert_eq!(
                    store
                        .load_or_new(NetworkId::Devnet)
                        .unwrap()
                        .tip_hash()
                        .to_hex(),
                    tip
                );
            }
        }
    }
    #[test]
    fn repeated_invocation_uses_distinct_backups() {
        let tmp = Tmp::new();
        fixture(&tmp.0, Format::Binary, false);
        let a = recover(&tmp.0, NetworkId::Devnet).unwrap();
        let b = recover(&tmp.0, NetworkId::Devnet).unwrap();
        assert_ne!(a, b);
        assert!(a.join(BIN).exists() && b.join(BIN).exists());
    }
    #[test]
    fn wrong_network_does_not_move_original_chain() {
        let tmp = Tmp::new();
        let (_, contents) = fixture(&tmp.0, Format::Json, false);
        assert!(recover(&tmp.0, NetworkId::Mainnet).is_err());
        for (name, value) in contents {
            assert_eq!(fs::read(tmp.0.join(name)).unwrap(), value);
        }
    }
    #[test]
    fn incomplete_pruned_layout_fails_closed() {
        let tmp = Tmp::new();
        fixture(&tmp.0, Format::Binary, true);
        fs::remove_file(tmp.0.join(HORIZON)).unwrap();
        assert!(recover(&tmp.0, NetworkId::Devnet).is_err());
    }
    #[test]
    fn injected_failures_never_erase_original_tip() {
        for failure in [
            "stage-save",
            "stage-save-after",
            "backup-1",
            "backup-2",
            "install-1",
            "install-2",
            "verify",
        ] {
            let tmp = Tmp::new();
            let (tip, original) = fixture(&tmp.0, Format::Binary, false);
            std::env::set_var("NIGHTFALL_RECOVERY_TEST_FAILURE", failure);
            let attempt = recover(&tmp.0, NetworkId::Devnet);
            std::env::remove_var("NIGHTFALL_RECOVERY_TEST_FAILURE");
            assert!(attempt.is_err(), "expected fault {failure}");
            for (name, contents) in &original {
                assert_eq!(
                    fs::read(tmp.0.join(name)).unwrap(),
                    *contents,
                    "fault={failure} file={name}"
                );
            }
            assert_eq!(
                ChainStore::new(&tmp.0)
                    .load_or_new(NetworkId::Devnet)
                    .unwrap()
                    .tip_hash()
                    .to_hex(),
                tip
            );
            assert_eq!(
                fs::read(tmp.0.join("wallet.seed")).unwrap(),
                b"DO-NOT-CHANGE-THIS-SEED"
            );
            assert!(!tmp.0.join(JOURNAL).exists());
        }
    }
    #[test]
    fn incomplete_prior_transaction_is_not_reused() {
        let tmp = Tmp::new();
        fixture(&tmp.0, Format::Binary, false);
        fs::write(tmp.0.join(JOURNAL), "uncommitted").unwrap();
        assert!(recover(&tmp.0, NetworkId::Devnet).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn linked_chain_is_rejected() {
        use std::os::unix::fs::symlink;
        let tmp = Tmp::new();
        fixture(&tmp.0, Format::Binary, false);
        fs::rename(tmp.0.join(BIN), tmp.0.join("original-bin")).unwrap();
        symlink(tmp.0.join("original-bin"), tmp.0.join(BIN)).unwrap();
        assert!(recover(&tmp.0, NetworkId::Devnet).is_err());
    }
}
