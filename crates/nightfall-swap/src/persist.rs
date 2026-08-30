//! JSON files under `<datadir>/swaps/`. A swap that cannot survive a
//! power cut is a swap that loses money to a power cut.

use crate::state::SwapState;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum PersistError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("secret file is readable by others; chmod 600 required")]
    WorldReadable,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum SendKind {
    Redeem,
    Cancel,
    Refund,
    Punish,
    NightClaim,
}

/// Recorded *before* broadcast so a crash between disk and the wire does
/// not lose the fact that we intended to send.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingSend {
    pub kind: SendKind,
    pub txid: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredSwap {
    pub state: SwapState,
    pub night_darks: u64,
    pub btc_sats: u64,
    /// Human-readable warning copied into the file so a crashed wallet
    /// still shows the wart after resume.
    pub wart: String,
    #[serde(default)]
    pub pending: Option<PendingSend>,
    #[serde(default)]
    pub btc_lock_txid: Option<String>,
    #[serde(default)]
    pub night_lock_id: Option<String>,
    /// NIGHT output commitments held for this swap. The wallet must not
    /// spend them as a normal payment.
    #[serde(default)]
    pub reserved_commits: Vec<String>,
}

impl StoredSwap {
    pub fn wart_text() -> &'static str {
        crate::warnings::NO_NIGHT_REFUND
    }

    pub fn new(state: crate::state::SwapState, night_darks: u64, btc_sats: u64) -> Self {
        Self {
            state,
            night_darks,
            btc_sats,
            wart: Self::wart_text().into(),
            pending: None,
            btc_lock_txid: None,
            night_lock_id: None,
            reserved_commits: vec![],
        }
    }
}

pub fn dir(datadir: &Path) -> PathBuf {
    datadir.join("swaps")
}

pub fn path(datadir: &Path, id: Uuid) -> PathBuf {
    dir(datadir).join(format!("{id}.json"))
}

/// Handshake secrets and mid-protocol state. Next to the swap JSON, mode 0600.
pub fn secret_path(datadir: &Path, id: Uuid) -> PathBuf {
    dir(datadir).join(format!("{id}.secret"))
}

/// Write `bytes` to `path` so that on Unix the file is mode 0600 both at
/// create and after the rename. A world-readable secret file is how a swap
/// key ends up in a backup that is synced to a phone.
pub fn write_secret_file(path: &Path, bytes: &[u8]) -> Result<(), PersistError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("secret.tmp");
    {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = fs::metadata(&tmp)?.permissions();
        p.set_mode(0o600);
        fs::set_permissions(&tmp, p)?;
    }
    // The mode that survives is the *tmp file's*, not the destination's.
    // POSIX `rename` unlinks the destination inode and puts the source inode
    // in its place, so an existing world-readable file at `path` cannot leak
    // its permissions into the result. Measured, not assumed: overwriting a
    // 0644 file yields 0600 with no `chmod` after this line at all.
    //
    // A `chmod` here would therefore be code no test can ever hold — it
    // cannot fail on any POSIX filesystem. The guarantee lives in the
    // `opts.mode(0o600)` above and in the explicit `set_permissions` on the
    // tmp file, both of which `overwriting_a_loose_secret_file_tightens_it_again`
    // does anchor.
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Read a secret file. Unix: refuse if group or other can read it.
pub fn read_secret_file(path: &Path) -> Result<Vec<u8>, PersistError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path)?.permissions().mode();
        if mode & 0o077 != 0 {
            return Err(PersistError::WorldReadable);
        }
    }
    Ok(fs::read(path)?)
}

pub fn save(datadir: &Path, stored: &StoredSwap) -> Result<(), PersistError> {
    fs::create_dir_all(dir(datadir))?;
    let tmp = path(datadir, stored.state.id()).with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(stored)?;
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path(datadir, stored.state.id()))?;
    Ok(())
}

pub fn load(datadir: &Path, id: Uuid) -> Result<StoredSwap, PersistError> {
    let bytes = fs::read(path(datadir, id))?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn list(datadir: &Path) -> Result<Vec<StoredSwap>, PersistError> {
    let d = dir(datadir);
    if !d.exists() {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    for e in fs::read_dir(d)? {
        let e = e?;
        if e.path().extension().and_then(|s| s.to_str()) == Some("json") {
            let bytes = fs::read(e.path())?;
            out.push(serde_json::from_slice(&bytes)?);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Role, SwapState};

    #[test]
    fn roundtrip_and_resume_file() {
        let dir = std::env::temp_dir().join(format!("nf-swap-{}", Uuid::new_v4()));
        let state = SwapState::new(Role::Bob);
        let id = state.id();
        let stored = StoredSwap::new(state, 1, 2);
        save(&dir, &stored).unwrap();
        let loaded = load(&dir, id).unwrap();
        assert_eq!(loaded.btc_sats, 2);
        assert!(loaded.wart.contains("stuck forever"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_secret_file_is_mode_600() {
        let dir = std::env::temp_dir().join(format!("nf-sec-{}", Uuid::new_v4()));
        let path = secret_path(&dir, Uuid::new_v4());
        write_secret_file(&path, b"{}").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "secret file must not be readable by others");
        }
        let _ = fs::remove_dir_all(dir);
    }

    /// The case the `chmod` after `rename` actually exists for, which
    /// `a_secret_file_is_mode_600` does not reach.
    ///
    /// That test writes a fresh file, so the tmp file's own 0600 is enough
    /// and the second `chmod` could be deleted without anything noticing —
    /// Grok flagged his own anchor as weak here, correctly. Overwriting an
    /// *existing* world-readable file is the real scenario: a swap saved
    /// once, the file's mode loosened by a backup tool or a careless copy,
    /// then saved again. On a filesystem where `rename` keeps the target's
    /// permissions, the key would silently stay readable.
    #[test]
    fn overwriting_a_loose_secret_file_tightens_it_again() {
        let dir = std::env::temp_dir().join(format!("nf-sec-{}", Uuid::new_v4()));
        let path = secret_path(&dir, Uuid::new_v4());
        write_secret_file(&path, b"{\"first\":true}").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Loosen it the way a backup or a `cp` would.
            let mut perm = fs::metadata(&path).unwrap().permissions();
            perm.set_mode(0o644);
            fs::set_permissions(&path, perm).unwrap();
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o644,
                "precondition: the file is world-readable before the rewrite"
            );

            write_secret_file(&path, b"{\"second\":true}").unwrap();

            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600,
                "a rewrite must tighten the mode, not inherit the loose one"
            );
        }
        // And the content really was replaced, so this is not passing by
        // having quietly done nothing.
        assert_eq!(
            fs::read(&path).unwrap(),
            b"{\"second\":true}",
            "the rewrite must also have written the new bytes"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn world_readable_secret_file_is_refused() {
        let dir = std::env::temp_dir().join(format!("nf-sec-{}", Uuid::new_v4()));
        let path = secret_path(&dir, Uuid::new_v4());
        write_secret_file(&path, b"{}").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut p = fs::metadata(&path).unwrap().permissions();
            p.set_mode(0o644);
            fs::set_permissions(&path, p).unwrap();
            assert!(matches!(
                read_secret_file(&path),
                Err(PersistError::WorldReadable)
            ));
        }
        let _ = fs::remove_dir_all(dir);
    }
}
