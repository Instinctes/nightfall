//! Nightfall Node Kit.
//!
//! Tools for someone running their own node without asking the founder for
//! help. Everything here either reads, or writes a file the operator named —
//! never a system location, never a running node's data.
//!
//! * [`doctor`] inspects a data directory and says what is wrong with it.
//! * [`status`] asks a running node who it is and whether it is keeping up.
//! * [`verify_sums`] checks downloads against a published `SHA256SUMS` file.
//! * [`render_service`] writes a systemd or launchd unit for review.
//!
//! The network identity every check compares against is not a hash typed into
//! this file. It is computed from the same consensus code the node runs
//! ([`pinned_genesis`]), so a kit built from a given tree can never pin a
//! genesis that tree would not produce.

use anyhow::{bail, Context, Result};
use nightfall_types::NetworkId;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The genesis hash this build's consensus code produces for `network`.
pub fn pinned_genesis(network: NetworkId) -> Result<String> {
    let chain = nightfall_consensus::Chain::new_fair(network)
        .map_err(|e| anyhow::anyhow!("consensus refused to build genesis: {e}"))?;
    Ok(chain.genesis_hash.to_hex())
}

pub fn parse_network(name: &str) -> Result<NetworkId> {
    Ok(match name {
        "mainnet" => NetworkId::Mainnet,
        "testnet" => NetworkId::Testnet,
        "devnet" => NetworkId::Devnet,
        other => bail!("unknown network '{other}' (mainnet, testnet or devnet)"),
    })
}

// ------------------------------------------------------------------ doctor --

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    Note,
    Warn,
    Fail,
}

#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    pub level: Level,
    pub check: &'static str,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub network: String,
    pub datadir: String,
    pub pinned_genesis: String,
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn worst(&self) -> Level {
        self.findings
            .iter()
            .map(|f| f.level)
            .max_by_key(|l| *l as u8)
            .unwrap_or(Level::Ok)
    }

    fn push(&mut self, level: Level, check: &'static str, detail: impl Into<String>) {
        self.findings.push(Finding {
            level,
            check,
            detail: detail.into(),
        });
    }
}

