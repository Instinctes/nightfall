//! Exercise clap requirements and the actual OS-level lock through nightfalld.
#[cfg(unix)]
use nightfall_storage::dirlock;
use nightfall_storage::ChainStore;
use nightfall_types::NetworkId;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};
struct Tmp(PathBuf);
impl Tmp {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "nf-recovery-cli-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        ChainStore::new(&p)
            .save(&nightfall_consensus::Chain::new_fair(NetworkId::Devnet).unwrap())
            .unwrap();
        Self(p)
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_nightfalld"))
        .args(args)
        .output()
        .unwrap()
}
#[cfg(unix)]
#[test]
fn explicit_flags_required_and_lock_refuses_live_holder() {
    let tmp = Tmp::new();
    let path = tmp.0.to_str().unwrap();
    for args in [
        vec!["recover-stalled-fork", "--force"],
        vec!["--datadir", path, "recover-stalled-fork", "--force"],
        vec!["--network", "devnet", "recover-stalled-fork", "--force"],
        vec![
            "--datadir",
            path,
            "--network",
            "devnet",
            "recover-stalled-fork",
        ],
    ] {
        assert!(
            !cli(&args).status.success(),
            "unexpected acceptance: {args:?}"
        );
    }
    let lock = dirlock::acquire(&tmp.0).unwrap();
    let out = cli(&[
        "--datadir",
        path,
        "--network",
        "devnet",
        "recover-stalled-fork",
        "--force",
    ]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("already using"));
    drop(lock);
    let out = cli(&[
        "--datadir",
        path,
        "--network",
        "devnet",
        "recover-stalled-fork",
        "--force",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("WITHOUT --mine"));
}

#[cfg(unix)]
#[test]
fn running_real_daemon_refuses_recovery_via_cli() {
    use std::{process::Stdio, thread, time::Duration};
    let tmp = Tmp::new();
    let path = tmp.0.to_str().unwrap();
    let mut daemon = Command::new(env!("CARGO_BIN_EXE_nightfalld"))
        .args([
            "--datadir",
            path,
            "--network",
            "devnet",
            "run",
            "--listen",
            "127.0.0.1:0",
            "--rpc-listen",
            "127.0.0.1:0",
            "--proxy",
            "off",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut locked = false;
    for _ in 0..60 {
        if daemon.try_wait().unwrap().is_some() {
            break;
        }
        match dirlock::acquire(&tmp.0) {
            Err(dirlock::LockError::Busy { .. }) => {
                locked = true;
                break;
            }
            Ok(guard) => drop(guard),
            Err(e) => panic!("lock test failed: {e}"),
        }
        thread::sleep(Duration::from_millis(100));
    }
    if locked {
        let out = cli(&[
            "--datadir",
            path,
            "--network",
            "devnet",
            "recover-stalled-fork",
            "--force",
        ]);
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("already using"));
    }
    let _ = daemon.kill();
    let _ = daemon.wait();
    assert!(locked, "daemon never acquired its data-directory lock");
}

#[test]
fn interrupted_layout_refuses_status_and_daemon_without_mutation() {
    let tmp = Tmp::new();
    let block = tmp.0.join("blocks.bin");
    let backup = tmp.0.join("recovery-original-backup");
    fs::create_dir(&backup).unwrap();
    fs::rename(&block, backup.join("blocks.bin")).unwrap();

    let old_meta = fs::read(tmp.0.join("chain-meta.json")).unwrap();
    let old_blocks = fs::read(backup.join("blocks.bin")).unwrap();

    fs::write(
        tmp.0.join(".stalled-fork-recovery-in-progress"),
        b"BACKUP=demo",
    )
    .unwrap();

    let path = tmp.0.to_str().unwrap();

    for command in ["status", "run"] {
        let output = cli(&["--network", "devnet", "--datadir", path, command]);

        assert!(!output.status.success(), "{command} unexpectedly started");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("unfinished stalled-fork recovery"),
            "{command}: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        assert!(!block.exists());
        assert_eq!(fs::read(tmp.0.join("chain-meta.json")).unwrap(), old_meta);
        assert_eq!(fs::read(backup.join("blocks.bin")).unwrap(), old_blocks);
    }
}

#[cfg(windows)]
#[test]
fn windows_recovery_explicitly_refuses_without_changes() {
    let tmp = Tmp::new();
    let original = fs::read(tmp.0.join("blocks.bin")).unwrap();

    let output = cli(&[
        "--network",
        "devnet",
        "--datadir",
        tmp.0.to_str().unwrap(),
        "recover-stalled-fork",
        "--force",
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("supported only on Unix"));

    assert_eq!(fs::read(tmp.0.join("blocks.bin")).unwrap(), original);
    assert!(!tmp.0.join(".stalled-fork-recovery-in-progress").exists());
}
