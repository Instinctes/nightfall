//! Every check here runs in a private temporary directory. Nothing touches a
//! real data directory, a real node, or the network beyond a loopback socket
//! this file opens itself.

use super::*;
use std::io::BufRead;
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "nightfall-nodekit-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        assert!(self.0.to_string_lossy().contains("nightfall-nodekit-test-"));
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn meta(network: &str, genesis: &str, validated: u64) -> String {
    serde_json::json!({
        "network": network, "genesis_hash": genesis, "block_count": 42,
        "protocol_version": 8, "validated_by": "x", "validated_tip": "y",
        "validated_bytes": validated, "first_height": 0, "pruned": false,
        "horizon_work": "0", "validated_horizon_bytes": 0, "validated_headers_bytes": 0
    })
    .to_string()
}

fn find<'a>(r: &'a Report, check: &str) -> &'a Finding {
    r.findings
        .iter()
        .find(|f| f.check == check)
        .unwrap_or_else(|| panic!("no '{check}' finding in {:#?}", r.findings))
}

#[test]
fn the_pinned_genesis_is_what_consensus_produces_not_a_literal() {
    // Mainnet's genesis is public and fixed; if this ever changes, the kit is
    // being built from a tree that would fork, and should say so loudly.
    assert_eq!(
        pinned_genesis(NetworkId::Mainnet).unwrap(),
        "061a052d49607ff8f4b306c75d622ebd230cff4ec3a45a6dffc2f7738d4b20de"
    );
    assert_ne!(
        pinned_genesis(NetworkId::Mainnet).unwrap(),
        pinned_genesis(NetworkId::Devnet).unwrap()
    );
}

#[test]
fn a_missing_directory_is_a_failure_and_nothing_else_is_guessed() {
    let s = Scratch::new();
    let r = doctor(NetworkId::Devnet, &s.path().join("absent")).unwrap();
    assert_eq!(r.worst(), Level::Fail);
    assert_eq!(
        r.findings.len(),
        1,
        "no further checks on a directory that is not there"
    );
    assert!(
        !s.path().join("absent").exists(),
        "doctor must not create the directory"
    );
}

#[test]
fn a_fresh_directory_is_healthy_and_says_why() {
    let s = Scratch::new();
    let r = doctor(NetworkId::Devnet, s.path()).unwrap();
    assert_ne!(r.worst(), Level::Fail);
    assert_eq!(find(&r, "chain-meta").level, Level::Note);
}