/// Inspect a data directory. Opens files for reading only; creates nothing.
///
/// Safe to run beside a live node: it takes no lock and writes no byte. The
/// price of that is that a node writing while this reads can make a size
/// comparison momentarily stale — a reason to run it twice, not to take the
/// lock the node needs.
pub fn doctor(network: NetworkId, datadir: &Path) -> Result<Report> {
    let pinned = pinned_genesis(network)?;
    let mut r = Report {
        network: network.as_str().into(),
        datadir: datadir.display().to_string(),
        pinned_genesis: pinned.clone(),
        findings: Vec::new(),
    };

    let meta_dir = match fs::metadata(datadir) {
        Ok(m) if m.is_dir() => m,
        Ok(_) => {
            r.push(Level::Fail, "datadir", "exists but is not a directory");
            return Ok(r);
        }
        Err(e) => {
            r.push(
                Level::Fail,
                "datadir",
                format!("cannot be read ({e}). Nothing else can be checked."),
            );
            return Ok(r);
        }
    };
    let _ = meta_dir;
    r.push(Level::Ok, "datadir", "present and readable");

    // A copied-in chain from another network, or from an abandoned genesis, is
    // the failure this check exists for: it looks like a healthy directory.
    let meta_path = datadir.join("chain-meta.json");
    let meta: Option<serde_json::Value> = match fs::read(&meta_path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(v) => Some(v),
            Err(e) => {
                r.push(
                    Level::Fail,
                    "chain-meta",
                    format!("chain-meta.json is not valid JSON ({e}). Do not delete it; see the recovery section of docs/NODE-KIT.md."),
                );
                None
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            r.push(
                Level::Note,
                "chain-meta",
                "no chain yet — a fresh directory, or a node that has not saved its first block",
            );
            None
        }
        Err(e) => {
            r.push(Level::Fail, "chain-meta", format!("unreadable ({e})"));
            None
        }
    };

    if let Some(m) = &meta {
        match m.get("network").and_then(|v| v.as_str()) {
            Some(n) if n == network.as_str() => {
                r.push(Level::Ok, "network", format!("chain belongs to {n}"))
            }
            Some(n) => r.push(
                Level::Fail,
                "network",
                format!("this directory holds a {n} chain, not {}. Point the node at the right directory rather than editing files.", network.as_str()),
            ),
            None => r.push(Level::Fail, "network", "chain-meta.json names no network"),
        }
        match m.get("genesis_hash").and_then(|v| v.as_str()) {
            Some(g) if g == pinned => r.push(Level::Ok, "genesis", format!("matches this build ({})", short(g))),
            Some(g) => r.push(
                Level::Fail,
                "genesis",
                format!(
                    "chain starts at {}, this build expects {}. That is a different chain, not an older copy of this one.",
                    short(g),
                    short(&pinned)
                ),
            ),
            None => r.push(Level::Fail, "genesis", "chain-meta.json names no genesis"),
        }
        let blocks = m.get("block_count").and_then(|v| v.as_u64()).unwrap_or(0);
        r.push(Level::Note, "height", format!("{blocks} blocks on disk"));
        if m.get("pruned").and_then(|v| v.as_bool()).unwrap_or(false) {
            let first = m.get("first_height").and_then(|v| v.as_u64()).unwrap_or(0);
            r.push(
                Level::Note,
                "pruned",
                format!("bodies start at height {first}. Fine for relaying; wallets that need full history must use an archive node."),
            );
        }

        let validated = m.get("validated_bytes").and_then(|v| v.as_u64());
        match (fs::metadata(datadir.join("blocks.bin")), validated) {
            (Ok(f), Some(v)) if f.len() == v => {
                r.push(Level::Ok, "blocks.bin", format!("{} validated, size matches", human_bytes(v)))
            }
            (Ok(f), Some(v)) if f.len() > v => r.push(
                Level::Note,
                "blocks.bin",
                format!(
                    "{} beyond the last validated point. Normal while the node is appending; if the node is stopped, the next start re-verifies that tail.",
                    human_bytes(f.len() - v)
                ),
            ),
            (Ok(f), Some(v)) => r.push(
                Level::Fail,
                "blocks.bin",
                format!(
                    "{} shorter than the validated record ({} of {}). The file was truncated or replaced. Keep a copy before any repair.",
                    human_bytes(v - f.len()),
                    human_bytes(f.len()),
                    human_bytes(v)
                ),
            ),
            (Ok(f), None) => r.push(Level::Note, "blocks.bin", format!("{} (no validation record)", human_bytes(f.len()))),
            (Err(_), _) => r.push(Level::Warn, "blocks.bin", "chain-meta.json exists but blocks.bin does not"),
        }
    }

    // The node's lock is an OS advisory lock (flock / LockFileEx) that the
    // kernel releases when the process dies. The *file* stays behind with the
    // last owner's pid and is harmless. So the file existing says nothing on
    // its own — an earlier version of this check reported every leftover file
    // as "held". Finding out for certain would mean taking the lock, which a
    // read-only tool must not do beside a node that may want it; asking
    // whether the recorded pid is still alive answers the useful question
    // without touching anything.
    match fs::read_to_string(datadir.join(".nightfall-lock")) {
        Ok(owner) => {
            let pid = owner
                .trim()
                .strip_prefix("pid ")
                .and_then(|p| p.trim().parse::<u32>().ok());
            match pid.map(process_alive) {
                Some(Some(true)) => r.push(
                    Level::Note,
                    "lock",
                    format!("pid {} is running and owns this directory. This kit does not touch it.", pid.unwrap_or(0)),
                ),
                Some(Some(false)) => r.push(
                    Level::Note,
                    "lock",
                    format!("leftover record from pid {}, which is no longer running. Harmless — the node takes the lock over on start. Do not delete it while anything might be using the directory.", pid.unwrap_or(0)),
                ),
                _ => r.push(
                    Level::Note,
                    "lock",
                    format!("lock file present ('{}'); whether its owner is alive could not be determined here.", owner.trim()),
                ),
            }
        }
        Err(_) => r.push(
            Level::Note,
            "lock",
            "no lock file — no node has used this directory, or it was cleaned",
        ),
    }

    // A plaintext seed that others can read is the one wallet file worth a
    // warning from an operator tool.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for name in ["core.seed", "miner.seed", "wallet.seed"] {
            if let Ok(m) = fs::metadata(datadir.join(name)) {
                let mode = m.permissions().mode() & 0o777;
                if mode & 0o077 != 0 {
                    r.push(
                        Level::Warn,
                        "permissions",
                        format!(
                            "{name} is readable by others (mode {mode:o}). Run: chmod 600 {}",
                            datadir.join(name).display()
                        ),
                    );
                } else {
                    r.push(
                        Level::Ok,
                        "permissions",
                        format!("{name} is private (mode {mode:o})"),
                    );
                }
            }
        }
    }

    if let Ok(peers) = fs::read(datadir.join("peers.json")) {
        match serde_json::from_slice::<serde_json::Value>(&peers) {
            Ok(v) => {
                let n = v.as_array().map(|a| a.len()).or_else(|| {
                    v.as_object()
                        .and_then(|o| o.values().find_map(|x| x.as_array().map(|a| a.len())))
                });
                r.push(
                    Level::Note,
                    "peers",
                    match n {
                        Some(n) => format!("{n} remembered peers"),
                        None => "peers.json readable".into(),
                    },
                );
            }
            Err(_) => r.push(
                Level::Warn,
                "peers",
                "peers.json is not valid JSON; the node will rebuild it",
            ),
        }
    }

    Ok(r)
}

