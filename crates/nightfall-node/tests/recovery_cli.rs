//! End-to-end tests for `nightfalld recover-stalled-fork`, driven through the
//! real binary.
//!
//! The one thing a direct call to the recovery helper cannot prove is that a
//! running node or wallet makes the command refuse: `main()` takes the
//! data-directory lock before dispatching, so the refusal only happens for a
//! real process. These tests hold that same lock and spawn the actual binary.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const BIN: &str = env!("CARGO_BIN_EXE_nightfalld");

fn unique_tmp(tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!(
        "nf-recover-cli-{tag}-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        N.fetch_add(1, Ordering::Relaxed),
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn run(datadir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(BIN)
        .arg("--datadir")
        .arg(datadir)
        .arg("--network")
        .arg("devnet")
        .args(args)
        .output()
        .expect("spawn nightfalld")
}

fn backup_dirs(datadir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(datadir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("recovery-backup-"))
                    .unwrap_or(false)
        })
        .collect()
}

#[test]
fn a_running_node_lock_makes_recovery_refuse() {
    let dir = unique_tmp("locked");

    // Set up a real chain first (this process releases the lock on exit).
    assert!(run(&dir, &["init"]).status.success(), "init failed");

    // Hold the data-directory lock, exactly as a running node or wallet would.
    let _held = nightfall_storage::dirlock::acquire(&dir).expect("take the lock");

    let out = run(&dir, &["recover-stalled-fork", "--force"]);
    assert!(
        !out.status.success(),
        "recovery must refuse while the directory is locked"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("already using"),
        "the refusal must name the lock holder, got: {stderr}"
    );
    // It refused before doing anything: no backup was made.
    assert!(backup_dirs(&dir).is_empty());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn happy_path_resets_to_genesis_and_keeps_the_wallet() {
    let dir = unique_tmp("happy");

    assert!(run(&dir, &["init"]).status.success(), "init failed");
    assert!(
        run(&dir, &["mine", "--blocks", "3"]).status.success(),
        "mine failed"
    );
    // Mining wrote the miner seed — a private key recovery must never touch.
    let seed_path = dir.join("miner.seed");
    let seed_before = std::fs::read(&seed_path).expect("miner.seed exists after mining");

    let out = run(&dir, &["recover-stalled-fork", "--force"]);
    assert!(
        out.status.success(),
        "recovery failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("backup"),
        "summary must name the backup: {stdout}"
    );
    assert!(
        stdout.contains("resync") && stdout.to_lowercase().contains("mining"),
        "summary must give the resync-without-mining next step: {stdout}"
    );

    // A backup was made, the seed is byte-identical, and the chain is genesis.
    assert_eq!(backup_dirs(&dir).len(), 1);
    assert_eq!(std::fs::read(&seed_path).unwrap(), seed_before);

    let status = run(&dir, &["status"]);
    assert!(status.status.success());
    let s = String::from_utf8_lossy(&status.stdout);
    assert!(
        s.lines()
            .any(|l| l.starts_with("blocks") && l.contains(" 0")),
        "the reset chain must be back at zero blocks: {s}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_explicit_network_is_required() {
    let dir = unique_tmp("net");
    assert!(run(&dir, &["init"]).status.success(), "init failed");

    // No --network: the global default must not be enough for a destructive reset.
    let out = Command::new(BIN)
        .arg("--datadir")
        .arg(&dir)
        .args(["recover-stalled-fork", "--force"])
        .output()
        .expect("spawn nightfalld");
    assert!(!out.status.success(), "must require an explicit --network");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--network"), "got: {stderr}");

    std::fs::remove_dir_all(&dir).ok();
}
