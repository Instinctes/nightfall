//! End-to-end daemon shutdown and datadir restart contract.
//! This test runs the actual executable, not a mocked NodeHandle.
#![cfg(unix)]

use nightfall_storage::ChainStore;
use nightfall_types::NetworkId;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn start_daemon(dir: &Path) -> Child {
    let mut child = Command::new(env!("CARGO_BIN_EXE_nightfalld"))
        .arg("--network")
        .arg("devnet")
        .arg("--datadir")
        .arg(dir)
        .arg("run")
        .arg("--listen")
        .arg("127.0.0.1:0")
        .arg("--rpc-listen")
        .arg("127.0.0.1:0")
        .env("NIGHTFALL_PEERS_URL", "off")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn actual nightfalld");

    let output = child.stdout.take().expect("daemon stdout");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            match line {
                Ok(line) if line.contains("node online") => {
                    let _ = tx.send(Ok(()));
                    return;
                }
                Err(error) => {
                    let _ = tx.send(Err(error.to_string()));
                    return;
                }
                _ => {}
            }
        }
        let _ = tx.send(Err("node closed stdout before becoming ready".to_owned()));
    });

    match rx.recv_timeout(Duration::from_secs(45)) {
        Ok(Ok(())) => child,
        other => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("daemon not ready: {other:?}");
        }
    }
}

fn signal_and_stop(child: &mut Child) {
    assert!(Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .status()
        .expect("send SIGTERM")
        .success());

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().expect("poll daemon exit") {
            assert!(status.success(), "daemon exited unsuccessfully: {status}");
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("SIGTERM shutdown did not complete within 30 seconds");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn sigterm_clean_shutdown_then_restart_preserves_datadir() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "nightfall-daemon-restart-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create private test datadir");

    let mut first = start_daemon(&dir);
    signal_and_stop(&mut first);

    let chain = ChainStore::new(&dir)
        .load_or_new(NetworkId::Devnet)
        .expect("reload chain after SIGTERM");
    chain
        .verify_supply()
        .expect("supply invariant after clean stop");
    let tip = chain.tip_hash();
    drop(chain);

    let mut second = start_daemon(&dir);
    signal_and_stop(&mut second);

    let loaded = ChainStore::new(&dir)
        .load_or_new(NetworkId::Devnet)
        .expect("reload chain after second shutdown");
    loaded
        .verify_supply()
        .expect("supply invariant after restart");
    assert_eq!(loaded.tip_hash(), tip, "restart altered the canonical tip");

    fs::remove_dir_all(dir).expect("cleanup test datadir");
}