// ------------------------------------------------------------------ status --

#[derive(Clone, Debug, Serialize)]
pub struct NodeStatus {
    pub network: String,
    pub tip_height: u64,
    pub best_peer_height: Option<u64>,
    pub peers: Option<u64>,
    pub loading: bool,
    pub lag: Option<u64>,
}

/// Ask a running node for its status over its local RPC port, and refuse to
/// report anything if it is not the chain this build expects.
///
/// A node on the wrong network answers `status` happily; the only useful
/// answer from a tool like this is "that is not the node you think it is".
pub fn status(network: NetworkId, addr: &str, timeout: Duration) -> Result<NodeStatus> {
    let pinned = pinned_genesis(network)?;
    let sock = addr
        .to_socket_addrs()
        .with_context(|| format!("cannot resolve {addr}"))?
        .next()
        .with_context(|| format!("{addr} resolved to nothing"))?;
    let mut stream = TcpStream::connect_timeout(&sock, timeout).with_context(|| {
        format!(
            "no node answering at {addr}. Is it running, and is --rpc-listen set to this address?"
        )
    })?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    writeln!(stream, r#"{{"method":"status","params":{{}},"id":1}}"#)
        .with_context(|| format!("no node answering at {addr}: RPC write failed"))?;

    let mut line = Vec::new();
    let received = BufReader::new(stream.take(1 << 20))
        .read_until(b'\n', &mut line)
        .with_context(|| format!("no node answering at {addr}: RPC read failed"))?;
    if received == 0 {
        bail!("no node answering at {addr}: RPC connection closed without a reply");
    }
    let reply: serde_json::Value =
        serde_json::from_slice(&line).context("the node's reply was not JSON")?;
    if let Some(e) = reply.get("error").filter(|e| !e.is_null()) {
        bail!("node refused status: {e}");
    }
    let s = reply.get("result").context("reply had no result")?;

    let genesis = s.get("genesis").and_then(|v| v.as_str()).unwrap_or("");
    if genesis != pinned {
        bail!(
            "the node at {addr} is on a different chain: genesis {} where this build expects {}",
            short(genesis),
            short(&pinned)
        );
    }
    let tip = s
        .get("blocks")
        .and_then(|v| v.as_u64())
        .map(|b| b.saturating_sub(1))
        .or_else(|| s.get("tip_height").and_then(|v| v.as_u64()))
        .context("reply carried no height")?;
    let best = s.get("best_peer_height").and_then(|v| v.as_u64());
    Ok(NodeStatus {
        network: network.as_str().into(),
        tip_height: tip,
        best_peer_height: best,
        peers: s
            .get("peers")
            .and_then(|v| v.as_u64().or_else(|| v.as_array().map(|a| a.len() as u64))),
        loading: s.get("loading").and_then(|v| v.as_bool()).unwrap_or(false),
        lag: best.map(|b| b.saturating_sub(tip)),
    })
}

// ------------------------------------------------------------------ verify --

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum Verdict {
    Ok,
    Mismatch { expected: String, actual: String },
    Missing,
}

/// Check files in `dir` against a `SHA256SUMS`-style list.
///
/// Every line must be `<64 hex>  <name>`. A name with a path separator is
/// refused rather than followed: a checksum file is downloaded next to the
/// binaries, and it must not be able to point this tool at `../../etc/…`.
pub fn verify_sums(sums: &Path, dir: &Path, only: Option<&str>) -> Result<Vec<(String, Verdict)>> {
    let text =
        fs::read_to_string(sums).with_context(|| format!("cannot read {}", sums.display()))?;
    let mut out = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (hash, name) = line
            .split_once(char::is_whitespace)
            .with_context(|| format!("line {} is not '<hash>  <file>'", n + 1))?;
        let name = name.trim().trim_start_matches('*');
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("line {} has no SHA-256 hash", n + 1);
        }
        if name.is_empty()
            || name.contains('/')
            || name.contains('\\')
            || name == "."
            || name == ".."
        {
            bail!(
                "line {} names '{name}', which is not a plain file name",
                n + 1
            );
        }
        if only.is_some_and(|o| o != name) {
            continue;
        }
        let path = dir.join(name);
        let verdict = match fs::File::open(&path) {
            Ok(mut f) => {
                let mut h = Sha256::new();
                let mut buf = vec![0u8; 1 << 16];
                loop {
                    let k = f.read(&mut buf)?;
                    if k == 0 {
                        break;
                    }
                    h.update(&buf[..k]);
                }
                let actual = hex::encode(h.finalize());
                if actual.eq_ignore_ascii_case(hash) {
                    Verdict::Ok
                } else {
                    Verdict::Mismatch {
                        expected: hash.to_ascii_lowercase(),
                        actual,
                    }
                }
            }
            Err(_) => Verdict::Missing,
        };
        out.push((name.to_string(), verdict));
    }
    if let Some(o) = only {
        if out.is_empty() {
            bail!("{o} is not listed in {}", sums.display());
        }
    }
    Ok(out)
}