#[test]
fn a_matching_chain_passes_and_doctor_writes_nothing() {
    let s = Scratch::new();
    let g = pinned_genesis(NetworkId::Devnet).unwrap();
    fs::write(s.path().join("chain-meta.json"), meta("devnet", &g, 1000)).unwrap();
    fs::write(s.path().join("blocks.bin"), vec![0u8; 1000]).unwrap();
    let before: Vec<_> = fs::read_dir(s.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let r = doctor(NetworkId::Devnet, s.path()).unwrap();
    assert_eq!(r.worst(), Level::Note);
    assert_eq!(find(&r, "genesis").level, Level::Ok);
    assert_eq!(find(&r, "blocks.bin").level, Level::Ok);
    let after: Vec<_> = fs::read_dir(s.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        before, after,
        "doctor must leave the directory exactly as it found it"
    );
}

#[test]
fn a_chain_from_another_genesis_is_named_as_a_different_chain() {
    let s = Scratch::new();
    fs::write(
        s.path().join("chain-meta.json"),
        meta("devnet", &"ab".repeat(32), 10),
    )
    .unwrap();
    let r = doctor(NetworkId::Devnet, s.path()).unwrap();
    let f = find(&r, "genesis");
    assert_eq!(f.level, Level::Fail);
    assert!(f.detail.contains("different chain"), "{}", f.detail);
}

#[test]
fn a_directory_of_the_wrong_network_fails() {
    let s = Scratch::new();
    let g = pinned_genesis(NetworkId::Mainnet).unwrap();
    fs::write(s.path().join("chain-meta.json"), meta("mainnet", &g, 10)).unwrap();
    let r = doctor(NetworkId::Devnet, s.path()).unwrap();
    assert_eq!(find(&r, "network").level, Level::Fail);
}

#[test]
fn a_truncated_chain_file_fails_and_a_growing_one_does_not() {
    let s = Scratch::new();
    let g = pinned_genesis(NetworkId::Devnet).unwrap();
    fs::write(s.path().join("chain-meta.json"), meta("devnet", &g, 1000)).unwrap();
    fs::write(s.path().join("blocks.bin"), vec![0u8; 400]).unwrap();
    assert_eq!(
        find(&doctor(NetworkId::Devnet, s.path()).unwrap(), "blocks.bin").level,
        Level::Fail
    );
    fs::write(s.path().join("blocks.bin"), vec![0u8; 1500]).unwrap();
    assert_eq!(
        find(&doctor(NetworkId::Devnet, s.path()).unwrap(), "blocks.bin").level,
        Level::Note
    );
}

#[test]
fn broken_metadata_is_reported_not_repaired() {
    let s = Scratch::new();
    fs::write(s.path().join("chain-meta.json"), b"{not json").unwrap();
    let r = doctor(NetworkId::Devnet, s.path()).unwrap();
    assert_eq!(find(&r, "chain-meta").level, Level::Fail);
    assert_eq!(
        fs::read(s.path().join("chain-meta.json")).unwrap(),
        b"{not json"
    );
}

#[cfg(unix)]
#[test]
fn a_lock_file_is_judged_by_whether_its_owner_is_alive() {
    let s = Scratch::new();
    let lock = s.path().join(".nightfall-lock");

    // This test process is certainly alive.
    fs::write(&lock, format!("pid {}\n", std::process::id())).unwrap();
    let live = find(&doctor(NetworkId::Devnet, s.path()).unwrap(), "lock")
        .detail
        .clone();
    assert!(live.contains("is running"), "{live}");

    // A leftover file from a dead process is the common case after a crash,
    // and it must not be reported as held — that was the original bug.
    let mut dead = 999_999u32;
    while process_alive(dead) == Some(true) {
        dead -= 1;
    }
    fs::write(&lock, format!("pid {dead}\n")).unwrap();
    let stale = find(&doctor(NetworkId::Devnet, s.path()).unwrap(), "lock")
        .detail
        .clone();
    assert!(
        stale.contains("no longer running") && stale.contains("Harmless"),
        "{stale}"
    );

    assert!(lock.exists(), "the lock file must never be removed");
}

#[cfg(unix)]
#[test]
fn a_world_readable_seed_is_a_warning_with_the_fix() {
    use std::os::unix::fs::PermissionsExt;
    let s = Scratch::new();
    let seed = s.path().join("core.seed");
    fs::write(&seed, b"00").unwrap();
    fs::set_permissions(&seed, fs::Permissions::from_mode(0o644)).unwrap();
    let r = doctor(NetworkId::Devnet, s.path()).unwrap();
    let f = find(&r, "permissions");
    assert_eq!(f.level, Level::Warn);
    assert!(f.detail.contains("chmod 600"));
    assert_eq!(
        fs::metadata(&seed).unwrap().permissions().mode() & 0o777,
        0o644,
        "doctor reports, it does not chmod"
    );
}

fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[test]
fn verify_accepts_matching_files_and_names_mismatches() {
    let s = Scratch::new();
    fs::write(s.path().join("good.bin"), b"good").unwrap();
    fs::write(s.path().join("bad.bin"), b"tampered").unwrap();
    let sums = s.path().join("SHA256SUMS");
    fs::write(
        &sums,
        format!(
            "{}  good.bin\n{}  bad.bin\n{}  absent.bin\n",
            sha(b"good"),
            sha(b"original"),
            sha(b"x")
        ),
    )
    .unwrap();
    let r = verify_sums(&sums, s.path(), None).unwrap();
    assert_eq!(r[0], ("good.bin".into(), Verdict::Ok));
    assert!(matches!(r[1].1, Verdict::Mismatch { .. }));
    assert_eq!(r[2].1, Verdict::Missing);
}

#[test]
fn verify_refuses_a_checksum_list_that_points_outside_its_directory() {
    let s = Scratch::new();
    let sums = s.path().join("SHA256SUMS");
    for name in ["../escape", "/etc/passwd", "sub/dir", "..\\win", ".."] {
        fs::write(&sums, format!("{}  {name}\n", "a".repeat(64))).unwrap();
        assert!(
            verify_sums(&sums, s.path(), None).is_err(),
            "{name} was followed"
        );
    }
}

#[test]
fn verify_refuses_malformed_lines_and_unknown_single_files() {
    let s = Scratch::new();
    let sums = s.path().join("SHA256SUMS");
    fs::write(&sums, "nothex  file\n").unwrap();
    assert!(verify_sums(&sums, s.path(), None).is_err());
    fs::write(&sums, format!("{}  file\n", sha(b"f"))).unwrap();
    assert!(verify_sums(&sums, s.path(), Some("other")).is_err());
}

fn spec(kind: ServiceKind) -> ServiceSpec {
    ServiceSpec {
        kind,
        network: NetworkId::Mainnet,
        binary: "/home/op/bin/nightfalld".into(),
        datadir: "/home/op/nightfall".into(),
        user: Some("nightfall".into()),
        public_p2p: false,
        light_port: None,
    }
}

#[test]
fn the_light_api_is_only_ever_offered_on_loopback() {
    for kind in [ServiceKind::Systemd, ServiceKind::Launchd] {
        let mut s = spec(kind);
        assert!(
            !render_service(&s).unwrap().contains("--mobile-listen"),
            "off unless asked"
        );
        s.light_port = Some(17888);
        s.public_p2p = true;
        let unit = render_service(&s).unwrap();
        assert!(unit.contains("--mobile-listen") && unit.contains("127.0.0.1:17888"));
        assert!(!unit.contains("0.0.0.0:17888"));
        s.light_port = Some(0);
        assert!(render_service(&s).is_err());
    }
}

fn host(domain: &str) -> SelfHostSpec {
    SelfHostSpec {
        domain: domain.into(),
        wallet_dir: "/srv/nightfall-wallet".into(),
        light_port: 17888,
    }
}

#[test]
fn the_self_hosted_wallet_keeps_its_policy_and_reaches_only_the_light_api() {
    let c = render_caddy(&host("wallet.example.org")).unwrap();
    assert!(c.starts_with("# Generated by nightfall-nodekit"));
    assert!(c.contains("wallet.example.org {"));
    // Same policy as the official origin: the page may talk to itself only.
    assert!(c.contains("connect-src 'self'") && c.contains("'wasm-unsafe-eval'"));
    assert!(c.contains("frame-ancestors 'none'"));
    // Only POST /wallet-api is forwarded, to loopback, with the node's body cap.
    assert!(c.contains("method POST") && c.contains("reverse_proxy 127.0.0.1:17888"));
    assert!(c.contains("max_size 512KB"));
    assert_eq!(c.matches("reverse_proxy").count(), 1);
    assert!(c.contains("root * /srv/nightfall-wallet"));
    // Braces balance, or Caddy refuses the file.
    assert_eq!(c.matches('{').count(), c.matches('}').count());
    // Public-source tests must be self-contained; compare generated headers.
    for part in [
        "default-src 'self'",
        "connect-src 'self'",
        "frame-ancestors 'none'",
        "base-uri 'none'",
        "form-action 'none'",
        "object-src 'none'",
    ] {
        assert!(
            c.contains(part),
            "Generated policy no longer contains {part}"
        );
    }
}

#[test]
fn self_hosting_refuses_odd_hosts_paths_and_the_official_domain() {
    for bad in [
        "localhost",
        "Wallet.Example.org",
        "wallet example.org",
        "-a.example.org",
        "a.example.org;",
        "wallet.nightfallcoin.org",
        "x.example.org\n",
    ] {
        assert!(render_caddy(&host(bad)).is_err(), "{bad} should be refused");
    }
    let mut h = host("wallet.example.org");
    h.wallet_dir = "srv/wallet".into();
    assert!(render_caddy(&h).is_err());
    h.wallet_dir = "/srv/my wallet".into();
    assert!(render_caddy(&h).is_err());
    h.wallet_dir = "/srv/w}allet".into();
    assert!(render_caddy(&h).is_err());
    let mut h = host("wallet.example.org");
    h.light_port = 0;
    assert!(render_caddy(&h).is_err());
}

#[test]
fn a_service_never_publishes_rpc_and_uses_the_real_start_command() {
    for kind in [ServiceKind::Systemd, ServiceKind::Launchd] {
        let mut s = spec(kind);
        s.public_p2p = true;
        let unit = render_service(&s).unwrap();
        assert!(
            unit.contains("127.0.0.1:17881"),
            "RPC must stay on loopback"
        );
        assert!(!unit.contains("0.0.0.0:17881"));
        assert!(unit.contains("0.0.0.0:17891"), "public P2P was requested");
        assert!(
            unit.contains("run"),
            "the node is started with its `run` command"
        );
        assert!(unit.contains("--network") && unit.contains("mainnet"));
    }
    let private = render_service(&spec(ServiceKind::Systemd)).unwrap();
    assert!(
        private.contains("127.0.0.1:17891"),
        "P2P stays private unless asked"
    );
    assert!(
        private.contains("NoNewPrivileges=true")
            && private.contains("ReadWritePaths=/home/op/nightfall")
    );
}

#[test]
fn a_service_refuses_relative_paths_root_and_quoting_hazards() {
    let mut s = spec(ServiceKind::Systemd);
    s.binary = "nightfalld".into();
    assert!(render_service(&s).is_err());
    let mut s = spec(ServiceKind::Systemd);
    s.datadir = "/home/op/my $dir".into();
    assert!(render_service(&s).is_err());
    let mut s = spec(ServiceKind::Systemd);
    s.user = Some("root".into());
    assert!(render_service(&s).is_err());
    let mut s = spec(ServiceKind::Systemd);
    s.user = Some("op; rm -rf /".into());
    assert!(render_service(&s).is_err());
}

#[test]
fn a_path_with_a_space_is_refused_for_systemd_and_escaped_for_launchd() {
    // The real macOS install path. systemd would split it in two; launchd
    // takes it as one argument and must receive valid XML.
    let app = PathBuf::from("/Applications/NIGHTFALLCOIN Core.app/Contents/MacOS/nightfalld");
    let mut s = spec(ServiceKind::Systemd);
    s.binary = app.clone();
    assert!(
        render_service(&s).is_err(),
        "systemd must not receive a path it will split"
    );
    let mut s = spec(ServiceKind::Launchd);
    s.binary = app;
    let plist = render_service(&s).unwrap();
    assert!(plist.contains(
        "<string>/Applications/NIGHTFALLCOIN Core.app/Contents/MacOS/nightfalld</string>"
    ));
    let mut s = spec(ServiceKind::Launchd);
    s.datadir = "/Users/op/a&b<c>".into();
    let plist = render_service(&s).unwrap();
    assert!(plist.contains("/Users/op/a&amp;b&lt;c&gt;") && !plist.contains("a&b<c>"));
}

#[test]
fn service_files_are_new_files_outside_system_directories() {
    let s = Scratch::new();
    let out = s.path().join("nightfall-mainnet.service");
    write_new_file(&out, "unit").unwrap();
    assert!(
        write_new_file(&out, "again").is_err(),
        "an existing file must not be overwritten"
    );
    assert_eq!(fs::read_to_string(&out).unwrap(), "unit");
    for p in [
        "/etc/systemd/system/x.service",
        "/Library/LaunchDaemons/x.plist",
        "/usr/lib/x",
    ] {
        assert!(
            write_new_file(Path::new(p), "x").is_err(),
            "{p} was not refused"
        );
    }
}

/// A fake node on a loopback port that answers one `status` request.
fn fake_node(reply: serde_json::Value) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        if let Ok((s, _)) = l.accept() {
            let mut line = String::new();
            let _ = std::io::BufReader::new(s.try_clone().unwrap()).read_line(&mut line);
            assert!(line.contains("\"status\""));
            let mut s = s;
            let _ = writeln!(
                s,
                "{}",
                serde_json::json!({"result": reply, "error": null, "id": 1})
            );
        }
    });
    addr
}

