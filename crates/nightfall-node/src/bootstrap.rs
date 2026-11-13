//! Fetching the published chain archive, so a new node does not have to pull
//! half a million blocks from peers one page at a time.
//!
//! Nothing here is trusted. The manifest and the archive come from the web,
//! and the SHA-256 in the manifest only proves that the download arrived
//! intact — whoever can replace one can replace the other. What decides
//! whether the blocks are the chain is the node itself, afterwards: the
//! archive is loaded exactly like a chain file copied in by hand
//! (`Chain::replay_foreign_blocks`), which checks it against this build's
//! checkpoint and validates everything past it in full. A wrong archive costs
//! a download and is refused.
//!
//! This talks plain HTTPS to two hosts, so it is never done on the node's own
//! initiative: only when the person running it asks, and not at all when the
//! node is set up to keep its traffic behind a proxy.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Where the current archive is described. The same file the website's chain
/// page reads.
pub const MANIFEST_URL: &str = "https://nightfallcoin.org/chain/bootstrap.json";

/// Where archives live. The manifest names a file; it never gets to name a
/// host.
const ARCHIVE_BASE: &str =
    "https://github.com/Instinctes/nightfall/releases/download/chain-bootstrap/";

/// The file an import directory holds once a download is complete. Named so
/// that `ChainStore` reads the directory as a binary-format datadir.
pub const ARCHIVE_FILE: &str = "blocks.bin";
const PART_FILE: &str = "blocks.bin.part";

const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MIN_ARCHIVE_BYTES: u64 = 1024;
const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub network: String,
    /// Blocks in the archive.
    pub height: u64,
    pub tip: String,
    pub genesis: String,
    pub archive: String,
    pub archive_bytes: u64,
    pub blocks_bin_sha256: String,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        // Per read, not for the whole transfer: the archive takes minutes.
        .timeout_read(Duration::from_secs(45))
        .build()
}

impl Manifest {
    /// Fetch and check the manifest for the mainnet archive.
    pub fn fetch(genesis_hex: &str) -> anyhow::Result<Self> {
        refuse_under_lockdown()?;
        Self::fetch_from(MANIFEST_URL, genesis_hex)
    }

    fn fetch_from(url: &str, genesis_hex: &str) -> anyhow::Result<Self> {
        let response = agent()
            .get(url)
            .call()
            .map_err(|e| anyhow::anyhow!("could not read the archive description: {e}"))?;
        let mut body = Vec::new();
        response
            .into_reader()
            .take(MAX_MANIFEST_BYTES + 1)
            .read_to_end(&mut body)?;
        if body.len() as u64 > MAX_MANIFEST_BYTES {
            anyhow::bail!("the archive description is implausibly large");
        }
        let manifest: Manifest = serde_json::from_slice(&body)
            .map_err(|e| anyhow::anyhow!("the archive description is not valid: {e}"))?;
        manifest.check(genesis_hex)?;
        Ok(manifest)
    }

    /// Refuse a manifest that is for another chain or that asks for anything
    /// other than "one archive file of a sane size from the usual place".
    pub fn check(&self, genesis_hex: &str) -> anyhow::Result<()> {
        if self.network != "mainnet" {
            anyhow::bail!("the archive is for {}, not mainnet", self.network);
        }
        if !self.genesis.eq_ignore_ascii_case(genesis_hex) {
            anyhow::bail!("the archive is for a chain with a different genesis block");
        }
        if !is_archive_name(&self.archive) {
            anyhow::bail!("the archive has an unexpected file name");
        }
        if self.blocks_bin_sha256.len() != 64
            || !self
                .blocks_bin_sha256
                .chars()
                .all(|c| c.is_ascii_hexdigit())
        {
            anyhow::bail!("the archive description carries no usable checksum");
        }
        if !(MIN_ARCHIVE_BYTES..=MAX_ARCHIVE_BYTES).contains(&self.archive_bytes) {
            anyhow::bail!("the archive description states an implausible size");
        }
        Ok(())
    }