// ----------------------------------------------------------------- service --

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceKind {
    Systemd,
    Launchd,
}

pub struct ServiceSpec {
    pub kind: ServiceKind,
    pub network: NetworkId,
    pub binary: PathBuf,
    pub datadir: PathBuf,
    pub user: Option<String>,
    pub public_p2p: bool,
    /// Serve the light API (for a self-hosted web wallet) on this loopback
    /// port. Loopback only: a reverse proxy in front of it adds TLS and is
    /// what the world talks to.
    pub light_port: Option<u16>,
}

/// Directories a generated unit must never be written into directly.
///
/// Installing a service is a privileged act the operator should see happen.
/// This tool writes a file they can read first; the install step is a copy
/// they run themselves.
///
/// Deliberately not `/var` or `/private`: on macOS the per-user temporary
/// directory lives at `/var/folders/…` (really `/private/var/folders/…`), and
/// refusing it would block an operator's own scratch space while protecting
/// nothing. The list names where services are installed and where the
/// system keeps itself, which is what this guard is for.
const SYSTEM_PREFIXES: &[&str] = &[
    "/etc",
    "/usr",
    "/lib",
    "/lib64",
    "/bin",
    "/sbin",
    "/boot",
    "/Library",
    "/System",
    "/private/etc",
    "/var/lib/systemd",
];

