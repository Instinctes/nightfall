//! Exercise clap requirements and the actual OS-level lock through nightfalld.
use nightfall_storage::{dirlock, ChainStore};
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