    fn url(&self) -> String {
        format!("{ARCHIVE_BASE}{}", self.archive)
    }
}

/// `nightfall-chain-<height>-<yyyy-mm-dd>.bin`, and nothing else: no path
/// separators, no dots beyond the extension, no query string.
fn is_archive_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("nightfall-chain-") else {
        return false;
    };
    let Some(rest) = rest.strip_suffix(".bin") else {
        return false;
    };
    !rest.is_empty() && rest.len() <= 40 && rest.chars().all(|c| c.is_ascii_digit() || c == '-')
}

/// Download the archive named by `manifest` into `dir`, returning the path of
/// the finished file ([`ARCHIVE_FILE`] inside `dir`).
///
/// An interrupted download is continued, not restarted: the bytes already on
/// disk are hashed again and the server is asked for the rest. The file only
/// takes its final name after its length and SHA-256 match the manifest.
///
/// `progress` receives `(bytes so far, total bytes)`. Setting `cancel` stops
/// the transfer at the next chunk and leaves the partial file for next time.
pub fn download(
    manifest: &Manifest,
    dir: &Path,
    cancel: &AtomicBool,
    progress: impl FnMut(u64, u64),
) -> anyhow::Result<PathBuf> {
    refuse_under_lockdown()?;
    manifest.check(&manifest.genesis)?;
    download_from(&manifest.url(), manifest, dir, cancel, progress)
}

/// Whether this node may make the direct HTTPS requests a download needs.
///
/// Not when it is configured to keep its traffic behind a proxy: these
/// requests do not go through it, and a node set up that way must not reveal
/// its address to a web server because someone pressed a button.
pub fn allowed() -> bool {
    !nightfall_p2p::privacy_lockdown_enabled()
}

fn refuse_under_lockdown() -> anyhow::Result<()> {
    if allowed() {
        Ok(())
    } else {
        anyhow::bail!(
            "this node keeps its traffic behind a proxy, and the chain archive is a direct \
             download — it is not fetched in this mode"
        )
    }
}

fn download_from(
    url: &str,
    manifest: &Manifest,
    dir: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> anyhow::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let done = dir.join(ARCHIVE_FILE);
    let part = dir.join(PART_FILE);
    let total = manifest.archive_bytes;

    // A finished file from an earlier attempt is used only if it is this
    // archive. Anything else is stale and removed.
    if done.exists() {
        if file_matches(&done, manifest, cancel)? {
            progress(total, total);
            return Ok(done);
        }
        fs::remove_file(&done)?;
    }

    let mut hasher = Sha256::new();
    let mut have = 0u64;
    if let Ok(mut existing) = File::open(&part) {
        let len = existing.metadata()?.len();
        if len > 0 && len < total {
            have = hash_into(&mut existing, &mut hasher, cancel)?;
        }
        if have != len || len >= total {
            // Longer than the archive, or it changed under us: start over.
            hasher = Sha256::new();
            have = 0;
        }
    }

    let mut request = agent().get(url);
    if have > 0 {
        request = request.set("Range", &format!("bytes={have}-"));
    }
    let response = request
        .call()
        .map_err(|e| anyhow::anyhow!("could not download the chain archive: {e}"))?;
    let resumed = have > 0 && response.status() == 206;
    if !resumed {
        hasher = Sha256::new();
        have = 0;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(&part)?;
    let mut reader = response.into_reader();
    let mut buf = vec![0u8; 1 << 20];
    progress(have, total);
    loop {
        if cancel.load(Ordering::SeqCst) {
            file.sync_all()?;
            anyhow::bail!("download cancelled");
        }
        let n = match reader.read(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                // Keep what arrived: the next attempt continues from here.
                let _ = file.sync_all();
                anyhow::bail!("the download was interrupted after {have} of {total} bytes: {e}");
            }
        };
        if n == 0 {
            break;
        }
        have += n as u64;
        if have > total {
            drop(file);
            let _ = fs::remove_file(&part);
            anyhow::bail!("the server sent more than the archive is supposed to hold");
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])?;
        progress(have, total);
    }
    file.sync_all()?;
    drop(file);

    if have != total {
        // Keep the part: the next attempt continues from here.
        anyhow::bail!("the download stopped early ({have} of {total} bytes)");
    }
    let digest = hex::encode(hasher.finalize());
    if !digest.eq_ignore_ascii_case(&manifest.blocks_bin_sha256) {
        let _ = fs::remove_file(&part);
        anyhow::bail!("the downloaded archive does not match its checksum");
    }
    fs::rename(&part, &done)?;
    Ok(done)
}