/// Absolute in the sense of the machine the unit or site file is for.
///
/// systemd, launchd and Caddy files describe a Unix server, so the check
/// follows Unix rules on every host: a Windows operator preparing a Linux
/// server's unit must get the same answer as one on the server itself.
fn server_absolute(p: &Path) -> bool {
    p.to_string_lossy().starts_with('/')
}

pub fn render_service(spec: &ServiceSpec) -> Result<String> {
    for (what, p) in [("binary", &spec.binary), ("datadir", &spec.datadir)] {
        if !server_absolute(p) {
            bail!("{what} must be an absolute path; a service does not start in your shell's directory");
        }
        let s = p.to_string_lossy();
        if s.chars()
            .any(|c| c.is_control() || c == '"' || c == '\'' || c == '\\' || c == '%' || c == '$')
        {
            bail!("{what} contains a character that would need quoting in a unit file; choose a plain path");
        }
        // systemd splits ExecStart on whitespace, so "/Applications/NIGHTFALLCOIN
        // Core.app/…" would become two arguments and the unit would start
        // nothing — or the wrong thing. launchd takes one <string> per
        // argument and is fine with it; the path is XML-escaped below.
        if spec.kind == ServiceKind::Systemd && s.chars().any(char::is_whitespace) {
            bail!("{what} contains a space, which systemd would split into two arguments. Use a path without spaces, or a symlink to one.");
        }
    }
    if let Some(u) = &spec.user {
        if u.is_empty()
            || !u
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            || u == "root"
        {
            bail!("service user must be a plain unprivileged account name, not '{u}'");
        }
    }
    let net = spec.network.as_str();
    let p2p = spec.network.default_p2p_port();
    let rpc = spec.network.default_rpc_port();
    // RPC stays on loopback whatever else changes. It is an administrative
    // port, and this tool will not emit a unit that publishes it.
    let listen = if spec.public_p2p {
        format!("0.0.0.0:{p2p}")
    } else {
        format!("127.0.0.1:{p2p}")
    };
    if spec.light_port == Some(0) {
        bail!("light API port must be between 1 and 65535");
    }
    let bin = spec.binary.display();
    let dir = spec.datadir.display();
    let light_arg = spec
        .light_port
        .map(|p| format!(" --mobile-listen 127.0.0.1:{p}"))
        .unwrap_or_default();
    let light_plist = spec
        .light_port
        .map(|p| format!("    <string>--mobile-listen</string><string>127.0.0.1:{p}</string>\n"))
        .unwrap_or_default();
    Ok(match spec.kind {
        ServiceKind::Systemd => {
            let user = spec.user.as_deref().unwrap_or("nightfall");
            format!(
                "# Generated by nightfall-nodekit for {net}. Review, then install yourself:\n\
                 #   sudo cp nightfall-{net}.service /etc/systemd/system/\n\
                 #   sudo systemctl daemon-reload && sudo systemctl enable --now nightfall-{net}\n\
                 [Unit]\n\
                 Description=NIGHTFALLCOIN node ({net})\n\
                 After=network-online.target\n\
                 Wants=network-online.target\n\
                 \n\
                 [Service]\n\
                 Type=simple\n\
                 User={user}\n\
                 Group={user}\n\
                 ExecStart={bin} --network {net} --datadir {dir} run --listen {listen} --rpc-listen 127.0.0.1:{rpc}{light_arg}\n\
                 Restart=always\n\
                 RestartSec=30\n\
                 NoNewPrivileges=true\n\
                 PrivateTmp=true\n\
                 PrivateDevices=true\n\
                 ProtectSystem=strict\n\
                 ProtectHome=true\n\
                 ReadWritePaths={dir}\n\
                 \n\
                 [Install]\n\
                 WantedBy=multi-user.target\n"
            )
        }
        ServiceKind::Launchd => {
            let label = format!("org.nightfallcoin.node.{net}");
            let bin = xml_escape(&spec.binary.to_string_lossy());
            let dir = xml_escape(&spec.datadir.to_string_lossy());
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                 <!-- Generated by nightfall-nodekit for {net}. Review, then install yourself:\n\
                 \x20    cp {label}.plist ~/Library/LaunchAgents/\n\
                 \x20    launchctl load ~/Library/LaunchAgents/{label}.plist -->\n\
                 <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
                 <plist version=\"1.0\">\n<dict>\n\
                 \x20 <key>Label</key><string>{label}</string>\n\
                 \x20 <key>ProgramArguments</key>\n  <array>\n\
                 \x20   <string>{bin}</string>\n\
                 \x20   <string>--network</string><string>{net}</string>\n\
                 \x20   <string>--datadir</string><string>{dir}</string>\n\
                 \x20   <string>run</string>\n\
                 \x20   <string>--listen</string><string>{listen}</string>\n\
                 \x20   <string>--rpc-listen</string><string>127.0.0.1:{rpc}</string>\n\
                 {light_plist}\
                 \x20 </array>\n\
                 \x20 <key>RunAtLoad</key><true/>\n\
                 \x20 <key>KeepAlive</key><true/>\n\
                 \x20 <key>ThrottleInterval</key><integer>30</integer>\n\
                 \x20 <key>StandardOutPath</key><string>{dir}/node.log</string>\n\
                 \x20 <key>StandardErrorPath</key><string>{dir}/node.log</string>\n\
                 </dict>\n</plist>\n"
            )
        }
    })
}

