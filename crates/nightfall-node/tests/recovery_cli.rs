//! End-to-end tests for `nightfalld recover-stalled-fork`, driven through the
//! real binary.
//!
//! Several boundaries only hold for a real process: the data-directory lock is
//! taken by `main()` before dispatch, input validation happens before that lock
//! (so a bad `--datadir` is never created), and the recovery journal is checked
//! on every normal start. These tests spawn the actual binary to exercise them.

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
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.is_dir()
                        && p.file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.starts_with("recovery-backup-"))
                            .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_running_node_lock_makes_recovery_refuse() {
    let dir = unique_tmp("locked");
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

    assert_eq!(backup_dirs(&dir).len(), 1);
    assert_eq!(std::fs::read(&seed_path).unwrap(), seed_before);
    // No journal remains, and a normal start works again.
    assert!(!dir.join(".recovery-journal.json").exists());

    let status = run(&dir, &["status"]);
    assert!(status.status.success(), "status after reset must succeed");
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

#[test]
fn a_nonexistent_datadir_is_refused_without_creating_it() {
    // A fresh unique path that does not exist. Recovery must refuse before the
    // lock (which would create it).
    let base = unique_tmp("nonexist");
    let missing = base.join("does-not-exist");
    assert!(!missing.exists());

    let out = Command::new(BIN)
        .arg("--datadir")
        .arg(&missing)
        .arg("--network")
        .arg("devnet")
        .args(["recover-stalled-fork", "--force"])
        .output()
        .expect("spawn nightfalld");
    assert!(
        !out.status.success(),
        "a nonexistent datadir must be refused"
    );
    assert!(
        !missing.exists(),
        "the datadir must not be created before the refusal"
    );

    std::fs::remove_dir_all(&base).ok();
}

#[cfg(unix)]
#[test]
fn a_symlinked_lock_file_is_not_followed() {
    let dir = unique_tmp("symlock");
    assert!(run(&dir, &["init"]).status.success(), "init failed");
    // Replace the lock file with a symlink to an unrelated fixture.
    let victim = unique_tmp("symlock-victim").join("victim");
    std::fs::write(&victim, b"do not touch").unwrap();
    let lock = dir.join(".nightfall-lock");
    std::fs::remove_file(&lock).ok();
    std::os::unix::fs::symlink(&victim, &lock).unwrap();

    let out = run(&dir, &["recover-stalled-fork", "--force"]);
    assert!(!out.status.success(), "a symlinked lock must be refused");
    assert_eq!(
        std::fs::read(&victim).unwrap(),
        b"do not touch",
        "the symlink target must be untouched"
    );
    assert!(backup_dirs(&dir).is_empty());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_interrupted_recovery_blocks_a_normal_start() {
    // Reproduce the maintainer's interrupted layout: the active blocks file is
    // in a backup, the old metadata remains, and the recovery journal is left.
    let dir = unique_tmp("interrupted");
    assert!(run(&dir, &["init"]).status.success(), "init failed");
    assert!(
        run(&dir, &["mine", "--blocks", "2"]).status.success(),
        "mine failed"
    );

    let backup = dir.join("recovery-backup-interrupted");
    std::fs::create_dir(&backup).unwrap();
    std::fs::rename(dir.join("blocks.bin"), backup.join("blocks.bin")).unwrap();
    std::fs::write(
        dir.join(".recovery-journal.json"),
        format!("{{\"backup\":\"{}\"}}", backup.display()),
    )
    .unwrap();

    // A normal start must refuse rather than report a fresh zero-block chain.
    let out = run(&dir, &["status"]);
    assert!(
        !out.status.success(),
        "status must refuse over an interrupted recovery, not report a fresh chain"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("interrupted") && stderr.contains("backup"),
        "the refusal must explain the interrupted recovery and the backup: {stderr}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
