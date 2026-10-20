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
//! * It validates its inputs **before** taking the data-directory lock, so a
//!   bad `--datadir` is refused without being created and a symlinked lock
//!   file is never written through.
//! * It writes a **recovery journal** before moving anything and removes it
//!   only once the reset is complete and verified. While the journal exists
//!   every load path (daemon, Core, the CLI) refuses to start, so an
//!   interrupted reset can never be mistaken for a healthy fresh chain.
//! * It backs up the current chain files into a freshly created, uniquely
//!   named directory, stages and verifies a fresh genesis before moving any
//!   original, fsyncs at each step, and never deletes a backup.
//! * It preserves the storage format and the metadata / headers / pruned
//!   horizon file set, refuses an ambiguous (mixed-format), symlinked or
//!   wrong-network layout rather than guessing, and never touches the wallet,
//!   `miner.seed`, the install id or an earlier backup.
//! * It is offline only. `main()` holds the one-writer data-directory lock,
//!   which the wallet takes too, so a running node or wallet makes it refuse.
//!   There is no recovery RPC and no automatic mining.
//!
//! It is a manual escape hatch, not a promise that every liveness problem is
//! solved: it resets to genesis and the operator resyncs. The existing
//! "don't mine while a fresh peer is ahead" guard keeps catch-up safe when the
//! node is started again.

use anyhow::Context;
use nightfall_consensus::Chain;
use nightfall_storage::dirlock::LOCK_FILE;
use nightfall_storage::{codec, now_unix, sync_dir, ChainStore, RECOVERY_JOURNAL};
use nightfall_types::NetworkId;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Chain-state files recovery backs up and resets. Everything else in the
/// datadir — the wallet directory, `miner.seed`, `install-id`, the lock file,
/// the recovery journal and any earlier backup directory — is left untouched.
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

/// Validate a recovery invocation **before** the caller takes the
/// data-directory lock.
///
/// `main()` acquires the lock for every command, and acquisition creates the
/// directory and opens the lock file. Doing the destructive-command checks here
/// first means a non-existent `--datadir` is refused without being created, and
/// a symlinked lock file is refused before lock acquisition could write through
/// it. The lock open is also hardened with `O_NOFOLLOW`; this is the matching
/// pre-flight check.
pub fn precheck(
    opts: &RecoverOpts,
    network_explicit: bool,
    datadir_explicit: bool,
) -> anyhow::Result<()> {
    if !network_explicit {
        anyhow::bail!(
            "recover-stalled-fork requires an explicit --network (for example --network mainnet)"
        );
    }
    if !datadir_explicit {
        anyhow::bail!(
            "recover-stalled-fork requires an explicit --datadir (the directory of the stalled node)"
        );
    }
    if !opts.force {
        anyhow::bail!(
            "recover-stalled-fork resets the chain in {} to a fresh genesis \
             (the wallet is kept). Stop the node, then re-run with --force.",
            opts.datadir.display()
        );
    }
    let datadir = opts.datadir.as_path();
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
    // The lock file must not be a symlink: lock acquisition would otherwise
    // write its pid line through to the link target.
    let lock = datadir.join(LOCK_FILE);
    if let Ok(lmd) = fs::symlink_metadata(&lock) {
        if lmd.file_type().is_symlink() {
            anyhow::bail!(
                "{} is a symlink; refusing (lock acquisition would follow it)",
                lock.display()
            );
        }
    }
    Ok(())
}

/// Back up a stalled-fork chain and reset the datadir to a fresh genesis,
/// keeping the wallet. See the module docs for the safety boundary.
///
/// The caller must already hold the data-directory lock (`main()` does) and
/// should have run [`precheck`] before acquiring it.
pub fn recover_stalled_fork(opts: &RecoverOpts) -> anyhow::Result<RecoverReport> {
    run(opts, Fault::None)
}

/// Test-only fault points, so each recoverable-failure path can be exercised
/// without a flaky filesystem trick. Production always passes `Fault::None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fault {
    None,
    /// Fail while saving the staged fresh genesis (before any original moves).
    StagingSave,
    /// Fail partway through moving the originals into the backup.
    MidRename,
    /// Fail after all originals are in the backup, before any replacement lands.
    AfterBackup,
    /// Fail partway through installing the staged replacement.
    DuringInstall,
}