pub struct SelfHostSpec {
    /// Public hostname the wallet will be served on, e.g. `wallet.example.org`.
    pub domain: String,
    /// Absolute path of the directory holding the web wallet's static files.
    pub wallet_dir: PathBuf,
    /// Loopback port of the node's light API (`--mobile-listen`).
    pub light_port: u16,
}

/// The web wallet's response headers, identical to what nightfallcoin.org's
/// Worker sends for the wallet origin. `connect-src 'self'` is the point: the
/// page can talk to its own origin and nothing else, so the node has to be
/// reached through the same host rather than by loosening the policy.
pub const WALLET_HEADERS: &[(&str, &str)] = &[
    ("Content-Security-Policy", "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'; object-src 'none'; upgrade-insecure-requests"),
    ("Strict-Transport-Security", "max-age=31536000"),
    ("X-Content-Type-Options", "nosniff"),
    ("X-Frame-Options", "DENY"),
    ("Referrer-Policy", "no-referrer"),
    ("Permissions-Policy", "accelerometer=(), camera=(), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), payment=(), usb=(), interest-cohort=()"),
    ("Cross-Origin-Opener-Policy", "same-origin"),
    ("Cross-Origin-Resource-Policy", "same-origin"),
];

/// A Caddyfile serving the web wallet and its node from one origin.
///
/// Caddy because it obtains and renews the TLS certificate itself: the wallet
/// needs a secure context (IndexedDB persistence, WebCrypto), and a guide that
/// ends in "now set up certificates" is where self-hosting usually stops.
/// Only POST `/wallet-api` reaches the node, bodies are capped at the node's
/// own 512 KB, and the node's light API stays on loopback behind it.
pub fn render_caddy(spec: &SelfHostSpec) -> Result<String> {
    let d = &spec.domain;
    let valid_label = |l: &str| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    };
    if d.len() > 253 || !d.contains('.') || !d.split('.').all(valid_label) {
        bail!("'{d}' is not a plain lowercase hostname such as wallet.example.org");
    }
    if d.ends_with("nightfallcoin.org") {
        bail!("this is for your own domain; nightfallcoin.org is already served");
    }
    if !server_absolute(&spec.wallet_dir) {
        bail!("wallet directory must be an absolute path");
    }
    let dir = spec.wallet_dir.to_string_lossy();
    if dir.chars().any(|c| {
        c.is_control()
            || c.is_whitespace()
            || matches!(c, '"' | '\'' | '\\' | '{' | '}' | '#' | '$')
    }) {
        bail!("wallet directory contains a character Caddy would need quoted; choose a plain path");
    }
    if spec.light_port == 0 {
        bail!("light API port must be between 1 and 65535");
    }
    let port = spec.light_port;
    let headers: String = WALLET_HEADERS
        .iter()
        .map(|(k, v)| format!("\t\t{k} \"{v}\"\n"))
        .collect();
    Ok(format!(
        "# Generated by nightfall-nodekit. Review, then install yourself, e.g.\n\
         #   sudo cp Caddyfile /etc/caddy/Caddyfile && sudo systemctl reload caddy\n\
         # The node must run with --mobile-listen 127.0.0.1:{port} (see docs/MY-NODE.md).\n\
         # Your node sees your visitors' IP addresses and the transactions they send.\n\
         # It never sees keys: they stay in the browser.\n\
         {d} {{\n\
         \tencode zstd gzip\n\
         \theader {{\n\
         {headers}\
         \t\t-Server\n\
         \t}}\n\
         \n\
         \t@api {{\n\
         \t\tpath /wallet-api\n\
         \t\tmethod POST\n\
         \t}}\n\
         \thandle @api {{\n\
         \t\theader Cache-Control no-store\n\
         \t\trequest_body {{\n\
         \t\t\tmax_size 512KB\n\
         \t\t}}\n\
         \t\t# Caddy replaces any X-Forwarded-For a visitor sends with their real\n\
         \t\t# address; the node trusts that header only from loopback, so each\n\
         \t\t# visitor gets their own rate-limit budget.\n\
         \t\treverse_proxy 127.0.0.1:{port}\n\
         \t}}\n\
         \thandle /wallet-api {{\n\
         \t\trespond 405\n\
         \t}}\n\
         \n\
         \thandle {{\n\
         \t\troot * {dir}\n\
                  \t\tfile_server\n\
         \t}}\n\
         }}\n"
    ))
}

