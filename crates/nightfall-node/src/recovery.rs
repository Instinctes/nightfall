//! Offline recovery escape hatch for a node stuck on a dead fork.
//!
//! When a node ends up permanently stalled on a branch it can never leave —
//! the failure behind issue #40: hours at one height with no recovery path —
//! an operator needs a way to reset the local chain and resync from the
//! network. On the desktop, Core offers "Resync chain, keep wallet" from its
//! dashboard. A headless `nightfalld` (a systemd service, a node on WSL2, a
//! mining rig) has no dashboard, and the only remaining move is to delete
//! chain files by hand — undocumented and easy to get wrong.
//!
//! This command is that move, made safe and explicit:
//!
//! * It backs up the current chain files into a freshly created, uniquely
//!   named directory before replacing anything, and never deletes a backup.
//! * It stages and verifies a fresh genesis before it moves a single original
//!   file, so a failure leaves the originals recoverable rather than a
//!   half-reset datadir reported as success.
//! * It never touches wallet keys, the miner seed, the install id or an
//!   earlier backup, and it preserves the datadir's storage format.
//! * It is offline only. `main()` already holds the one-writer data-directory
//!   lock, which the wallet takes too, so a running node or wallet makes the
//!   command refuse rather than race a live writer. There is no recovery RPC
//!   and no automatic mining.
//!
//! It is a manual escape hatch, not a promise that every liveness problem is
//! solved: it resets to genesis and the operator resyncs. The existing
//! "don't mine while a fresh peer is ahead" guard keeps catch-up safe when the
//! node is started again.

use anyhow::Context;
use nightfall_consensus::Chain;
use nightfall_storage::{codec, ChainStore};
use nightfall_types::NetworkId;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Chain-state files recovery backs up and resets. Everything else in the
/// datadir — the wallet directory, `miner.seed`, `install-id`, the lock file
/// and any earlier backup directory — is left untouched.
const CHAIN_FILES: &[&str] = &[
    codec::BLOCKS_BIN,   // "blocks.bin"
    codec::BLOCKS_JSONL, // "blocks.jsonl"
    "headers.jsonl",
    "utxo-horizon.json",
    "chain-meta.json",
    "chain.json",              // legacy v4 chain
    "blocks.jsonl.pre-binary", // kept by `migrate-storage`
];

const BACKUP_PREFIX: &str = "recovery-backup-";
const STAGING_PREFIX: &str = ".recovery-staging-";

/// Inputs for [`recover_stalled_fork`].
#[derive(Clone, Debug)]
pub struct RecoverOpts {
    pub datadir: PathBuf,
    pub network: NetworkId,
    pub force: bool,
}

/// What a successful recovery did, for the operator-facing summary.
#[derive(Clone, Debug)]
pub struct RecoverReport {
    pub backup_dir: PathBuf,
    pub format: codec::Format,
    pub previous_height: u64,
    pub previous_tip: Option<String>,
    pub genesis_hash: String,
    pub files_backed_up: Vec<String>,
}

/// Back up a stalled-fork chain and reset the datadir to a fresh genesis,
/// keeping the wallet. See the module docs for the safety boundary.
///
/// The caller must already hold the data-directory lock (`main()` does, for
/// every command). This function is the sole writer while it runs.
pub fn recover_stalled_fork(opts: &RecoverOpts) -> anyhow::Result<RecoverReport> {
    run(opts, Fault::None)
}

/// Test-only fault points, so the recoverable-failure path can be exercised
/// without a flaky filesystem trick. Production always passes `Fault::None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fault {
    None,
    /// Fail after the originals are in the backup, before any replacement lands.
    AfterBackup,
    /// Fail partway through installing the staged replacement.
    DuringInstall,
}