fn run(opts: &RecoverOpts, fault: Fault) -> anyhow::Result<RecoverReport> {
    let datadir = opts.datadir.as_path();
    let network = opts.network;

    // Defensive re-validation under the lock (precheck ran before it).
    if !opts.force {
        anyhow::bail!("--force is required");
    }
    let md = fs::symlink_metadata(datadir)
        .with_context(|| format!("--datadir {} does not exist", datadir.display()))?;
    if md.file_type().is_symlink() || !md.is_dir() {
        anyhow::bail!("--datadir {} is not a usable directory", datadir.display());
    }

    let store = ChainStore::new(datadir);
    let format = codec::detect(datadir);

    // Mixed format is ambiguous: codec::detect would silently pick binary.
    if is_regular_file(&datadir.join(codec::BLOCKS_BIN))
        && is_regular_file(&datadir.join(codec::BLOCKS_JSONL))
    {
        anyhow::bail!(
            "{} holds both {} and {} — an ambiguous mixed-format layout. \
             Refusing to guess which is the real chain; resolve it by hand first.",
            datadir.display(),
            codec::BLOCKS_BIN,
            codec::BLOCKS_JSONL,
        );
    }

    let present = present_chain_files(datadir)?;
    if present.is_empty() {
        anyhow::bail!(
            "no NIGHTFALLCOIN chain found in {}. recover-stalled-fork needs an \
             existing datadir with a chain; check --datadir.",
            datadir.display()
        );
    }

    // A prior interrupted recovery must be resolved (backup restored) first.
    if datadir.join(RECOVERY_JOURNAL).is_file() {
        anyhow::bail!(
            "an earlier recover-stalled-fork did not finish ({} is present). Restore \
             that run's backup into the datadir and delete {} before running recovery again.",
            RECOVERY_JOURNAL,
            RECOVERY_JOURNAL
        );
    }

    // Confirm the network and genesis from chain-meta.json — never guess them.
    let meta_path = datadir.join("chain-meta.json");
    if !meta_path.is_file() {
        anyhow::bail!(
            "{} holds chain files but no readable chain-meta.json, so its network \
             and state cannot be confirmed. Refusing to guess.",
            datadir.display()
        );
    }
    let (stored_net, stored_genesis) = read_meta_identity(&meta_path)?;
    if stored_net != network {
        anyhow::bail!(
            "datadir holds a {} chain but --network {} was given. Pass the matching \
             --network; refusing to reset a chain for the wrong network.",
            stored_net.as_str(),
            network.as_str()
        );
    }
    let fresh_tmpl = Chain::new_fair(network)?;
    let expected_genesis = fresh_tmpl.genesis_hash.to_hex();
    if let Some(g) = &stored_genesis {
        if g != &expected_genesis {
            anyhow::bail!(
                "the datadir's recorded genesis {} does not match the {} genesis {}. \
                 This is not a genuine {} chain; refusing.",
                g,
                network.as_str(),
                expected_genesis,
                network.as_str()
            );
        }
    }

    let (previous_height, previous_tip) = store
        .peek_meta()
        .map(|(h, tip, _)| (h, Some(tip)))
        .unwrap_or((0, None));

    // A freshly created, uniquely named backup directory. `create_dir` (not
    // create_dir_all) fails if the name is taken, so a repeated run never
    // reuses or overwrites an earlier backup.
    let backup_dir = unique_dir(datadir, BACKUP_PREFIX)?;

    // Stage and verify the replacement BEFORE moving any original. If this
    // fails the originals have not been touched and no journal exists.
    let staging = unique_dir(datadir, STAGING_PREFIX)?;
    let genesis_hash = match build_and_verify_fresh(&staging, datadir, network, format, fault) {
        Ok(h) => h,
        Err(e) => {
            let _ = fs::remove_dir_all(&staging);
            let _ = fs::remove_dir(&backup_dir);
            return Err(e.context("could not build a fresh genesis; the datadir was not changed"));
        }
    };

    // Mark the reset in progress and make the mark durable before the first
    // destructive move. From here a crash leaves the journal, which every load
    // path refuses to start over until the backup is restored.
    write_journal(datadir, &backup_dir, network)?;
    let _ = sync_dir(datadir);

    // Move the originals into the backup. Everything moved is recoverable from
    // `backup_dir`.
    for (moved, name) in present.iter().enumerate() {
        if fault == Fault::MidRename && moved == 1 {
            anyhow::bail!(
                "injected mid-rename fault; the originals moved so far are in {}.",
                backup_dir.display()
            );
        }
        fs::rename(datadir.join(name), backup_dir.join(name)).with_context(|| {
            format!(
                "failed moving {name} into the backup at {}. The originals moved so far are \
                 there; the journal marks the datadir unsafe to start. Restore the backup.",
                backup_dir.display()
            )
        })?;
    }
    let _ = sync_dir(datadir);

    if fault == Fault::AfterBackup {
        anyhow::bail!(
            "injected fault after backup; the originals are in {}.",
            backup_dir.display()
        );
    }

    // Install the staged, already-verified replacement.
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
    let _ = sync_dir(datadir);

    // Verify the reset without going through load_or_new (which refuses while
    // the journal is present). Read the just-written metadata directly.
    verify_fresh_datadir(&store, datadir, &expected_genesis, format).with_context(|| {
        format!(
            "the reset datadir did not verify; the originals are in {}. Restore them to recover.",
            backup_dir.display()
        )
    })?;

    // The reset is complete and verified: remove the journal so a normal start
    // is allowed again, and make that durable.
    let _ = fs::remove_file(datadir.join(RECOVERY_JOURNAL));
    let _ = sync_dir(datadir);

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
/// a clean genesis of `network`. The datadir's `install-id` is copied in first
/// so the staged metadata belongs to this datadir and the reset chain is
/// trusted on the next load.
fn build_and_verify_fresh(
    staging: &Path,
    datadir: &Path,
    network: NetworkId,
    format: codec::Format,
    fault: Fault,
) -> anyhow::Result<String> {
    fs::create_dir_all(staging)?;

    let install_id = datadir.join("install-id");
    if install_id.is_file() {
        fs::copy(&install_id, staging.join("install-id"))?;
    }

    // Preserve the storage format: an empty staging dir is detected as binary,
    // so place an empty blocks.jsonl to make it JSON before the first save.
    if format == codec::Format::Json {
        fs::write(staging.join(codec::BLOCKS_JSONL), b"")?;
    }
    if codec::detect(staging) != format {
        anyhow::bail!("staging did not adopt the {:?} storage format", format);
    }

    let fresh = Chain::new_fair(network)?;
    let staged_store = ChainStore::new(staging);

    if fault == Fault::StagingSave {
        anyhow::bail!("injected staging-save fault");
    }
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

/// Confirm the datadir now holds a clean fresh genesis, reading the metadata
/// directly (load_or_new refuses while the recovery journal is still present).
fn verify_fresh_datadir(
    store: &ChainStore,
    datadir: &Path,
    expected_genesis: &str,
    format: codec::Format,
) -> anyhow::Result<()> {
    let (count, _tip, genesis) = store
        .peek_meta()
        .ok_or_else(|| anyhow::anyhow!("the reset datadir has no readable metadata"))?;
    if count != 0 {
        anyhow::bail!("the reset chain is not at genesis (block_count {count})");
    }
    if genesis != expected_genesis {
        anyhow::bail!("the reset genesis does not match the expected genesis");
    }
    if codec::detect(datadir) != format {
        anyhow::bail!("the reset changed the datadir's storage format");
    }
    Ok(())
}

fn write_journal(datadir: &Path, backup_dir: &Path, network: NetworkId) -> anyhow::Result<()> {
    let body = serde_json::json!({
        "backup": backup_dir.display().to_string(),
        "network": network.as_str(),
        "started_unix": now_unix(),
    });
    let path = datadir.join(RECOVERY_JOURNAL);
    let mut f = fs::File::create(&path).with_context(|| format!("creating {}", path.display()))?;
    f.write_all(&serde_json::to_vec_pretty(&body)?)?;
    // The journal itself must reach disk before the originals are moved.
    f.sync_all()?;
    Ok(())
}

fn is_regular_file(p: &Path) -> bool {
    fs::symlink_metadata(p)
        .map(|m| m.is_file())
        .unwrap_or(false)
}

/// Chain files present in `dir`, each confirmed a regular, non-symlinked file.
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

fn read_meta_identity(meta_path: &Path) -> anyhow::Result<(NetworkId, Option<String>)> {
    let raw = fs::read_to_string(meta_path)
        .with_context(|| format!("reading {}", meta_path.display()))?;
    let v: serde_json::Value =
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", meta_path.display()))?;
    let net = v
        .get("network")
        .and_then(|n| n.as_str())
        .ok_or_else(|| anyhow::anyhow!("{} has no network field", meta_path.display()))?;
    let net = match net {
        "mainnet" => NetworkId::Mainnet,
        "testnet" => NetworkId::Testnet,
        "devnet" => NetworkId::Devnet,
        other => anyhow::bail!("{} names an unknown network {other:?}", meta_path.display()),
    };
    let genesis = v
        .get("genesis_hash")
        .and_then(|g| g.as_str())
        .map(String::from);
    Ok((net, genesis))
}

/// A freshly created directory under `parent` whose name starts with `prefix`
/// and is unique by construction. A seconds-resolution name can collide on a
/// repeated invocation and silently reuse an earlier directory; nanoseconds
/// plus a process-lifetime counter, together with `create_dir` failing on an
/// existing name, make reuse impossible.
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

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

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

    fn mine(chain: &mut Chain, blocks: u64) {
        let miner = WalletKeys::generate().address();
        let base = now();
        for i in 0..blocks {
            chain
                .mine_block(&miner, vec![], base - (blocks - i) * 60)
                .unwrap();
        }
    }

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
        mine(&mut chain, blocks);
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

    fn journal_present(datadir: &Path) -> bool {
        datadir.join(RECOVERY_JOURNAL).is_file()
    }

    #[test]
    fn binary_format_is_preserved_and_reset_to_genesis() {
        let dir = unique_tmp("bin");
        let (tip, count) = seed_chain(&dir, NetworkId::Devnet, codec::Format::Binary, 3);
        assert_eq!(count, 3);

        let report = recover_stalled_fork(&opts(&dir, NetworkId::Devnet, true)).unwrap();

        assert_eq!(report.format, codec::Format::Binary);
        assert_eq!(report.previous_tip.as_deref(), Some(tip.as_str()));
        assert_eq!(codec::detect(&dir), codec::Format::Binary);
        assert!(dir.join("blocks.bin").is_file());
        assert!(!dir.join("blocks.jsonl").exists());
        assert!(
            !journal_present(&dir),
            "a successful reset leaves no journal"
        );
        let chain = ChainStore::new(&dir)
            .load_or_new(NetworkId::Devnet)
            .unwrap();
        assert_eq!(chain.block_count(), 0);
        assert!(report.backup_dir.join("blocks.bin").is_file());

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

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_mixed_format_datadir_is_refused() {
        let dir = unique_tmp("mixed");
        seed_chain(&dir, NetworkId::Devnet, codec::Format::Binary, 1);
        // Add a stray blocks.jsonl next to blocks.bin: ambiguous.
        fs::write(dir.join("blocks.jsonl"), b"").unwrap();

        let err = recover_stalled_fork(&opts(&dir, NetworkId::Devnet, true)).unwrap_err();
        assert!(err.to_string().contains("ambiguous"), "{err}");
        assert!(backup_dirs(&dir).is_empty());
        assert!(!journal_present(&dir));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn wallet_and_miner_seed_are_byte_identical() {
        let dir = unique_tmp("wallet");
        seed_chain(&dir, NetworkId::Devnet, codec::Format::Binary, 2);

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
        let r2 = recover_stalled_fork(&opts(&dir, NetworkId::Devnet, true)).unwrap();

        assert_ne!(r1.backup_dir, r2.backup_dir, "backups collided");
        assert_eq!(backup_dirs(&dir).len(), 2);

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
        assert!(backup_dirs(&dir).is_empty());
        assert!(dir.join("chain-meta.json").is_file());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_genesis_mismatch_is_refused() {
        let dir = unique_tmp("genmismatch");
        seed_chain(&dir, NetworkId::Devnet, codec::Format::Binary, 1);
        // Tamper the recorded genesis so it no longer matches the devnet genesis.
        let meta = fs::read_to_string(dir.join("chain-meta.json")).unwrap();
        let bad: serde_json::Value = {
            let mut v: serde_json::Value = serde_json::from_str(&meta).unwrap();
            v["genesis_hash"] = serde_json::json!("00".repeat(32));
            v
        };
        fs::write(
            dir.join("chain-meta.json"),
            serde_json::to_vec_pretty(&bad).unwrap(),
        )
        .unwrap();

        let err = recover_stalled_fork(&opts(&dir, NetworkId::Devnet, true)).unwrap_err();
        assert!(err.to_string().contains("genesis"), "{err}");
        assert!(backup_dirs(&dir).is_empty());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_genuine_pruned_chain_is_backed_up_and_restores() {
        let dir = unique_tmp("prunedreal");
        let store = ChainStore::new(&dir);
        let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();
        mine(&mut chain, 8);
        let dropped = chain.prune_keep(3).unwrap();
        assert!(dropped > 0 && chain.is_pruned(), "fixture must be pruned");
        let tip = chain.tip_hash().to_hex();
        store.save(&chain).unwrap();
        assert!(
            dir.join("utxo-horizon.json").is_file() || dir.join("headers.jsonl").is_file(),
            "a pruned chain writes a horizon/headers sidecar"
        );

        let report = recover_stalled_fork(&opts(&dir, NetworkId::Devnet, true)).unwrap();

        for f in &report.files_backed_up {
            assert!(report.backup_dir.join(f).is_file(), "backup missing {f}");
        }
        // The reset datadir is a clean unpruned genesis.
        let reset = ChainStore::new(&dir)
            .load_or_new(NetworkId::Devnet)
            .unwrap();
        assert_eq!(reset.block_count(), 0);
        assert!(!reset.is_pruned());
        // An operator restores by moving the backup back into the datadir,
        // which still holds its install-id. Mirror that: the install-id lets
        // the trusted validation record stand, so the pruned bodies (gone by
        // definition) are not re-checked.
        let restored = unique_tmp("prunedrestore");
        fs::copy(dir.join("install-id"), restored.join("install-id")).unwrap();
        for f in &report.files_backed_up {
            fs::copy(report.backup_dir.join(f), restored.join(f)).unwrap();
        }
        let back = ChainStore::new(&restored)
            .load_or_new(NetworkId::Devnet)
            .unwrap();
        assert_eq!(back.tip_hash().to_hex(), tip);
        assert!(back.is_pruned());

        fs::remove_dir_all(&dir).ok();
        fs::remove_dir_all(&restored).ok();
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
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn meta_without_a_chain_file_is_refused_not_guessed() {
        let dir = unique_tmp("metaonly");
        seed_chain(&dir, NetworkId::Devnet, codec::Format::Binary, 1);
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
        let outside = unique_tmp("outside").join("elsewhere.bin");
        fs::write(&outside, b"not ours").unwrap();
        fs::remove_file(dir.join("blocks.bin")).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("blocks.bin")).unwrap();

        let err = recover_stalled_fork(&opts(&dir, NetworkId::Devnet, true)).unwrap_err();
        assert!(err.to_string().contains("symlink"), "{err}");
        assert!(backup_dirs(&dir).is_empty());
        assert_eq!(fs::read(&outside).unwrap(), b"not ours");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_prior_interrupted_recovery_is_refused() {
        let dir = unique_tmp("priorjournal");
        seed_chain(&dir, NetworkId::Devnet, codec::Format::Binary, 1);
        fs::write(dir.join(RECOVERY_JOURNAL), b"{\"backup\":\"somewhere\"}").unwrap();

        let err = recover_stalled_fork(&opts(&dir, NetworkId::Devnet, true)).unwrap_err();
        assert!(err.to_string().contains("did not finish"), "{err}");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_staging_save_fault_leaves_the_datadir_untouched() {
        let dir = unique_tmp("faultstage");
        let (tip, _) = seed_chain(&dir, NetworkId::Devnet, codec::Format::Binary, 2);
        let blocks_before = fs::read(dir.join("blocks.bin")).unwrap();

        let err = run(&opts(&dir, NetworkId::Devnet, true), Fault::StagingSave).unwrap_err();
        assert!(err.to_string().contains("fresh genesis"), "{err}");

        // Nothing moved, no journal, no backup, chain intact.
        assert!(backup_dirs(&dir).is_empty());
        assert!(!journal_present(&dir));
        assert_eq!(fs::read(dir.join("blocks.bin")).unwrap(), blocks_before);
        let chain = ChainStore::new(&dir)
            .load_or_new(NetworkId::Devnet)
            .unwrap();
        assert_eq!(chain.tip_hash().to_hex(), tip);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_mid_rename_fault_preserves_the_originals_and_leaves_a_journal() {
        let dir = unique_tmp("faultmid");
        let (tip, _) = seed_chain(&dir, NetworkId::Devnet, codec::Format::Binary, 2);
        // Ensure more than one chain file so a mid-rename point exists.
        fs::write(dir.join("headers.jsonl"), b"{}\n").unwrap();

        let err = run(&opts(&dir, NetworkId::Devnet, true), Fault::MidRename).unwrap_err();
        assert!(err.to_string().contains("mid-rename"), "{err}");

        // Journal remains, so a normal start refuses.
        assert!(journal_present(&dir));
        let dirs = backup_dirs(&dir);
        assert_eq!(dirs.len(), 1);
        let reload = ChainStore::new(&dir).load_or_new(NetworkId::Devnet);
        assert!(
            reload.is_err(),
            "load must refuse while the journal is present"
        );
        assert!(reload.unwrap_err().to_string().contains("interrupted"));

        // Restoring the backup reproduces the original tip.
        let restored = unique_tmp("midrestore");
        for f in ["blocks.bin", "chain-meta.json", "headers.jsonl"] {
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
    fn an_after_backup_fault_preserves_the_originals_and_leaves_a_journal() {
        let dir = unique_tmp("faultafter");
        let (tip, _) = seed_chain(&dir, NetworkId::Devnet, codec::Format::Binary, 2);
        let blocks_before = fs::read(dir.join("blocks.bin")).unwrap();

        let err = run(&opts(&dir, NetworkId::Devnet, true), Fault::AfterBackup).unwrap_err();
        assert!(err.to_string().contains("after backup"), "{err}");

        assert!(journal_present(&dir));
        let dirs = backup_dirs(&dir);
        assert_eq!(dirs.len(), 1);
        assert_eq!(fs::read(dirs[0].join("blocks.bin")).unwrap(), blocks_before);
        let restored = unique_tmp("afterrestore");
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
    fn a_during_install_fault_preserves_the_originals() {
        let dir = unique_tmp("faultinstall");
        let (tip, _) = seed_chain(&dir, NetworkId::Devnet, codec::Format::Binary, 2);

        let err = run(&opts(&dir, NetworkId::Devnet, true), Fault::DuringInstall).unwrap_err();
        assert!(err.to_string().contains("during install"), "{err}");

        assert!(journal_present(&dir));
        let dirs = backup_dirs(&dir);
        assert_eq!(dirs.len(), 1);
        let restored = unique_tmp("installrestore");
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

        let fresh = ChainStore::new(&dir)
            .load_or_new(NetworkId::Devnet)
            .unwrap();
        assert_eq!(fresh.block_count(), 0);

        let backed = ChainStore::new(&report.backup_dir)
            .load_or_new(NetworkId::Devnet)
            .unwrap();
        assert_eq!(backed.tip_hash().to_hex(), tip);
        assert_eq!(backed.block_count(), 4);

        fs::remove_dir_all(&dir).ok();
    }
}