fn hash_into(file: &mut File, hasher: &mut Sha256, cancel: &AtomicBool) -> anyhow::Result<u64> {
    let mut buf = vec![0u8; 1 << 20];
    let mut seen = 0u64;
    loop {
        if cancel.load(Ordering::SeqCst) {
            anyhow::bail!("download cancelled");
        }
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(seen);
        }
        hasher.update(&buf[..n]);
        seen += n as u64;
    }
}

fn file_matches(path: &Path, manifest: &Manifest, cancel: &AtomicBool) -> anyhow::Result<bool> {
    let mut file = File::open(path)?;
    if file.metadata()?.len() != manifest.archive_bytes {
        return Ok(false);
    }
    let mut hasher = Sha256::new();
    hash_into(&mut file, &mut hasher, cancel)?;
    Ok(hex::encode(hasher.finalize()).eq_ignore_ascii_case(&manifest.blocks_bin_sha256))
}

/// Remove an import directory and whatever a download left in it.
pub fn discard(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    const GENESIS: &str = "061a052d49607ff8f4b306c75d622ebd230cff4ec3a45a6dffc2f7738d4b20de";

    fn manifest_for(body: &[u8]) -> Manifest {
        Manifest {
            network: "mainnet".into(),
            height: 3,
            tip: "00".repeat(32),
            genesis: GENESIS.into(),
            archive: "nightfall-chain-3-2026-11-12.bin".into(),
            archive_bytes: body.len() as u64,
            blocks_bin_sha256: hex::encode(Sha256::digest(body)),
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "nf-bootstrap-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    /// A one-file HTTP server. `cut_after` makes it drop the connection after
    /// that many body bytes, once, to stand in for a broken transfer. Returns
    /// the URL and a log of the `Range` header of each request.
    fn serve(
        body: Vec<u8>,
        cut_after: Option<usize>,
        honour_range: bool,
    ) -> (String, Arc<std::sync::Mutex<Vec<Option<u64>>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/archive.bin", listener.local_addr().unwrap());
        let ranges = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = Arc::clone(&ranges);
        std::thread::spawn(move || {
            let mut cut = cut_after;
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut range = None;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    let lower = line.to_ascii_lowercase();
                    if let Some(v) = lower.strip_prefix("range: bytes=") {
                        range = v.trim().trim_end_matches('-').parse::<u64>().ok();
                    }
                }
                log.lock().unwrap().push(range);
                let from = if honour_range {
                    range.unwrap_or(0) as usize
                } else {
                    0
                };
                let slice = &body[from.min(body.len())..];
                let head = if honour_range && range.is_some() {
                    format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        slice.len()
                    )
                } else {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        slice.len()
                    )
                };
                let _ = stream.write_all(head.as_bytes());
                match cut.take() {
                    Some(n) => {
                        let _ = stream.write_all(&slice[..n.min(slice.len())]);
                    }
                    None => {
                        let _ = stream.write_all(slice);
                    }
                }
                let _ = stream.flush();
            }
        });
        (url, ranges)
    }

    fn body(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 % 251) as u8).collect()
    }

    #[test]
    fn a_complete_download_is_verified_and_named() {
        let data = body(3_000_000);
        let manifest = manifest_for(&data);
        let (url, _) = serve(data.clone(), None, true);
        let dir = temp_dir("ok");
        let mut last = (0, 0);
        let path = download_from(&url, &manifest, &dir, &AtomicBool::new(false), |a, b| {
            last = (a, b)
        })
        .unwrap();
        assert_eq!(path, dir.join(ARCHIVE_FILE));
        assert_eq!(fs::read(&path).unwrap(), data);
        assert_eq!(last, (data.len() as u64, data.len() as u64));
        assert!(!dir.join(PART_FILE).exists());
        discard(&dir);
    }

    #[test]
    fn a_wrong_checksum_is_refused_and_leaves_nothing_usable() {
        let data = body(200_000);
        let mut manifest = manifest_for(&data);
        manifest.blocks_bin_sha256 = "ab".repeat(32);
        let (url, _) = serve(data, None, true);
        let dir = temp_dir("sha");
        let err =
            download_from(&url, &manifest, &dir, &AtomicBool::new(false), |_, _| {}).unwrap_err();
        assert!(err.to_string().contains("checksum"), "{err}");
        assert!(!dir.join(ARCHIVE_FILE).exists());
        assert!(!dir.join(PART_FILE).exists());
        discard(&dir);
    }

    #[test]
    fn an_interrupted_download_continues_where_it_stopped() {
        let data = body(2_500_000);
        let manifest = manifest_for(&data);
        let (url, ranges) = serve(data.clone(), Some(900_000), true);
        let dir = temp_dir("resume");
        let cancel = AtomicBool::new(false);

        download_from(&url, &manifest, &dir, &cancel, |_, _| {}).unwrap_err();
        let kept = fs::metadata(dir.join(PART_FILE)).unwrap().len();
        assert!(kept > 0 && kept <= 900_000, "kept {kept}");
        assert!(!dir.join(ARCHIVE_FILE).exists());

        let path = download_from(&url, &manifest, &dir, &cancel, |_, _| {}).unwrap();
        assert_eq!(fs::read(&path).unwrap(), data);
        assert_eq!(*ranges.lock().unwrap(), vec![None, Some(kept)]);
        discard(&dir);
    }

    #[test]
    fn a_server_that_ignores_range_restarts_cleanly() {
        let data = body(1_200_000);
        let manifest = manifest_for(&data);
        let (url, _) = serve(data.clone(), Some(500_000), false);
        let dir = temp_dir("norange");
        let cancel = AtomicBool::new(false);
        download_from(&url, &manifest, &dir, &cancel, |_, _| {}).unwrap_err();
        // Second attempt asks for the rest, gets the whole file with a 200,
        // and must not append it to the half it already has.
        let path = download_from(&url, &manifest, &dir, &cancel, |_, _| {}).unwrap();
        assert_eq!(fs::read(&path).unwrap(), data);
        discard(&dir);
    }

    #[test]
    fn a_server_that_sends_too_much_is_cut_off() {
        let data = body(400_000);
        let mut manifest = manifest_for(&data);
        manifest.archive_bytes = 100_000;
        let (url, _) = serve(data, None, true);
        let dir = temp_dir("toolong");
        let err =
            download_from(&url, &manifest, &dir, &AtomicBool::new(false), |_, _| {}).unwrap_err();
        assert!(err.to_string().contains("more than"), "{err}");
        assert!(!dir.join(ARCHIVE_FILE).exists());
        discard(&dir);
    }

    #[test]
    fn cancelling_stops_and_keeps_the_part() {
        let data = body(4_000_000);
        let manifest = manifest_for(&data);
        let (url, _) = serve(data, None, true);
        let dir = temp_dir("cancel");
        let cancel = AtomicBool::new(false);
        let err = download_from(&url, &manifest, &dir, &cancel, |have, _| {
            if have >= 1_000_000 {
                cancel.store(true, Ordering::SeqCst);
            }
        })
        .unwrap_err();
        assert!(err.to_string().contains("cancelled"), "{err}");
        assert!(dir.join(PART_FILE).exists());
        assert!(!dir.join(ARCHIVE_FILE).exists());
        discard(&dir);
    }

    #[test]
    fn a_finished_file_from_before_is_reused_only_if_it_is_this_archive() {
        let data = body(300_000);
        let manifest = manifest_for(&data);
        let dir = temp_dir("reuse");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(ARCHIVE_FILE), &data).unwrap();
        // No server at this address: a matching file needs no request at all.
        let path = download_from(
            "http://127.0.0.1:9/none",
            &manifest,
            &dir,
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert_eq!(fs::read(path).unwrap(), data);

        // The same bytes under a manifest for a different archive are stale.
        let other = body(300_001);
        let other_manifest = manifest_for(&other);
        let (url, _) = serve(other.clone(), None, true);
        let path = download_from(
            &url,
            &other_manifest,
            &dir,
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert_eq!(fs::read(path).unwrap(), other);
        discard(&dir);
    }

    #[test]
    fn the_manifest_cannot_point_anywhere_but_an_archive_file() {
        let good = manifest_for(b"0123456789");
        let mut ok = good.clone();
        ok.archive_bytes = MIN_ARCHIVE_BYTES;
        ok.check(GENESIS).unwrap();
        assert!(ok.url().starts_with(ARCHIVE_BASE));

        for name in [
            "../../etc/passwd",
            "nightfall-chain-1-2026-11-12.bin?x=https://evil.example",
            "https://evil.example/nightfall-chain-1-2026-11-12.bin",
            "nightfall-chain-1/2026.bin",
            "nightfall-chain-.bin",
            "nightfall-chain-1-2026-11-12.json",
            "",
        ] {
            let mut m = ok.clone();
            m.archive = name.into();
            assert!(m.check(GENESIS).is_err(), "{name:?} was accepted");
        }

        let mut m = ok.clone();
        m.network = "devnet".into();
        assert!(m.check(GENESIS).is_err());
        let mut m = ok.clone();
        m.genesis = "11".repeat(32);
        assert!(m.check(GENESIS).is_err());
        let mut m = ok.clone();
        m.blocks_bin_sha256 = "xyz".into();
        assert!(m.check(GENESIS).is_err());
        let mut m = ok.clone();
        m.archive_bytes = MAX_ARCHIVE_BYTES + 1;
        assert!(m.check(GENESIS).is_err());
        let mut m = ok;
        m.archive_bytes = 0;
        assert!(m.check(GENESIS).is_err());
    }

    #[test]
    fn the_published_manifest_format_parses() {
        // The file as make-chain-bootstrap.sh writes it, extra fields and all.
        let text = r#"{
          "network": "mainnet",
          "height": 505591,
          "tip": "5e403f8f48bfd9a278ac0e56c89ebc2e40c8e1764bbdd01d5609248e276d89cb",
          "genesis": "061a052d49607ff8f4b306c75d622ebd230cff4ec3a45a6dffc2f7738d4b20de",
          "taken_utc": "2026-11-12T21:17:18Z",
          "archive": "nightfall-chain-505591-2026-11-12.bin",
          "archive_bytes": 741469627,
          "blocks_bin_sha256": "bb741043a46d5dfedeea00255afec77a0db40cfb39295880e550f8e88a2555f2",
          "verified_on_import": true,
          "note": "Contains blocks only."
        }"#;
        let m: Manifest = serde_json::from_str(text).unwrap();
        m.check(GENESIS).unwrap();
        assert_eq!(
            m.url(),
            "https://github.com/Instinctes/nightfall/releases/download/chain-bootstrap/nightfall-chain-505591-2026-11-12.bin"
        );
    }
}