fn run(opts: &RecoverOpts, fault: Fault) -> anyhow::Result<RecoverReport> {
    let datadir = opts.datadir.as_path();
    let network = opts.network;

    // (1) A destructive reset requires explicit intent.
    if !opts.force {
        anyhow::bail!(
            "recover-stalled-fork resets the chain in {} to a fresh genesis \
             (the wallet is kept). Stop the node, then re-run with --force.",
            datadir.display()
        );
    }

    // (2) The datadir must already exist and be a real directory.
    let md = fs::symlink_metadata(datadir)
        .with_context(|| format!("--datadir {} does not exist", datadir.display()))?;
    if md.file_type().is_symlink() {
        anyhow::bail!(
            "--datadir {} is a symlink; refusing to follow it",
            datadir.display()
        );
    }
    if !md.is_dir() {
        anyhow::bail!("--datadir {} is not a directory", datadir.display());
    }

    let store = ChainStore::new(datadir);
    let format = codec::detect(datadir);

    // (3) The chain files present now, each a regular, non-symlinked file.
    let present = present_chain_files(datadir)?;
    if present.is_empty() {
        anyhow::bail!(
            "no NIGHTFALLCOIN chain found in {}. recover-stalled-fork needs an \
             existing datadir with a chain; check --datadir.",
            datadir.display()
        );
    }

    // (4) Confirm the network from chain-meta.json — never guess it.
    let meta_path = datadir.join("chain-meta.json");
    if !meta_path.is_file() {
        anyhow::bail!(
            "{} holds chain files but no readable chain-meta.json, so its network \
             and state cannot be confirmed. Refusing to guess.",
            datadir.display()
        );
    }
    let stored = stored_network(&meta_path)?;
    if stored != network {
        anyhow::bail!(
            "datadir holds a {} chain but --network {} was given. Pass the \
             matching --network; refusing to reset a chain for the wrong network.",
            stored.as_str(),
            network.as_str()
        );
    }

    let (previous_height, previous_tip) = store
        .peek_meta()
        .map(|(h, tip, _)| (h, Some(tip)))
        .unwrap_or((0, None));

    // (5) A freshly created, uniquely named backup directory. `create_dir`
    //     (not create_dir_all) fails if the name is taken, so a repeated run
    //     never reuses or overwrites an earlier backup.
    let backup_dir = unique_dir(datadir, BACKUP_PREFIX)?;

    // (6) Stage and verify the replacement BEFORE moving any original. If this
    //     fails the originals have not been touched.
    let staging = unique_dir(datadir, STAGING_PREFIX)?;
    let genesis_hash = match build_and_verify_fresh(&staging, datadir, network, format) {
        Ok(h) => h,
        Err(e) => {
            let _ = fs::remove_dir_all(&staging);
            let _ = fs::remove_dir(&backup_dir);
            return Err(e.context("could not build a fresh genesis; the datadir was not changed"));
        }
    };

    // (7) Move the originals into the backup. From here the operation is
    //     destructive, but everything moved is recoverable from `backup_dir`.
    for name in &present {
        fs::rename(datadir.join(name), backup_dir.join(name)).with_context(|| {
            format!(
                "failed moving {name} into the backup at {}. The originals moved so \
                 far are there; restore them to recover.",
                backup_dir.display()
            )
        })?;
    }

    if fault == Fault::AfterBackup {
        anyhow::bail!(
            "injected fault after backup; the originals are in {}.",
            backup_dir.display()
        );
    }

    // (8) Install the staged, already-verified replacement.
    let staged = present_chain_files(&staging)?;
    for name in &staged {
        if fault == Fault::DuringInstall && *name == "chain-meta.json" {
            anyhow::bail!(
                "injected fault during install; the datadir is incomplete and the \
                 originals are in {}. Restore them to recover.",
                backup_dir.display()
            );
        }
        fs::rename(staging.join(name), datadir.join(name)).with_context(|| {
            format!(
                "failed installing {name}; the datadir is incomplete and the originals \
                 are in {}. Restore them to recover.",
                backup_dir.display()
            )
        })?;
    }
    let _ = fs::remove_dir_all(&staging);

    // (9) The datadir must now load as a clean fresh genesis of this network,
    //     in the original storage format. Anything else is a failed reset, not
    //     a success — the originals are still in the backup.
    let after = store.load_or_new(network).with_context(|| {
        format!(
            "the reset datadir did not load; the originals are in {}. Restore them to recover.",
            backup_dir.display()
        )
    })?;
    if after.network != network
        || after.block_count() != 0
        || after.genesis_hash.to_hex() != genesis_hash
    {
        anyhow::bail!(
            "the reset datadir is not a clean fresh genesis; the originals are in {}. \
             Restore them to recover.",
            backup_dir.display()
        );
    }
    if codec::detect(datadir) != format {
        anyhow::bail!(
            "the reset changed the datadir's storage format; the originals are in {}. \
             Restore them to recover.",
            backup_dir.display()
        );
    }

    Ok(RecoverReport {
        backup_dir,
        format,
        previous_height,
        previous_tip,
        genesis_hash,
        files_backed_up: present.iter().map(|s| s.to_string()).collect(),
    })
}