/// Write a rendered unit to a file the operator named. Refuses system
/// directories and refuses to replace an existing file.
pub fn write_new_file(path: &Path, content: &str) -> Result<()> {
    if !path.is_absolute()
        && path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        bail!("refusing a path that climbs out with '..'");
    }
    let shown = path.to_string_lossy();
    if SYSTEM_PREFIXES
        .iter()
        .any(|p| shown == *p || shown.starts_with(&format!("{p}/")))
    {
        bail!("refusing to write into {shown}. Write the file somewhere you own, read it, then copy it into place yourself.");
    }
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| {
            format!("{shown} already exists or cannot be created; nothing was overwritten")
        })?;
    f.write_all(content.as_bytes())?;
    Ok(())
}

// ----------------------------------------------------------------- helpers --

/// Whether a process with this pid exists, asked without signalling it
/// (`kill -0` delivers nothing; it only checks). `None` where the question
/// cannot be asked this way.
fn process_alive(pid: u32) -> Option<bool> {
    if pid == 0 {
        return Some(false);
    }
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .ok()
            .map(|s| s.success())
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        None
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn short(h: &str) -> String {
    if h.len() > 12 {
        format!("{}…", &h[..12])
    } else {
        h.to_string()
    }
}

fn human_bytes(n: u64) -> String {
    const U: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

#[cfg(test)]
mod tests;