#[test]
fn status_reports_a_node_on_the_expected_chain() {
    let g = pinned_genesis(NetworkId::Devnet).unwrap();
    let addr = fake_node(
        serde_json::json!({"genesis": g, "blocks": 101, "best_peer_height": 105, "peers": 4, "loading": false}),
    );
    let s = status(NetworkId::Devnet, &addr, Duration::from_secs(3)).unwrap();
    assert_eq!(s.tip_height, 100);
    assert_eq!(s.lag, Some(5));
    assert_eq!(s.peers, Some(4));
}

#[test]
fn status_refuses_to_describe_a_node_on_another_chain() {
    let addr = fake_node(serde_json::json!({"genesis": "cd".repeat(32), "blocks": 5}));
    let e = status(NetworkId::Devnet, &addr, Duration::from_secs(3))
        .unwrap_err()
        .to_string();
    assert!(e.contains("different chain"), "{e}");
}

#[test]
fn status_explains_an_absent_node() {
    // Reserve the port throughout the probe to prevent another process
    // from taking it between bind and status().
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let e = status(
        NetworkId::Devnet,
        &addr.to_string(),
        Duration::from_millis(500),
    )
    .unwrap_err()
    .to_string();
    assert!(e.contains("no node answering"), "{e}");
}

#[test]
fn status_explains_peer_disconnect_before_reply() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let _ = stream.shutdown(std::net::Shutdown::Both);
    });
    let e = status(NetworkId::Devnet, &addr.to_string(), Duration::from_secs(3))
        .unwrap_err()
        .to_string();
    server.join().unwrap();
    assert!(e.contains("no node answering"), "{e}");
}