/// Build a fresh genesis into `staging`, in `format`, and verify it reloads as
/// a clean genesis of `network`. Returns the genesis hash. The datadir's
/// `install-id` is copied in first so the staged metadata belongs to this
/// datadir and the reset chain is trusted on the next load instead of forcing
/// a needless re-verification.
fn build_and_verify_fresh(
    staging: &Path,
    datadir: &Path,
    network: NetworkId,
    format: codec::Format,
) -> anyhow::Result<String> {
    fs::create_dir_all(staging)?;

    let install_id = datadir.join("install-id");
    if install_id.is_file() {
        fs::copy(&install_id, staging.join("install-id"))?;
    }

    // Preserve the storage format. An empty staging dir is detected as binary;
    // placing an empty blocks.jsonl makes it JSON before the first save, which
    // `save` then overwrites with the real (genesis-empty) file.
    if format == codec::Format::Json {
        fs::write(staging.join(codec::BLOCKS_JSONL), b"")?;
    }
    if codec::detect(staging) != format {
        anyhow::bail!("staging did not adopt the {:?} storage format", format);
    }

    let fresh = Chain::new_fair(network)?;
    let staged_store = ChainStore::new(staging);
    staged_store.save(&fresh)?;

    let check = staged_store.load_or_new(network)?;
    if check.network != network || check.block_count() != 0 {
        anyhow::bail!(
            "the staged genesis is not a clean {} genesis",
            network.as_str()
        );
    }
    if codec::detect(staging) != format {
        anyhow::bail!("the staged genesis changed storage format");
    }

    Ok(fresh.genesis_hash.to_hex())
}

/// Chain files present in `dir`, each confirmed a regular, non-symlinked file.
/// A symlink or non-regular file at a chain path is refused rather than
/// followed or clobbered.
fn present_chain_files(dir: &Path) -> anyhow::Result<Vec<&'static str>> {
    let mut out = Vec::new();
    for &name in CHAIN_FILES {
        let p = dir.join(name);
        match fs::symlink_metadata(&p) {
            Ok(md) => {
                if md.file_type().is_symlink() {
                    anyhow::bail!("{} is a symlink; refusing to follow it", p.display());
                }
                if !md.is_file() {
                    anyhow::bail!("{} is not a regular file; refusing", p.display());
                }
                out.push(name);
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => {
                return Err(anyhow::Error::from(e))
                    .with_context(|| format!("checking {}", p.display()))
            }
        }
    }
    Ok(out)
}

fn stored_network(meta_path: &Path) -> anyhow::Result<NetworkId> {
    let raw = fs::read_to_string(meta_path)
        .with_context(|| format!("reading {}", meta_path.display()))?;
    let v: serde_json::Value =
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", meta_path.display()))?;
    let net = v
        .get("network")
        .and_then(|n| n.as_str())
        .ok_or_else(|| anyhow::anyhow!("{} has no network field", meta_path.display()))?;
    match net {
        "mainnet" => Ok(NetworkId::Mainnet),
        "testnet" => Ok(NetworkId::Testnet),
        "devnet" => Ok(NetworkId::Devnet),
        other => anyhow::bail!("{} names an unknown network {other:?}", meta_path.display()),
    }
}

/// A freshly created directory under `parent` whose name starts with `prefix`
/// and is unique by construction. A seconds-resolution name can collide on a
/// repeated invocation (two resets in the same second) and silently reuse an
/// earlier directory; nanoseconds plus a process-lifetime counter, together
/// with `create_dir` failing on an existing name, make reuse impossible.
fn unique_dir(parent: &Path, prefix: &str) -> anyhow::Result<PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    for _ in 0..10_000 {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!("{prefix}{nanos}-{n}"));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(anyhow::Error::from(e))
                    .with_context(|| format!("creating {}", path.display()))
            }
        }
    }
    anyhow::bail!(
        "could not create a unique directory under {}",
        parent.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use nightfall_crypto::WalletKeys;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_tmp(tag: &str) -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "nf-recovery-{tag}-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            N.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    /// Build a datadir holding a `blocks`-long chain of `network` in `format`.
    /// Returns (tip_hex, block_count).
    fn seed_chain(
        dir: &Path,
        network: NetworkId,
        format: codec::Format,
        blocks: u64,
    ) -> (String, u64) {
        if format == codec::Format::Json {
            fs::write(dir.join(codec::BLOCKS_JSONL), b"").unwrap();
        }
        let store = ChainStore::new(dir);
        let mut chain = Chain::new_fair(network).unwrap();
        let miner = WalletKeys::generate().address();
        // Timestamps must be in the recent past: a reload revalidates every
        // block against the wall clock and rejects one "too far in the future".
        let base = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        for i in 0..blocks {
            chain
                .mine_block(&miner, vec![], base - (blocks - i) * 60)
                .unwrap();
        }
        store.save(&chain).unwrap();
        assert_eq!(codec::detect(dir), format, "seed produced the wrong format");
        (chain.tip_hash().to_hex(), chain.block_count())
    }

    fn opts(dir: &Path, network: NetworkId, force: bool) -> RecoverOpts {
        RecoverOpts {
            datadir: dir.to_path_buf(),
            network,
            force,
        }
    }

    fn backup_dirs(datadir: &Path) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = fs::read_dir(datadir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.is_dir()
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with(BACKUP_PREFIX))
                        .unwrap_or(false)
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn binary_format_is_preserved_and_reset_to_genesis() {
        let dir = unique_tmp("bin");
        let (tip, count) = seed_chain(&dir, NetworkId::Devnet, codec::Format::Binary, 3);
        assert_eq!(count, 3);

        let report = recover_stalled_fork(&opts(&dir, NetworkId::Devnet, true)).unwrap();

        assert_eq!(report.format, codec::Format::Binary);
        assert_eq!(report.previous_tip.as_deref(), Some(tip.as_str()));
        // datadir is a clean genesis again, still binary.
        assert_eq!(codec::detect(&dir), codec::Format::Binary);
        assert!(dir.join("blocks.bin").is_file());
        assert!(!dir.join("blocks.jsonl").exists());
        let chain = ChainStore::new(&dir)
            .load_or_new(NetworkId::Devnet)
            .unwrap();
        assert_eq!(chain.block_count(), 0);
        // backup holds the originals.
        assert!(report.backup_dir.join("blocks.bin").is_file());
        assert!(report.backup_dir.join("chain-meta.json").is_file());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn json_format_is_preserved() {
        let dir = unique_tmp("json");
        seed_chain(&dir, NetworkId::Devnet, codec::Format::Json, 2);

        let report = recover_stalled_fork(&opts(&dir, NetworkId::Devnet, true)).unwrap();

        assert_eq!(report.format, codec::Format::Json);
        assert_eq!(codec::detect(&dir), codec::Format::Json);
        assert!(dir.join("blocks.jsonl").is_file());
        assert!(!dir.join("blocks.bin").exists());
        assert!(report.backup_dir.join("blocks.jsonl").is_file());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn wallet_and_miner_seed_are_byte_identical() {
        let dir = unique_tmp("wallet");
        seed_chain(&dir, NetworkId::Devnet, codec::Format::Binary, 2);

        // Secrets recovery must never touch.
        let wallet = nightfall_storage::wallet_dir(&dir);
        fs::create_dir_all(&wallet).unwrap();
        fs::write(wallet.join("wallet.json"), b"private-wallet-state").unwrap();
        fs::write(dir.join("miner.seed"), b"deadbeefdeadbeefdeadbeefdeadbeef").unwrap();
        let wallet_before = fs::read(wallet.join("wallet.json")).unwrap();
        let seed_before = fs::read(dir.join("miner.seed")).unwrap();
        let id_before = fs::read(dir.join("install-id")).unwrap();

        recover_stalled_fork(&opts(&dir, NetworkId::Devnet, true)).unwrap();

        assert_eq!(fs::read(wallet.join("wallet.json")).unwrap(), wallet_before);
        assert_eq!(fs::read(dir.join("miner.seed")).unwrap(), seed_before);
        assert_eq!(fs::read(dir.join("install-id")).unwrap(), id_before);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn repeated_invocation_makes_distinct_backups() {
        let dir = unique_tmp("repeat");
        seed_chain(&dir, NetworkId::Devnet, codec::Format::Binary, 2);

        let r1 = recover_stalled_fork(&opts(&dir, NetworkId::Devnet, true)).unwrap();
        // A second reset of the now-fresh genesis must not reuse the first backup.
        let r2 = recover_stalled_fork(&opts(&dir, NetworkId::Devnet, true)).unwrap();

        assert_ne!(r1.backup_dir, r2.backup_dir, "backups collided");
        let dirs = backup_dirs(&dir);
        assert_eq!(dirs.len(), 2, "expected two distinct backups, got {dirs:?}");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn wrong_network_is_refused_and_nothing_moves() {
        let dir = unique_tmp("wrongnet");
        seed_chain(&dir, NetworkId::Devnet, codec::Format::Binary, 2);

        let err = recover_stalled_fork(&opts(&dir, NetworkId::Testnet, true)).unwrap_err();
        assert!(
            err.to_string().contains("devnet") && err.to_string().contains("testnet"),
            "{err}"
        );
        // Nothing was backed up or reset.
        assert!(backup_dirs(&dir).is_empty());
        assert!(dir.join("chain-meta.json").is_file());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pruned_metadata_and_horizon_are_backed_up() {
        let dir = unique_tmp("pruned");
        seed_chain(&dir, NetworkId::Devnet, codec::Format::Binary, 2);
        // A pruned node also carries headers and a UTXO horizon; mark the meta pruned.
        fs::write(dir.join("headers.jsonl"), b"{}\n").unwrap();
        fs::write(dir.join("utxo-horizon.json"), b"{\"horizon\":true}").unwrap();
        let meta = fs::read_to_string(dir.join("chain-meta.json")).unwrap();
        let bumped = meta
            .replace("\"pruned\": false", "\"pruned\": true")
            .replace("\"first_height\": 0", "\"first_height\": 1");
        fs::write(dir.join("chain-meta.json"), bumped).unwrap();

        let report = recover_stalled_fork(&opts(&dir, NetworkId::Devnet, true)).unwrap();

        assert!(report.backup_dir.join("headers.jsonl").is_file());
        assert!(report.backup_dir.join("utxo-horizon.json").is_file());
        assert!(report.backup_dir.join("chain-meta.json").is_file());
        // The reset datadir is an unpruned fresh genesis: no horizon file.
        assert!(!dir.join("utxo-horizon.json").exists());
        assert!(report
            .files_backed_up
            .iter()
            .any(|f| f == "utxo-horizon.json"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_chain_is_refused() {
        let dir = unique_tmp("empty");
        let err = recover_stalled_fork(&opts(&dir, NetworkId::Devnet, true)).unwrap_err();
        assert!(err.to_string().contains("no NIGHTFALLCOIN chain"), "{err}");
        assert!(backup_dirs(&dir).is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn force_is_required() {
        let dir = unique_tmp("noforce");
        seed_chain(&dir, NetworkId::Devnet, codec::Format::Binary, 1);
        let err = recover_stalled_fork(&opts(&dir, NetworkId::Devnet, false)).unwrap_err();
        assert!(err.to_string().contains("--force"), "{err}");
        assert!(backup_dirs(&dir).is_empty());
        assert!(dir.join("chain-meta.json").is_file());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn meta_without_a_chain_file_is_refused_not_guessed() {
        let dir = unique_tmp("metaonly");
        seed_chain(&dir, NetworkId::Devnet, codec::Format::Binary, 1);
        // Remove chain-meta.json but keep blocks: the network can no longer be
        // confirmed, so recovery must refuse rather than guess.
        fs::remove_file(dir.join("chain-meta.json")).unwrap();
        let err = recover_stalled_fork(&opts(&dir, NetworkId::Devnet, true)).unwrap_err();
        assert!(err.to_string().contains("Refusing to guess"), "{err}");
        assert!(backup_dirs(&dir).is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_chain_file_is_refused() {
        let dir = unique_tmp("symlink");
        seed_chain(&dir, NetworkId::Devnet, codec::Format::Binary, 1);
        // Replace blocks.bin with a symlink pointing outside the datadir.
        let outside = unique_tmp("outside").join("elsewhere.bin");
        fs::write(&outside, b"not ours").unwrap();
        fs::remove_file(dir.join("blocks.bin")).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("blocks.bin")).unwrap();

        let err = recover_stalled_fork(&opts(&dir, NetworkId::Devnet, true)).unwrap_err();
        assert!(err.to_string().contains("symlink"), "{err}");
        assert!(backup_dirs(&dir).is_empty());
        // The symlink target was not touched.
        assert_eq!(fs::read(&outside).unwrap(), b"not ours");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fault_after_backup_preserves_the_originals() {
        let dir = unique_tmp("fault1");
        let (tip, _) = seed_chain(&dir, NetworkId::Devnet, codec::Format::Binary, 2);
        let blocks_before = fs::read(dir.join("blocks.bin")).unwrap();

        let err = run(&opts(&dir, NetworkId::Devnet, true), Fault::AfterBackup).unwrap_err();
        assert!(err.to_string().contains("injected fault"), "{err}");

        // Exactly one backup, and it holds the untouched originals.
        let dirs = backup_dirs(&dir);
        assert_eq!(dirs.len(), 1);
        assert_eq!(fs::read(dirs[0].join("blocks.bin")).unwrap(), blocks_before);
        // No success was reported, and the backup restores the original tip.
        let restored = unique_tmp("restore1");
        for f in ["blocks.bin", "chain-meta.json"] {
            fs::copy(dirs[0].join(f), restored.join(f)).unwrap();
        }
        let chain = ChainStore::new(&restored)
            .load_or_new(NetworkId::Devnet)
            .unwrap();
        assert_eq!(chain.tip_hash().to_hex(), tip);

        fs::remove_dir_all(&dir).ok();
        fs::remove_dir_all(&restored).ok();
    }

    #[test]
    fn fault_during_install_preserves_the_originals() {
        let dir = unique_tmp("fault2");
        let (tip, _) = seed_chain(&dir, NetworkId::Devnet, codec::Format::Binary, 2);

        let err = run(&opts(&dir, NetworkId::Devnet, true), Fault::DuringInstall).unwrap_err();
        assert!(err.to_string().contains("injected fault"), "{err}");

        let dirs = backup_dirs(&dir);
        assert_eq!(dirs.len(), 1);
        // Restore from the backup and confirm the original tip comes back.
        let restored = unique_tmp("restore2");
        for f in ["blocks.bin", "chain-meta.json"] {
            if dirs[0].join(f).is_file() {
                fs::copy(dirs[0].join(f), restored.join(f)).unwrap();
            }
        }
        let chain = ChainStore::new(&restored)
            .load_or_new(NetworkId::Devnet)
            .unwrap();
        assert_eq!(chain.tip_hash().to_hex(), tip);

        fs::remove_dir_all(&dir).ok();
        fs::remove_dir_all(&restored).ok();
    }

    #[test]
    fn the_backup_restores_the_original_tip() {
        let dir = unique_tmp("restoretip");
        let (tip, count) = seed_chain(&dir, NetworkId::Devnet, codec::Format::Binary, 4);
        assert_eq!(count, 4);

        let report = recover_stalled_fork(&opts(&dir, NetworkId::Devnet, true)).unwrap();

        // The reset datadir is genesis...
        let fresh = ChainStore::new(&dir)
            .load_or_new(NetworkId::Devnet)
            .unwrap();
        assert_eq!(fresh.block_count(), 0);

        // ...and loading the backup directory reproduces the original tip exactly.
        let backed = ChainStore::new(&report.backup_dir)
            .load_or_new(NetworkId::Devnet)
            .unwrap();
        assert_eq!(backed.tip_hash().to_hex(), tip);
        assert_eq!(backed.block_count(), 4);

        fs::remove_dir_all(&dir).ok();
    }
}
