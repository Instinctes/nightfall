//! Pre-release native persistence used by the development Core vault UI.
//!
//! Own both the data-directory lock and a per-wallet lock. `seed.vault/` is a
//! permanent no-downgrade marker; `wallet.nfv` is the only committed snapshot.
//! Interrupted encrypted temporary files are never promoted automatically.
//! All mutations are copy -> edit -> encrypt -> sync -> rename -> directory
//! sync -> publish. A transaction must not be broadcast inside an edit closure.
//! A post-rename error makes the session unusable until reopened: the caller
//! must not guess whether the old or new state survived.
//!
//! Currently enabled on Unix only. Windows durability/ACL semantics need a
//! tested implementation before activation there. OS sync calls are not a
//! promise against faulty hardware, malicious same-user processes or rollback.
use crate::vault::{Vault, VaultError, MAX_PLAINTEXT_BYTES, MAX_VAULT_BYTES};
use crate::{Wallet, WalletFile};
use anyhow::{ensure, Context};
use nightfall_crypto::WalletKeys;
use nightfall_storage::dirlock::{self, DirLock};
use nightfall_types::NetworkId;
use rand::{rngs::OsRng, RngCore};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use zeroize::Zeroizing;

const SNAPSHOT: &str = "wallet.nfv";
/// Deliberately not hex: old clients must fail to parse this as a seed instead
/// of generating a replacement when the legacy secret is retired.
const LEGACY_TOMBSTONE: &[u8] = b"NIGHTFALL ENCRYPTED VAULT v1\nThis file is not a seed. Use a vault-compatible wallet. Do not delete this marker or the adjacent .vault directory.\n";

/// Explicit keys-only recovery. No Debug/Clone: the returned wallet owns secrets.
/// Saved chain state and pending sends remain solely in the untouched source.
pub struct RecoveredBackupKeys {
    pub wallet: Wallet,
    pub source_scanned_to: u64,
    pub source_history_entries: usize,
    pub source_pending_sends: usize,
    pub source_reservations: usize,
}

/// Authenticate a bounded regular backup, then recover ONLY its keys into a
/// fresh in-memory wallet at genesis. No filesystem mutation or network access.
/// This is not a full snapshot import and cannot resume the backup's raw sends.
pub fn recover_backup_keys(
    source: &Path,
    password: &str,
    network: NetworkId,
) -> anyhow::Result<RecoveredBackupKeys> {
    ensure!(
        crate::vault_fs::SUPPORTED,
        "Native backup recovery is not available on this platform."
    );
    ensure!(source.is_absolute(), "Use an absolute backup filename.");
    let bytes = read_required(source, MAX_VAULT_BYTES)?;
    let mut vault = Vault::from_bytes(&bytes)?;
    vault.unlock(password, network)?;
    let original = vault.wallet()?;
    Ok(RecoveredBackupKeys {
        wallet: Wallet::in_memory(network, WalletKeys::from_seed(original.keys.seed), 0),
        source_scanned_to: original.db.scanned_to,
        source_history_entries: original.db.history.len(),
        source_pending_sends: original.unconfirmed_sends().len(),
        source_reservations: original.db.reserved.len(),
    })
}

/// Explicit full-state recovery. No Debug/Clone: the returned wallet owns secrets.
///
/// Unlike [`RecoveredBackupKeys`] this carries the backup's scan position,
/// outputs, history and reservations, so recovery does not mean rescanning the
/// chain from genesis.
pub struct RecoveredBackupState {
    pub wallet: Wallet,
    /// Unconfirmed sends carried over, all of them quarantined.
    pub quarantined_sends: usize,
    pub scanned_to: u64,
    pub history_entries: usize,
    pub reservations: usize,
}

/// Authenticate a bounded regular backup, then recover its COMPLETE state into
/// a fresh in-memory wallet. No filesystem mutation or network access; the
/// source file is not altered.
///
/// Every outgoing entry in the backup is quarantined before the wallet is
/// handed back, including confirmed entries that a later reorg could make
/// pending again. The total unresolved count comes back with it: a caller
/// must show the owner what is being carried over.
///
/// The reservations come across unchanged. They lock coins a swap was relying
/// on, and a backup cannot tell whether that swap is still running — releasing
/// them here would be a guess with someone's money.
pub fn recover_backup_state(
    source: &Path,
    password: &str,
    network: NetworkId,
) -> anyhow::Result<RecoveredBackupState> {
    ensure!(
        crate::vault_fs::SUPPORTED,
        "Native backup recovery is not available on this platform."
    );
    ensure!(source.is_absolute(), "Use an absolute backup filename.");
    let bytes = read_required(source, MAX_VAULT_BYTES)?;
    let mut vault = Vault::from_bytes(&bytes)?;
    vault.unlock(password, network)?;
    let original = vault.wallet()?;

    // Round-tripping through the state blob is what `Vault::create` does, and
    // it is deliberate here too: the import parser rejects unknown fields, so a
    // backup written by a future version is refused rather than half-read.
    let state = Zeroizing::new(original.export_state()?);
    let mut wallet = Wallet::import_state(&state)?;
    ensure!(
        wallet.network == network,
        "This backup belongs to a different network."
    );
    ensure!(
        wallet.address() == original.address(),
        "The recovered state does not derive the backup's own address."
    );

    let quarantined_sends = wallet.quarantine_imported_sends()?;
    debug_assert!(wallet.resendable().is_empty());
    Ok(RecoveredBackupState {
        quarantined_sends,
        scanned_to: wallet.scanned_to(),
        history_entries: wallet.history().len(),
        reservations: wallet.db.reserved.len(),
        wallet,
    })
}

/// No Debug/Clone: a store owns both its writer lock and secret session.
pub struct VaultStore {
    root: PathBuf,
    directory: PathBuf,
    seed_name: String,
    network: NetworkId,
    _data_lock: Arc<DirLock>,
    _wallet_lock: DirLock,
    session: Option<Vault>,
    disk: Option<Vec<u8>>,
    unusable: bool,
    #[cfg(test)]
    fault: Option<Checkpoint>,
    #[cfg(test)]
    crash: Option<Checkpoint>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Checkpoint {
    PartialWritten,
    TemporaryWritten,
    FileSynced,
    Renamed,
    DirectorySynced,
    DatabaseRetired,
    SeedRetired,
}

impl VaultStore {
    /// Explicit enrollment/open, not a presence probe. Creating the directory
    /// disables legacy writes immediately and permanently. A failed migration
    /// is resumed here, never by deleting the marker or generating a new seed.
    /// The caller must stop legacy workers before enrollment; the data lock
    /// prevents another cooperating process from writing the original files.
    pub fn open(
        data_lock: Arc<DirLock>,
        seed_name: &str,
        network: NetworkId,
    ) -> anyhow::Result<Self> {
        ensure!(
            crate::vault_fs::SUPPORTED,
            "Native vault persistence is not available on this platform."
        );
        ensure!(
            !seed_name.is_empty()
                && seed_name.len() <= 80
                && seed_name != "."
                && seed_name != ".."
                && seed_name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
            "Invalid wallet filename."
        );
        let root = data_lock
            .path()
            .parent()
            .context("Missing data directory")?
            .canonicalize()?;
        let directory = root.join(format!("{seed_name}.vault"));
        if let Some(meta) = metadata(&directory)? {
            ensure!(
                meta.is_dir() && !crate::vault_fs::is_link_like(&meta),
                "Vault directory is not a real directory."
            );
        } else {
            crate::vault_fs::create_private_dir(&directory)?;
        }
        // Repeat this even after a prior failed directory sync.
        sync_directory(&root)?;
        let lock_path = directory.join(dirlock::LOCK_FILE);
        if metadata(&lock_path)?.is_some() {
            ensure_regular(&lock_path)?;
        }
        let wallet_lock = dirlock::acquire(&directory)?;
        let disk = read_optional(&directory.join(SNAPSHOT), MAX_VAULT_BYTES)?;
        let session = disk.as_deref().map(Vault::from_bytes).transpose()?;
        Ok(Self {
            root,
            directory,
            seed_name: seed_name.into(),
            network,
            _data_lock: data_lock,
            _wallet_lock: wallet_lock,
            session,
            disk,
            unusable: false,
            #[cfg(test)]
            fault: None,
            #[cfg(test)]
            crash: None,
        })
    }

    pub fn has_snapshot(&self) -> bool {
        self.disk.is_some()
    }

    pub fn requires_reopen(&self) -> bool {
        self.unusable
    }

    pub fn is_locked(&self) -> bool {
        self.unusable || self.session.as_ref().is_none_or(Vault::is_locked)
    }

    pub fn needs_legacy_retirement(&self) -> anyhow::Result<bool> {
        let (seed, db, tmp) = self.paths();
        for path in [db, tmp] {
            if metadata(&path)?.is_some() {
                return Ok(true);
            }
        }
        let seed = read_optional(&seed, 1024)?.map(Zeroizing::new);
        Ok(match seed.as_deref() {
            Some(bytes) => bytes.as_slice() != LEGACY_TOMBSTONE,
            None => self.has_snapshot(),
        })
    }

    /// Provision an explicitly created/restored in-memory wallet. Never replace
    /// an existing vault or any legacy file, even if that file is corrupt.
    pub fn initialize(&mut self, wallet: &Wallet, password: &str) -> anyhow::Result<()> {
        self.healthy()?;
        ensure!(
            !self.has_snapshot(),
            "A vault already exists; it will not be replaced."
        );
        ensure!(
            !self.needs_legacy_retirement()?,
            "Legacy wallet found; use explicit migration."
        );
        ensure!(metadata(&self.paths().0)?.is_none(),
            "A retired seed marker exists but its vault is missing. Restore the original encrypted backup; do not create a replacement wallet.");
        ensure!(
            wallet.network == self.network,
            "Wallet belongs to another network."
        );
        self.install(wallet, password)?;
        self.unusable = true;
        write_tombstone(&self.paths().0)?;
        self.unusable = false;
        Ok(())
    }

    /// Strictly import seed and database without modifying either. Missing seed,
    /// invalid JSON and an unfinished legacy save are errors, not empty wallets.
    /// The encrypted copy is reopened and compared in full. Plaintext retirement
    /// is a separate explicit operation requiring the password again.
    pub fn migrate_legacy(&mut self, password: &str) -> anyhow::Result<()> {
        self.healthy()?;
        ensure!(
            !self.has_snapshot(),
            "Encrypted copy already exists; verify and finish migration instead."
        );
        let wallet = self.read_legacy()?;
        self.install(&wallet, password)
    }

    fn install(&mut self, wallet: &Wallet, password: &str) -> anyhow::Result<()> {
        let candidate = Vault::create(wallet, password)?;
        self.persist(candidate.sealed_bytes())?;
        // Read the actual committed file, not the candidate or a temp file.
        let bytes = read_required(&self.directory.join(SNAPSHOT), MAX_VAULT_BYTES)?;
        let mut verified = Vault::from_bytes(&bytes)?;
        verified.unlock(password, self.network)?;
        ensure!(
            same_wallet(wallet, verified.wallet()?)?,
            "Saved vault verification failed; original files preserved."
        );
        verified.forget_unlocked();
        self.session = Some(verified);
        Ok(())
    }

    /// Explicitly retire only the original seed/database, after password entry
    /// and a new full verification. Surviving files are checked before *any*
    /// deletion. Safe to retry after interruption between database retirement
    /// and replacement of the seed with a non-secret, non-hex downgrade marker.
    /// This is logical deletion, not secure erasure of SSDs/snapshots/backups.
    pub fn finish_legacy_retirement(&mut self, password: &str) -> anyhow::Result<()> {
        self.healthy()?;
        self.check_disk()?;
        let (seed_path, db_path, tmp_path) = self.paths();
        ensure!(
            metadata(&tmp_path)?.is_none(),
            "Unfinished legacy save found; preserve and resolve it before migration."
        );
        let bytes = read_required(&self.directory.join(SNAPSHOT), MAX_VAULT_BYTES)?;
        let mut verified = Vault::from_bytes(&bytes)?;
        verified.unlock(password, self.network)?;
        let wallet = verified.wallet()?;
        let seed = read_optional(&seed_path, 1024)?.map(Zeroizing::new);
        let db = read_optional(&db_path, MAX_PLAINTEXT_BYTES)?.map(Zeroizing::new);
        if let Some(seed) = &seed {
            if seed.as_slice() != LEGACY_TOMBSTONE {
                let legacy_keys = Zeroizing::new(parse_seed(seed)?);
                ensure!(
                    legacy_keys.seed == wallet.keys.seed,
                    "Legacy seed differs from the encrypted copy; nothing retired."
                );
            }
        }
        if let Some(db) = &db {
            let parsed: WalletFile = parse_database(db)?;
            ensure!(
                same_database(&parsed, &wallet.db)?,
                "Legacy database differs from the encrypted copy; nothing retired."
            );
        }
        self.unusable = true;
        if db.is_some() {
            fs::remove_file(&db_path)?;
            sync_directory(&self.root)?;
            self.checkpoint(Checkpoint::DatabaseRetired)?;
        }
        if seed
            .as_deref()
            .is_none_or(|bytes| bytes.as_slice() != LEGACY_TOMBSTONE)
        {
            write_tombstone(&seed_path)?;
            self.checkpoint(Checkpoint::SeedRetired)?;
        }
        // Retry the directory flush even when a previous attempt completed the
        // retirement but failed before confirming the directory was durable.
        sync_directory(&self.root)?;
        verified.forget_unlocked();
        self.session = Some(verified);
        self.unusable = false;
        Ok(())
    }

    pub fn unlock(&mut self, password: &str) -> anyhow::Result<()> {
        self.ready()?;
        self.check_disk()?;
        self.session
            .as_mut()
            .context("Vault has not been initialized.")?
            .unlock(password, self.network)?;
        Ok(())
    }

    /// No dirty state is exposed: successful edits already live on disk, so
    /// locking is infallible and does not need another write or password KDF.
    pub fn lock(&mut self) {
        if let Some(session) = &mut self.session {
            session.forget_unlocked();
        }
    }

    pub fn wallet(&self) -> anyhow::Result<&Wallet> {
        self.ready()?;
        Ok(self
            .session
            .as_ref()
            .context("Vault has not been initialized.")?
            .wallet()?)
    }

    /// The closure must only edit this in-memory candidate; no network I/O,
    /// file writes or broadcast. Its result (e.g. a signed transaction) is
    /// released only after durable commit. Errors preserve the prior session.
    pub fn update<T>(
        &mut self,
        edit: impl FnOnce(&mut Wallet) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        self.ready()?;
        let mut candidate = self
            .session
            .as_ref()
            .context("Vault has not been initialized.")?
            .copy_session()?;
        let result = edit(candidate.wallet_mut()?)?;
        ensure!(
            candidate.wallet()?.network == self.network,
            "Cannot change a vault's network."
        );
        ensure!(
            candidate.wallet()?.keys.seed == self.session.as_ref().unwrap().wallet()?.keys.seed
                && candidate.wallet()?.address()
                    == self.session.as_ref().unwrap().wallet()?.address(),
            "Cannot replace a vault's wallet identity."
        );
        candidate.snapshot()?;
        self.persist(candidate.sealed_bytes())?;
        self.session = Some(candidate);
        Ok(result)
    }

    pub fn change_password(&mut self, password: &str) -> anyhow::Result<()> {
        self.ready()?;
        let mut candidate = self
            .session
            .as_ref()
            .context("Vault has not been initialized.")?
            .copy_session()?;
        candidate.change_password(password)?;
        self.persist(candidate.sealed_bytes())?;
        self.session = Some(candidate);
        Ok(())
    }

    /// Export only the committed ciphertext. No overwrite, plaintext staging or
    /// source mutation. A failed/aborted write may leave an incomplete NEW file;
    /// it is never reported as verified. Retry using a different filename.
    pub fn export_backup(&self, destination: &Path, password: &str) -> anyhow::Result<()> {
        self.ready()?;
        let current = self.wallet()?;
        self.check_disk()?;
        ensure!(
            destination.is_absolute(),
            "Use an absolute backup filename."
        );
        let parent = destination
            .parent()
            .context("Missing backup directory")?
            .canonicalize()?;
        ensure!(!parent.starts_with(&self.root),
            "Save backups outside the active wallet data directory, preferably on a separate offline device.");
        let filename = destination.file_name().context("Missing backup filename")?;
        let destination = parent.join(filename);
        let bytes = self
            .disk
            .as_deref()
            .context("No committed vault snapshot")?;
        let mut verified = Vault::from_bytes(bytes)?;
        verified.unlock(password, self.network)?;
        ensure!(
            same_wallet(current, verified.wallet()?)?,
            "Committed vault does not match the active wallet."
        );
        // Password validation and complete state verification precede creation.
        let mut file = crate::vault_fs::create_private_new(&destination).context(
            "Backup not created. Choose a new filename; existing files are never replaced",
        )?;
        (|| -> anyhow::Result<()> {
            let split = bytes.len() / 2;
            file.write_all(&bytes[..split])?;
            self.checkpoint(Checkpoint::PartialWritten)?;
            file.write_all(&bytes[split..])?;
            self.checkpoint(Checkpoint::TemporaryWritten)?;
            file.sync_all()?;
            self.checkpoint(Checkpoint::FileSynced)?;
            drop(file);
            sync_directory(&parent)?;
            self.checkpoint(Checkpoint::DirectorySynced)?;
            let saved = read_required(&destination, MAX_VAULT_BYTES)?;
            ensure!(saved == bytes, "Saved backup differs from the committed snapshot.");
            let mut reopened = Vault::from_bytes(&saved)?;
            reopened.unlock(password, self.network)?;
            ensure!(same_wallet(current, reopened.wallet()?)?, "Saved backup verification failed.");
            self.check_disk()?;
            Ok(())
        })().context("Backup could not be verified. A new file may be incomplete; keep existing backups and retry with a new filename")
    }

    /// Read-only rehearsal, never an import or transaction submission. An older
    /// valid backup of this identity is allowed; `false` means its complete state
    /// differs from today's wallet, NOT that its chain state has been validated.
    pub fn verify_backup(&self, source: &Path, password: &str) -> anyhow::Result<bool> {
        self.ready()?;
        let current = self.wallet()?;
        self.check_disk()?;
        ensure!(source.is_absolute(), "Use an absolute backup filename.");
        let bytes = read_required(source, MAX_VAULT_BYTES)?;
        let mut backup = Vault::from_bytes(&bytes)?;
        backup.unlock(password, self.network)?;
        ensure!(
            backup.wallet()?.address() == current.address(),
            "This valid backup belongs to a different wallet. Nothing was imported."
        );
        let same = same_wallet(current, backup.wallet()?)?;
        self.check_disk()?;
        Ok(same)
    }

    fn healthy(&self) -> anyhow::Result<()> {
        ensure!(
            !self.unusable,
            "Storage commit outcome is uncertain. Close and reopen the vault before continuing."
        );
        Ok(())
    }

    fn ready(&self) -> anyhow::Result<()> {
        self.healthy()?;
        ensure!(
            !self.needs_legacy_retirement()?,
            "Legacy retirement is incomplete; verify it before using the vault."
        );
        Ok(())
    }

    fn check_disk(&self) -> anyhow::Result<()> {
        let current = read_optional(&self.directory.join(SNAPSHOT), MAX_VAULT_BYTES)?;
        ensure!(
            current == self.disk,
            "Vault changed or disappeared on disk; refusing to overwrite it. Reopen to inspect it."
        );
        Ok(())
    }

    fn paths(&self) -> (PathBuf, PathBuf, PathBuf) {
        let seed = self.root.join(&self.seed_name);
        let db = self.root.join(format!("{}.outputs.json", self.seed_name));
        let tmp = db.with_extension("json.tmp");
        (seed, db, tmp)
    }

    fn read_legacy(&self) -> anyhow::Result<Wallet> {
        let (seed, db, tmp) = self.paths();
        ensure!(
            metadata(&tmp)?.is_none(),
            "Unfinished legacy save found; nothing migrated."
        );
        let seed = Zeroizing::new(read_required(&seed, 1024)?);
        let keys = parse_seed(&seed)?;
        let mut wallet = Wallet::in_memory(self.network, keys, 0);
        if let Some(db) = read_optional(&db, MAX_PLAINTEXT_BYTES)?.map(Zeroizing::new) {
            wallet.db = parse_database(&db)?;
        }
        Ok(wallet)
    }

    fn persist(&mut self, bytes: &[u8]) -> anyhow::Result<()> {
        self.healthy()?;
        Vault::from_bytes(bytes)?;
        self.check_disk()?;
        let mut random = [0u8; 16];
        OsRng
            .try_fill_bytes(&mut random)
            .map_err(|_| VaultError::RandomUnavailable)?;
        let temp_path = self
            .directory
            .join(format!("pending-{}.nfv", hex::encode(random)));
        let mut file = crate::vault_fs::create_private_new(&temp_path)?;
        let _cleanup = TemporaryFile(temp_path.clone());
        let split = bytes.len() / 2;
        file.write_all(&bytes[..split])?;
        self.checkpoint(Checkpoint::PartialWritten)?;
        file.write_all(&bytes[split..])?;
        self.checkpoint(Checkpoint::TemporaryWritten)?;
        file.sync_all()?;
        self.checkpoint(Checkpoint::FileSynced)?;
        drop(file);
        // A second check also catches ordinary edits during a slow disk write.
        // The writer locks, not this comparison, serialize cooperating writers.
        self.check_disk()?;
        crate::vault_fs::commit_rename(&temp_path, &self.directory.join(SNAPSHOT))?;
        // Any subsequent failure is an uncertain commit, not safe to retry in
        // the old session. Reopen reads whichever committed snapshot survived.
        self.unusable = true;
        self.checkpoint(Checkpoint::Renamed)?;
        sync_directory(&self.directory)?;
        self.checkpoint(Checkpoint::DirectorySynced)?;
        self.disk = Some(bytes.to_vec());
        self.unusable = false;
        Ok(())
    }

    fn checkpoint(&self, _point: Checkpoint) -> anyhow::Result<()> {
        #[cfg(test)]
        if self.crash == Some(_point) {
            // Test subprocess only. No destructors run; the OS releases locks.
            std::process::exit(73);
        }
        #[cfg(test)]
        if self.fault == Some(_point) {
            anyhow::bail!("Injected storage interruption");
        }
        Ok(())
    }
}

struct TemporaryFile(PathBuf);
impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn write_tombstone(seed_path: &Path) -> anyhow::Result<()> {
    let root = seed_path.parent().context("Missing seed directory")?;
    let mut random = [0u8; 16];
    OsRng
        .try_fill_bytes(&mut random)
        .map_err(|_| VaultError::RandomUnavailable)?;
    let path = root.join(format!("vault-retirement-{}.tmp", hex::encode(random)));
    let mut file = crate::vault_fs::create_private_new(&path)?;
    let _cleanup = TemporaryFile(path.clone());
    file.write_all(LEGACY_TOMBSTONE)?;
    file.sync_all()?;
    drop(file);
    crate::vault_fs::commit_rename(&path, seed_path)?;
    sync_directory(root)?;
    Ok(())
}

fn metadata(path: &Path) -> std::io::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(Some(meta)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn ensure_regular(path: &Path) -> anyhow::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_file() && !crate::vault_fs::is_link_like(&meta),
        "Wallet path is not a regular file."
    );
    // Only Unix can answer this from a path alone. Windows needs an open
    // handle, so there the same refusal lives in `read_required`, where one
    // exists — which is also the stronger place for it, since a name can be
    // repointed between a check and an open and a handle cannot.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(meta.nlink() == 1, "Hard-linked wallet file refused.");
    }
    Ok(())
}

fn read_optional(path: &Path, max: usize) -> anyhow::Result<Option<Vec<u8>>> {
    if metadata(path)?.is_none() {
        return Ok(None);
    }
    read_required(path, max).map(Some)
}

fn read_required(path: &Path, max: usize) -> anyhow::Result<Vec<u8>> {
    ensure_regular(path)?;
    let file = crate::vault_fs::open_read_no_links(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() && meta.len() <= max as u64,
        "Wallet file is invalid or exceeds the size limit."
    );
    // Asked of the open handle, not of the path: between the check above and
    // this one the name could have been pointed elsewhere, and the handle
    // cannot be.
    ensure!(
        crate::vault_fs::link_count(&file)? == 1,
        "Hard-linked wallet file refused."
    );
    // Bounded even if a non-cooperating process grows a file after metadata().
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(max as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= max, "Wallet file exceeds the size limit.");
    Ok(std::mem::take(&mut *bytes))
}

fn parse_seed(bytes: &[u8]) -> anyhow::Result<WalletKeys> {
    let text = std::str::from_utf8(bytes).context("Invalid legacy seed encoding")?;
    let mut seed = Zeroizing::new([0u8; 32]);
    hex::decode_to_slice(text.trim(), &mut *seed).context("Invalid legacy seed")?;
    Ok(WalletKeys::from_seed(*seed))
}

fn parse_database(bytes: &[u8]) -> anyhow::Result<WalletFile> {
    // Reject future/unknown top-level fields as well as malformed JSON. Legacy
    // optional fields retain their established defaults, never required ones.
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Database {
        outputs: Vec<crate::OwnedOutput>,
        scanned_to: u64,
        #[serde(default)]
        history: Vec<crate::HistoryEntry>,
        #[serde(default)]
        birth_height: u64,
        #[serde(default)]
        reserved: Vec<String>,
        /// Which chain `scanned_to` was read from. Absent in every database
        /// written before the anchor existed, which is why it defaults rather
        /// than being required — but it must be listed, because this parser
        /// rejects unknown fields on purpose and a legacy wallet saved by a
        /// current build does contain it.
        #[serde(default)]
        scanned_tip: String,
        /// The till's invoices, for exactly the reason given above: a wallet
        /// saved by a current build carries them, and this parser rejects
        /// unknown fields. Leaving it out made migration fail outright on any
        /// wallet that had used Counter — caught by
        /// `core_reopens_vault_locked_without_starting_a_node`, which is what
        /// that comment was put there to prevent and did not.
        #[serde(default)]
        invoices: Vec<crate::counter::Invoice>,
    }
    let db: Database = serde_json::from_slice(bytes)
        .context("Invalid or unsupported legacy database; original preserved")?;
    Ok(WalletFile {
        outputs: db.outputs,
        scanned_to: db.scanned_to,
        history: db.history,
        birth_height: db.birth_height,
        reserved: db.reserved,
        // Carried across, not recomputed. A database old enough to lack it
        // arrives empty, which correctly means "unknown" — and inventing an
        // anchor from the node's current block at that height would assert
        // exactly the thing the anchor exists to prove.
        scanned_tip: db.scanned_tip,
        // Carried, not dropped. The first version of this line assumed a
        // legacy database predates the till and could not hold invoices —
        // which is wrong: the legacy backend is a *current* build writing a
        // plaintext file, so it holds whatever the shop has entered. Dropping
        // them would have lost a merchant's open invoices at the moment they
        // encrypted their wallet.
        invoices: db.invoices,
    })
}

fn same_wallet(a: &Wallet, b: &Wallet) -> anyhow::Result<bool> {
    let a = Zeroizing::new(a.export_state()?);
    let b = Zeroizing::new(b.export_state()?);
    Ok(*a == *b)
}

fn same_database(a: &WalletFile, b: &WalletFile) -> anyhow::Result<bool> {
    let a = Zeroizing::new(serde_json::to_vec(a)?);
    let b = Zeroizing::new(serde_json::to_vec(b)?);
    Ok(*a == *b)
}

/// Make a just-committed rename durable, as far as the platform allows.
/// See `vault_fs` for what "as far as" means on each one.
fn sync_directory(path: &Path) -> std::io::Result<()> {
    crate::vault_fs::sync_dir_after_commit(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Direction, HistoryEntry};
    use std::fs::OpenOptions;
    // Symbolic links and Unix permission bits. The tests that need them are
    // marked `#[cfg(unix)]` individually rather than the whole module being
    // gated, because everything else here — including every crash-injection
    // test — is about ordering and verification, which Windows must satisfy
    // too. Creating a symlink on Windows needs a privilege an ordinary CI
    // account does not have, so those cases stay Unix-only and say so instead
    // of being skipped invisibly.
    #[cfg(unix)]
    use std::os::unix::fs::{symlink, PermissionsExt};

    const PASSWORD: &str = "public unfunded test password";
    const NEW_PASSWORD: &str = "another public test password";
    const SEED: &str = "core.seed";

    struct Fixture {
        root: PathBuf,
        lock: Arc<DirLock>,
    }

    impl Fixture {
        fn new() -> Self {
            let mut random = [0u8; 16];
            OsRng.fill_bytes(&mut random);
            let root = std::env::temp_dir().join(format!(
                "nightfall-vault-store-test-{}",
                hex::encode(random)
            ));
            fs::create_dir(&root).unwrap();
            let lock = Arc::new(dirlock::acquire(&root).unwrap());
            Self { root, lock }
        }
        fn open(&self) -> VaultStore {
            VaultStore::open(self.lock.clone(), SEED, NetworkId::Mainnet).unwrap()
        }
        fn snapshot(&self) -> PathBuf {
            self.root.join("core.seed.vault/wallet.nfv")
        }
        fn seed(&self) -> PathBuf {
            self.root.join(SEED)
        }
        fn db(&self) -> PathBuf {
            self.root.join("core.seed.outputs.json")
        }
        fn legacy(&self) -> Wallet {
            let wallet = test_wallet();
            fs::write(self.seed(), hex::encode(wallet.keys.seed)).unwrap();
            fs::write(self.db(), serde_json::to_vec_pretty(&wallet.db).unwrap()).unwrap();
            wallet
        }
        fn initialized(&self) -> VaultStore {
            let mut store = self.open();
            store.initialize(&test_wallet(), PASSWORD).unwrap();
            store.unlock(PASSWORD).unwrap();
            store
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            // Only this freshly create_dir-created, randomly named fixture.
            assert!(self
                .root
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("nightfall-vault-store-test-"));
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn test_wallet() -> Wallet {
        // Public, unfunded deterministic seed. Never production data.
        let mut wallet = Wallet::in_memory(NetworkId::Mainnet, WalletKeys::from_seed([0; 32]), 42);
        wallet.db.scanned_to = 75;
        let blind = curve25519_dalek::scalar::Scalar::from(7u64);
        let offset = curve25519_dalek::scalar::Scalar::from(9u64);
        let commit = nightfall_crypto::Commitment::new(12345, &blind);
        wallet.db.outputs.push(crate::OwnedOutput {
            commit,
            value: 12345,
            blind_hex: hex::encode(blind.to_bytes()),
            key_offset_hex: hex::encode(offset.to_bytes()),
            memo: "public output fixture".into(),
            height: 60,
            spent: false,
            is_coinbase: true,
        });
        wallet.db.reserved = vec![commit.to_hex()];
        wallet.db.history.push(HistoryEntry {
            direction: Direction::Sent,
            amount: 12,
            fee: 2,
            memo: "public fixture".into(),
            height: None,
            txid: "cd".repeat(32),
            timestamp: 123,
            spent_commits: vec![[3; 32]],
            raw: Some("public fixture raw transaction".into()),
            quarantined: false,
        });
        wallet
    }

    #[test]
    fn keys_only_backup_recovery_never_imports_pending_state_and_starts_at_genesis() {
        let source = Fixture::new();
        let destination = Fixture::new();
        let mut original = test_wallet();
        // A deserializable public pending fixture: no node or submission here.
        original.db.history[0].raw = Some(
            serde_json::to_string(&nightfall_ledger::Transaction {
                version: nightfall_types::PROTOCOL_VERSION,
                inputs: vec![],
                outputs: vec![],
                kernels: vec![],
            })
            .unwrap(),
        );
        assert_eq!(original.resendable().len(), 1);
        let vault = Vault::create(&original, PASSWORD).unwrap();
        let path = source.root.join("source.nfv");
        fs::write(&path, vault.sealed_bytes()).unwrap();
        let recovered = recover_backup_keys(&path, PASSWORD, NetworkId::Mainnet).unwrap();
        assert_eq!(recovered.wallet.address(), original.address());
        assert_eq!(recovered.source_scanned_to, 75);
        assert_eq!(recovered.source_history_entries, 1);
        assert_eq!(recovered.source_pending_sends, 1);
        assert_eq!(recovered.source_reservations, 1);
        assert_eq!(recovered.wallet.scanned_to(), 0);
        assert!(recovered.wallet.outputs().is_empty() && recovered.wallet.history().is_empty());
        assert!(recovered.wallet.resendable().is_empty());
        assert!(recovered.wallet.db.reserved.is_empty());
        assert!(!destination.root.join("core.seed.vault").exists());
        let mut store = destination.open();
        store.initialize(&recovered.wallet, NEW_PASSWORD).unwrap();
        assert!(store.is_locked());
        drop(store);
        let mut store = destination.open();
        assert!(store.unlock(PASSWORD).is_err());
        store.unlock(NEW_PASSWORD).unwrap();
        assert_eq!(store.wallet().unwrap().address(), original.address());
        assert_eq!(store.wallet().unwrap().scan_from(), 0);
        assert!(store.wallet().unwrap().resendable().is_empty());
        assert!(store.wallet().unwrap().history().is_empty());
        assert_eq!(fs::read(&path).unwrap(), vault.sealed_bytes());
    }

    /// A full-state import keeps the scan position and quarantines old sends.
    ///
    /// Keys-only recovery is safe because it throws the past away and rescans.
    /// Full-state recovery keeps the past, which is the point — and which is
    /// exactly why the unconfirmed sends it carries must not be resumed. The
    /// mark has to survive being sealed into a vault and unlocked again, or the
    /// first sync after the restart puts the old payment back on the wire.
    #[test]
    fn full_state_backup_recovery_keeps_the_scan_position_and_quarantines_old_sends() {
        let source = Fixture::new();
        let destination = Fixture::new();
        let mut original = test_wallet();
        // A deserializable public pending fixture: no node or submission here.
        original.db.history[0].raw = Some(
            serde_json::to_string(&nightfall_ledger::Transaction {
                version: nightfall_types::PROTOCOL_VERSION,
                inputs: vec![],
                outputs: vec![],
                kernels: vec![],
            })
            .unwrap(),
        );
        assert_eq!(original.resendable().len(), 1, "fixture must be resendable");
        let vault = Vault::create(&original, PASSWORD).unwrap();
        let path = source.root.join("source.nfv");
        fs::write(&path, vault.sealed_bytes()).unwrap();

        let recovered = recover_backup_state(&path, PASSWORD, NetworkId::Mainnet).unwrap();
        assert_eq!(recovered.wallet.address(), original.address());
        // The whole point of a full import: no rescan from genesis.
        assert_eq!(recovered.scanned_to, 75);
        assert_eq!(recovered.wallet.scanned_to(), 75);
        assert_eq!(recovered.history_entries, 1);
        assert_eq!(recovered.reservations, 1);
        assert_eq!(
            recovered.wallet.outputs().len(),
            original.outputs().len(),
            "the coins must come across too"
        );
        // …and the part that must never be resumed.
        assert_eq!(recovered.quarantined_sends, 1);
        assert!(
            recovered.wallet.resendable().is_empty(),
            "an imported payment must not be handed to a node"
        );
        assert_eq!(recovered.wallet.quarantined().len(), 1);
        assert_eq!(fs::read(&path).unwrap(), vault.sealed_bytes());

        // Seal it, restart, unlock: the mark has to be in the ciphertext.
        let mut store = destination.open();
        store.initialize(&recovered.wallet, NEW_PASSWORD).unwrap();
        drop(store);
        let mut store = destination.open();
        store.unlock(NEW_PASSWORD).unwrap();
        let restored = store.wallet().unwrap();
        assert_eq!(restored.scan_from(), 75, "still no rescan from genesis");
        assert_eq!(restored.history().len(), 1);
        assert!(
            restored.resendable().is_empty(),
            "quarantine must survive being sealed and unlocked"
        );
        assert_eq!(restored.quarantined().len(), 1);
    }

    #[cfg(unix)] // symlinks / permission bits
    #[test]
    fn full_state_backup_recovery_refuses_wrong_password_network_and_damage() {
        let f = Fixture::new();
        let path = f.root.join("source.nfv");
        let vault = Vault::create(&test_wallet(), PASSWORD).unwrap();
        fs::write(&path, vault.sealed_bytes()).unwrap();
        assert!(recover_backup_state(&path, NEW_PASSWORD, NetworkId::Mainnet).is_err());
        assert!(recover_backup_state(&path, PASSWORD, NetworkId::Devnet).is_err());
        let link = f.root.join("link.nfv");
        symlink(&path, &link).unwrap();
        assert!(recover_backup_state(&link, PASSWORD, NetworkId::Mainnet).is_err());
        assert!(
            recover_backup_state(Path::new("relative.nfv"), PASSWORD, NetworkId::Mainnet).is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), vault.sealed_bytes());
        fs::write(&path, b"broken backup").unwrap();
        assert!(recover_backup_state(&path, PASSWORD, NetworkId::Mainnet).is_err());
        assert!(!f.root.join("core.seed.vault").exists());
        assert_eq!(fs::read(&path).unwrap(), b"broken backup");
    }

    #[test]
    fn repeated_backup_recovery_counts_existing_quarantine_and_marks_confirmed_sends() {
        let source = Fixture::new();
        let mut original = test_wallet();
        original.db.history[0].quarantined = true;
        let mut confirmed = original.db.history[0].clone();
        confirmed.txid = "ef".repeat(32);
        confirmed.height = Some(70);
        confirmed.quarantined = false;
        original.db.history.push(confirmed);
        let vault = Vault::create(&original, PASSWORD).unwrap();
        let path = source.root.join("already-restored.nfv");
        fs::write(&path, vault.sealed_bytes()).unwrap();
        let recovered = recover_backup_state(&path, PASSWORD, NetworkId::Mainnet).unwrap();
        assert_eq!(
            recovered.quarantined_sends, 1,
            "count existing unresolved sends too"
        );
        assert_eq!(recovered.wallet.quarantined().len(), 1);
        assert!(recovered
            .wallet
            .history()
            .iter()
            .all(|entry| entry.quarantined));
        assert!(recovered.wallet.resendable().is_empty());

        let second = Vault::create(&recovered.wallet, NEW_PASSWORD).unwrap();
        let second_path = source.root.join("second-generation.nfv");
        fs::write(&second_path, second.sealed_bytes()).unwrap();
        let again = recover_backup_state(&second_path, NEW_PASSWORD, NetworkId::Mainnet).unwrap();
        assert_eq!(again.quarantined_sends, 1);
        assert!(again.wallet.history().iter().all(|entry| entry.quarantined));
        assert_eq!(fs::read(&path).unwrap(), vault.sealed_bytes());
        assert_eq!(fs::read(&second_path).unwrap(), second.sealed_bytes());
    }

    #[cfg(unix)] // symlinks / permission bits
    #[test]
    fn keys_only_backup_recovery_refuses_wrong_password_network_links_and_damage() {
        let f = Fixture::new();
        let path = f.root.join("source.nfv");
        let vault = Vault::create(&test_wallet(), PASSWORD).unwrap();
        fs::write(&path, vault.sealed_bytes()).unwrap();
        assert!(recover_backup_keys(&path, NEW_PASSWORD, NetworkId::Mainnet).is_err());
        assert!(recover_backup_keys(&path, PASSWORD, NetworkId::Devnet).is_err());
        assert_eq!(fs::read(&path).unwrap(), vault.sealed_bytes());
        let link = f.root.join("link.nfv");
        symlink(&path, &link).unwrap();
        assert!(recover_backup_keys(&link, PASSWORD, NetworkId::Mainnet).is_err());
        fs::write(&path, b"broken backup").unwrap();
        assert!(recover_backup_keys(&path, PASSWORD, NetworkId::Mainnet).is_err());
        assert!(!f.root.join("core.seed.vault").exists());
        assert_eq!(fs::read(&path).unwrap(), b"broken backup");
    }

    #[cfg(unix)] // symlinks / permission bits
    #[test]
    fn encrypted_backup_roundtrip_preserves_full_state_and_old_password() {
        let f = Fixture::new();
        let external = Fixture::new();
        let path = external.root.join("offline.nfv");
        let mut store = f.initialized();
        let before = fs::read(f.snapshot()).unwrap();
        assert!(store.export_backup(&path, NEW_PASSWORD).is_err());
        assert!(!path.exists());
        store.export_backup(&path, PASSWORD).unwrap();
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(store.verify_backup(&path, PASSWORD).unwrap());
        assert!(store.export_backup(&path, PASSWORD).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        store
            .update(|wallet| {
                wallet.db.scanned_to += 1;
                Ok(())
            })
            .unwrap();
        let changed = Zeroizing::new(store.wallet().unwrap().export_state().unwrap());
        assert!(!store.verify_backup(&path, PASSWORD).unwrap());
        assert_eq!(*changed, store.wallet().unwrap().export_state().unwrap());
        store.change_password(NEW_PASSWORD).unwrap();
        assert!(!store.verify_backup(&path, PASSWORD).unwrap());
        assert!(store.verify_backup(&path, NEW_PASSWORD).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        store.lock();
        assert!(store.verify_backup(&path, PASSWORD).is_err());
        assert!(store
            .export_backup(&external.root.join("locked.nfv"), NEW_PASSWORD)
            .is_err());
        assert!(!external.root.join("locked.nfv").exists());
    }

    #[cfg(unix)] // symlinks / permission bits
    #[test]
    fn backup_verification_rejects_wrong_identity_network_links_and_damage() {
        let f = Fixture::new();
        let external = Fixture::new();
        let store = f.initialized();
        let before = fs::read(f.snapshot()).unwrap();
        let path = external.root.join("candidate.nfv");
        for wallet in [
            Wallet::in_memory(NetworkId::Mainnet, WalletKeys::from_seed([1; 32]), 0),
            Wallet::in_memory(NetworkId::Devnet, WalletKeys::from_seed([0; 32]), 0),
        ] {
            let vault = Vault::create(&wallet, PASSWORD).unwrap();
            fs::write(&path, vault.sealed_bytes()).unwrap();
            assert!(store.verify_backup(&path, PASSWORD).is_err());
            assert_eq!(fs::read(&path).unwrap(), vault.sealed_bytes());
        }
        for bytes in [&before[..before.len() / 2], b"not a vault".as_slice()] {
            fs::write(&path, bytes).unwrap();
            assert!(store.verify_backup(&path, PASSWORD).is_err());
        }
        fs::File::create(&path)
            .unwrap()
            .set_len(MAX_VAULT_BYTES as u64 + 1)
            .unwrap();
        assert!(store.verify_backup(&path, PASSWORD).is_err());
        fs::write(&path, &before).unwrap();
        let link = external.root.join("symlink.nfv");
        symlink(&path, &link).unwrap();
        assert!(store.verify_backup(&link, PASSWORD).is_err());
        let hardlink = external.root.join("hardlink.nfv");
        fs::hard_link(&path, &hardlink).unwrap();
        assert!(store.verify_backup(&hardlink, PASSWORD).is_err());
        assert_eq!(fs::read(f.snapshot()).unwrap(), before);
    }

    #[cfg(unix)] // symlinks / permission bits
    #[test]
    fn backup_export_refuses_active_paths_existing_links_and_external_wallet_changes() {
        let f = Fixture::new();
        let external = Fixture::new();
        let store = f.initialized();
        let before = fs::read(f.snapshot()).unwrap();
        assert!(store
            .export_backup(&f.root.join("backup.nfv"), PASSWORD)
            .is_err());
        assert!(!f.root.join("backup.nfv").exists());
        let alias = external.root.join("active-wallet");
        symlink(&f.root, &alias).unwrap();
        assert!(store
            .export_backup(&alias.join("backup.nfv"), PASSWORD)
            .is_err());
        let link = external.root.join("existing-link.nfv");
        symlink(f.snapshot(), &link).unwrap();
        assert!(store.export_backup(&link, PASSWORD).is_err());
        assert_eq!(fs::read(f.snapshot()).unwrap(), before);
        let path = external.root.join("valid.nfv");
        store.export_backup(&path, PASSWORD).unwrap();
        fs::write(f.snapshot(), b"external corruption").unwrap();
        assert!(store.verify_backup(&path, PASSWORD).is_err());
        assert!(store
            .export_backup(&external.root.join("new.nfv"), PASSWORD)
            .is_err());
        assert!(!external.root.join("new.nfv").exists());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read(f.snapshot()).unwrap(), b"external corruption");
    }

    #[test]
    fn interrupted_backup_export_never_replaces_existing_backups_or_mutates_wallet() {
        let f = Fixture::new();
        let external = Fixture::new();
        let mut store = f.initialized();
        let original = fs::read(f.snapshot()).unwrap();
        let good = external.root.join("known-good.nfv");
        store.export_backup(&good, PASSWORD).unwrap();
        for (index, point) in [
            Checkpoint::PartialWritten,
            Checkpoint::TemporaryWritten,
            Checkpoint::FileSynced,
            Checkpoint::DirectorySynced,
        ]
        .into_iter()
        .enumerate()
        {
            let path = external.root.join(format!("interrupted-{index}.nfv"));
            store.fault = Some(point);
            let error = store
                .export_backup(&path, PASSWORD)
                .unwrap_err()
                .to_string();
            assert!(error.contains("could not be verified"));
            assert_eq!(fs::read(f.snapshot()).unwrap(), original);
            assert_eq!(fs::read(&good).unwrap(), original);
            assert!(!store.requires_reopen() && !store.is_locked());
            store.fault = None;
            let partial = fs::read(&path).unwrap();
            assert!(store.export_backup(&path, PASSWORD).is_err());
            assert_eq!(fs::read(&path).unwrap(), partial);
            if point == Checkpoint::PartialWritten {
                assert!(store.verify_backup(&path, PASSWORD).is_err());
            }
        }
        store
            .export_backup(&external.root.join("retry-new-name.nfv"), PASSWORD)
            .unwrap();
    }

    #[test]
    fn migration_preserves_full_state_and_requires_explicit_retirement() {
        let f = Fixture::new();
        let original = f.legacy();
        let seed = fs::read(f.seed()).unwrap();
        let db = fs::read(f.db()).unwrap();
        let mut store = f.open();
        store.migrate_legacy(PASSWORD).unwrap();
        assert!(store.is_locked());
        assert!(store.needs_legacy_retirement().unwrap());
        assert!(store.unlock(PASSWORD).is_err());
        assert!(store.wallet().is_err());
        assert_eq!(fs::read(f.seed()).unwrap(), seed);
        assert_eq!(fs::read(f.db()).unwrap(), db);
        assert!(store.finish_legacy_retirement(NEW_PASSWORD).is_err());
        assert!(f.seed().exists() && f.db().exists());
        store.finish_legacy_retirement(PASSWORD).unwrap();
        assert_eq!(fs::read(f.seed()).unwrap(), LEGACY_TOMBSTONE);
        assert!(!f.db().exists());
        assert!(store.is_locked());
        store.unlock(PASSWORD).unwrap();
        assert!(same_wallet(&original, store.wallet().unwrap()).unwrap());
        store.lock();
        assert!(store.wallet().is_err());
        drop(store);
        let mut reopened = f.open();
        assert!(reopened.is_locked());
        reopened.unlock(PASSWORD).unwrap();
        assert!(same_wallet(&original, reopened.wallet().unwrap()).unwrap());
    }

    /// A merchant's open invoices must survive the day they encrypt their
    /// wallet.
    ///
    /// The legacy database is not an old format left behind by an old build —
    /// it is a *current* build writing a plaintext file, so it carries
    /// whatever the till holds. The first version of the migration reader
    /// assumed otherwise and dropped the invoices on the floor; worse, the
    /// strict field list meant a wallet that had used Counter could not be
    /// migrated at all. Both are pinned here.
    #[test]
    fn migration_carries_the_tills_invoices_across() {
        use crate::counter::Invoice;

        let f = Fixture::new();
        let mut legacy = f.legacy();
        legacy.db.invoices.push(Invoice {
            reference: "A-17".into(),
            amount_darks: Some(500),
            description: "two coffees".into(),
            created_unix: 1_000,
            expires_unix: None,
            closed_note: None,
        });
        // Written to the plaintext file the way a current build writes one —
        // which is the whole point: this file is not an artefact of an old
        // version, it is what an unencrypted wallet saves today.
        fs::write(f.db(), serde_json::to_vec_pretty(&legacy.db).unwrap()).unwrap();
        drop(legacy);

        let mut store = f.open();
        store.migrate_legacy(PASSWORD).unwrap();
        store.finish_legacy_retirement(PASSWORD).unwrap();
        store.unlock(PASSWORD).unwrap();
        let carried = store.wallet().unwrap().invoices();
        assert_eq!(carried.len(), 1, "the till was dropped by the migration");
        assert_eq!(carried[0].reference, "A-17");
        assert_eq!(carried[0].amount_darks, Some(500));
    }

    #[cfg(unix)] // symlinks / permission bits
    #[test]
    fn restart_edits_and_password_rotation_are_persistent() {
        let f = Fixture::new();
        let mut store = f.initialized();
        store
            .update(|wallet| {
                wallet.db.scanned_to = 90;
                wallet.db.history[0].memo = "edited before durable commit".into();
                Ok(())
            })
            .unwrap();
        let before_rotation = fs::read(f.snapshot()).unwrap();
        store.change_password(NEW_PASSWORD).unwrap();
        assert_ne!(fs::read(f.snapshot()).unwrap(), before_rotation);
        drop(store);
        let mut store = f.open();
        assert!(store.unlock(PASSWORD).is_err());
        assert!(store.wallet().is_err());
        store.unlock(NEW_PASSWORD).unwrap();
        assert_eq!(store.wallet().unwrap().scanned_to(), 90);
        assert_eq!(
            store.wallet().unwrap().history()[0].memo,
            "edited before durable commit"
        );
        assert_eq!(
            fs::metadata(f.snapshot()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(f.snapshot().parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[test]
    fn failed_edit_never_publishes_a_result_or_mutated_state() {
        let f = Fixture::new();
        let mut store = f.initialized();
        let disk = fs::read(f.snapshot()).unwrap();
        let result: anyhow::Result<()> = store.update(|wallet| {
            wallet.db.scanned_to = 999;
            anyhow::bail!("test edit failure")
        });
        assert!(result.is_err());
        assert_eq!(store.wallet().unwrap().scanned_to(), 75);
        assert_eq!(fs::read(f.snapshot()).unwrap(), disk);
        assert!(store
            .update(|wallet| {
                wallet.keys = WalletKeys::from_seed([1; 32]);
                Ok(())
            })
            .is_err());
        assert!(store
            .update(|wallet| {
                wallet.network = NetworkId::Testnet;
                Ok(())
            })
            .is_err());
        assert_eq!(fs::read(f.snapshot()).unwrap(), disk);
    }

    #[test]
    fn interrupted_edits_never_return_a_transaction_and_restart_is_unambiguous() {
        for point in [
            Checkpoint::PartialWritten,
            Checkpoint::TemporaryWritten,
            Checkpoint::FileSynced,
            Checkpoint::Renamed,
            Checkpoint::DirectorySynced,
        ] {
            let f = Fixture::new();
            let mut store = f.initialized();
            let before = fs::read(f.snapshot()).unwrap();
            store.fault = Some(point);
            let result = store.update(|wallet| {
                wallet.db.scanned_to = 99;
                Ok("must not broadcast")
            });
            assert!(result.is_err());
            let committed = matches!(point, Checkpoint::Renamed | Checkpoint::DirectorySynced);
            if committed {
                assert!(store.wallet().is_err());
                assert!(store.unlock(PASSWORD).is_err());
                assert!(store.change_password(NEW_PASSWORD).is_err());
            } else {
                assert_eq!(store.wallet().unwrap().scanned_to(), 75);
                assert_eq!(fs::read(f.snapshot()).unwrap(), before);
            }
            drop(store);
            let mut store = f.open();
            store.unlock(PASSWORD).unwrap();
            assert_eq!(
                store.wallet().unwrap().scanned_to(),
                if committed { 99 } else { 75 }
            );
        }
    }

    #[test]
    fn interrupted_initial_migration_keeps_originals_and_can_resume() {
        for point in [
            Checkpoint::PartialWritten,
            Checkpoint::TemporaryWritten,
            Checkpoint::FileSynced,
            Checkpoint::Renamed,
            Checkpoint::DirectorySynced,
        ] {
            let f = Fixture::new();
            let original = f.legacy();
            let seed = fs::read(f.seed()).unwrap();
            let db = fs::read(f.db()).unwrap();
            let mut store = f.open();
            store.fault = Some(point);
            assert!(store.migrate_legacy(PASSWORD).is_err());
            assert_eq!(fs::read(f.seed()).unwrap(), seed);
            assert_eq!(fs::read(f.db()).unwrap(), db);
            assert!(Wallet::open(&f.root, NetworkId::Mainnet, SEED).is_err());
            drop(store);
            let mut store = f.open();
            if !store.has_snapshot() {
                store.migrate_legacy(PASSWORD).unwrap();
            }
            store.finish_legacy_retirement(PASSWORD).unwrap();
            store.unlock(PASSWORD).unwrap();
            assert!(same_wallet(&original, store.wallet().unwrap()).unwrap());
        }
    }

    #[test]
    fn interrupted_retirement_can_resume_after_database_or_seed_retirement() {
        for point in [Checkpoint::DatabaseRetired, Checkpoint::SeedRetired] {
            let f = Fixture::new();
            let original = f.legacy();
            let mut store = f.open();
            store.migrate_legacy(PASSWORD).unwrap();
            let encrypted = fs::read(f.snapshot()).unwrap();
            store.fault = Some(point);
            assert!(store.finish_legacy_retirement(PASSWORD).is_err());
            assert!(store.wallet().is_err());
            assert_eq!(fs::read(f.snapshot()).unwrap(), encrypted);
            drop(store);
            let mut store = f.open();
            store.finish_legacy_retirement(PASSWORD).unwrap();
            store.unlock(PASSWORD).unwrap();
            assert!(same_wallet(&original, store.wallet().unwrap()).unwrap());
        }
    }

    #[test]
    fn changed_legacy_seed_or_database_prevents_all_retirement() {
        for change_seed in [true, false] {
            let f = Fixture::new();
            let _ = f.legacy();
            let mut store = f.open();
            store.migrate_legacy(PASSWORD).unwrap();
            if change_seed {
                fs::write(f.seed(), "11".repeat(32)).unwrap();
            } else {
                fs::write(f.db(), br#"{"outputs":[],"scanned_to":999}"#).unwrap();
            }
            let seed = fs::read(f.seed()).unwrap();
            let db = fs::read(f.db()).unwrap();
            assert!(store.finish_legacy_retirement(PASSWORD).is_err());
            assert_eq!(fs::read(f.seed()).unwrap(), seed);
            assert_eq!(fs::read(f.db()).unwrap(), db);
            assert!(store.unlock(PASSWORD).is_err());
        }
    }

    #[test]
    fn malformed_legacy_json_and_future_fields_are_not_empty_wallets() {
        for contents in [
            "broken",
            "{}",
            r#"{"outputs":[],"scanned_to":1,"future":true}"#,
            r#"{"outputs":[],"scanned_to":1,"scanned_to":2}"#,
        ] {
            let f = Fixture::new();
            let _ = f.legacy();
            fs::write(f.db(), contents).unwrap();
            let mut store = f.open();
            assert!(store.migrate_legacy(PASSWORD).is_err());
            assert!(!f.snapshot().exists());
            assert_eq!(fs::read_to_string(f.db()).unwrap(), contents);
            assert!(f.seed().exists());
        }
    }

    #[test]
    fn legacy_temporary_file_must_not_be_silently_discarded() {
        let f = Fixture::new();
        let _ = f.legacy();
        let tmp = f.db().with_extension("json.tmp");
        fs::write(&tmp, b"interrupted legacy state").unwrap();
        let mut store = f.open();
        assert!(store.migrate_legacy(PASSWORD).is_err());
        assert!(!f.snapshot().exists());
        fs::remove_file(&tmp).unwrap();
        store.migrate_legacy(PASSWORD).unwrap();
        fs::write(&tmp, b"another interrupted legacy state").unwrap();
        assert!(store.finish_legacy_retirement(PASSWORD).is_err());
        assert!(f.seed().exists() && f.db().exists() && tmp.exists());
    }

    #[test]
    fn one_writer_even_when_callers_share_the_data_lock() {
        let f = Fixture::new();
        let store = f.open();
        assert!(VaultStore::open(f.lock.clone(), SEED, NetworkId::Mainnet).is_err());
        assert!(dirlock::acquire(&f.root).is_err());
        drop(store);
        let _store = f.open();
    }

    #[test]
    fn missing_or_changed_snapshot_is_never_overwritten_or_replaced_with_plaintext() {
        for remove in [false, true] {
            let f = Fixture::new();
            let mut store = f.initialized();
            if remove {
                fs::remove_file(f.snapshot()).unwrap();
            } else {
                fs::write(f.snapshot(), b"unsupported or damaged snapshot").unwrap();
            }
            assert!(store
                .update(|wallet| {
                    wallet.db.scanned_to = 999;
                    Ok(())
                })
                .is_err());
            assert!(Wallet::open(&f.root, NetworkId::Mainnet, SEED).is_err());
            assert_eq!(fs::read(f.seed()).unwrap(), LEGACY_TOMBSTONE);
            assert!(!f.db().exists());
            if remove {
                assert!(!f.snapshot().exists());
            } else {
                assert_eq!(
                    fs::read(f.snapshot()).unwrap(),
                    b"unsupported or damaged snapshot"
                );
            }
        }
    }

    #[test]
    fn already_open_legacy_wallet_cannot_save_after_enrollment() {
        let f = Fixture::new();
        let _ = f.legacy();
        let legacy = Wallet::open(&f.root, NetworkId::Mainnet, SEED).unwrap();
        let before = fs::read(f.db()).unwrap();
        let _store = f.open();
        assert!(legacy.save().is_err());
        assert_eq!(fs::read(f.db()).unwrap(), before);
    }

    #[test]
    fn legacy_open_preserves_corrupt_database_instead_of_defaulting() {
        let f = Fixture::new();
        let _ = f.legacy();
        fs::write(f.db(), b"broken JSON").unwrap();
        assert!(Wallet::open(&f.root, NetworkId::Mainnet, SEED).is_err());
        assert_eq!(fs::read(f.db()).unwrap(), b"broken JSON");
    }

    #[cfg(unix)] // symlinks / permission bits
    #[test]
    fn links_and_nonregular_files_are_refused() {
        let f = Fixture::new();
        let _ = f.legacy();
        let target = f.root.join("public-target");
        fs::rename(f.seed(), &target).unwrap();
        symlink(&target, f.seed()).unwrap();
        let mut store = f.open();
        assert!(store.migrate_legacy(PASSWORD).is_err());
        fs::remove_file(f.seed()).unwrap();
        fs::hard_link(&target, f.seed()).unwrap();
        assert!(store.migrate_legacy(PASSWORD).is_err());
        fs::remove_file(f.seed()).unwrap();
        fs::create_dir(f.seed()).unwrap();
        assert!(store.migrate_legacy(PASSWORD).is_err());
        assert!(target.exists());
    }

    #[cfg(unix)] // symlinks / permission bits
    #[test]
    fn linked_vault_directory_and_linked_lock_are_refused() {
        let f = Fixture::new();
        let target = f.root.join("unrelated");
        fs::create_dir(&target).unwrap();
        let dir = f.root.join("core.seed.vault");
        symlink(&target, &dir).unwrap();
        assert!(VaultStore::open(f.lock.clone(), SEED, NetworkId::Mainnet).is_err());
        fs::remove_file(&dir).unwrap();
        fs::create_dir(&dir).unwrap();
        let target_file = target.join("preserve");
        fs::write(&target_file, b"do not truncate").unwrap();
        symlink(&target_file, dir.join(dirlock::LOCK_FILE)).unwrap();
        assert!(VaultStore::open(f.lock.clone(), SEED, NetworkId::Mainnet).is_err());
        assert_eq!(fs::read(target_file).unwrap(), b"do not truncate");
    }

    #[test]
    fn bad_filenames_and_oversized_files_fail_before_migration() {
        let f = Fixture::new();
        for name in ["", ".", "..", "../core.seed", "/core.seed", "a/b", "a\\b"] {
            assert!(VaultStore::open(f.lock.clone(), name, NetworkId::Mainnet).is_err());
        }
        let _ = f.legacy();
        OpenOptions::new()
            .write(true)
            .open(f.db())
            .unwrap()
            .set_len(MAX_PLAINTEXT_BYTES as u64 + 1)
            .unwrap();
        let mut store = f.open();
        assert!(store.migrate_legacy(PASSWORD).is_err());
        assert!(!f.snapshot().exists());
        assert!(f.seed().exists());
    }

    #[test]
    fn password_rotation_failure_keeps_old_password_and_committed_state() {
        let f = Fixture::new();
        let mut store = f.initialized();
        let before = fs::read(f.snapshot()).unwrap();
        store.fault = Some(Checkpoint::FileSynced);
        assert!(store.change_password(NEW_PASSWORD).is_err());
        assert_eq!(fs::read(f.snapshot()).unwrap(), before);
        store.lock();
        assert!(store.unlock(NEW_PASSWORD).is_err());
        store.unlock(PASSWORD).unwrap();
        assert!(same_wallet(&test_wallet(), store.wallet().unwrap()).unwrap());
    }

    #[test]
    fn missing_seed_is_not_generated_and_existing_vault_is_not_reinitialized() {
        let f = Fixture::new();
        let mut store = f.open();
        assert!(store.migrate_legacy(PASSWORD).is_err());
        assert!(!f.seed().exists());
        store.initialize(&test_wallet(), PASSWORD).unwrap();
        let before = fs::read(f.snapshot()).unwrap();
        assert!(store.initialize(&test_wallet(), NEW_PASSWORD).is_err());
        assert_eq!(fs::read(f.snapshot()).unwrap(), before);
    }

    #[test]
    fn orphaned_database_never_causes_a_new_legacy_seed() {
        let f = Fixture::new();
        fs::write(f.db(), br#"{"outputs":[],"scanned_to":75}"#).unwrap();
        let before = fs::read(f.db()).unwrap();
        assert!(Wallet::open(&f.root, NetworkId::Mainnet, SEED).is_err());
        assert!(!f.seed().exists());
        assert_eq!(fs::read(f.db()).unwrap(), before);
    }

    #[test]
    fn wrong_network_and_corrupted_copy_never_retire_legacy_files() {
        let f = Fixture::new();
        let _ = f.legacy();
        let mut store = f.open();
        store.migrate_legacy(PASSWORD).unwrap();
        drop(store);
        let mut other = VaultStore::open(f.lock.clone(), SEED, NetworkId::Testnet).unwrap();
        assert!(other.finish_legacy_retirement(PASSWORD).is_err());
        assert!(f.seed().exists() && f.db().exists());
        drop(other);
        let mut blob = fs::read(f.snapshot()).unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 1;
        fs::write(f.snapshot(), &blob).unwrap();
        let mut store = f.open();
        assert!(store.finish_legacy_retirement(PASSWORD).is_err());
        assert!(f.seed().exists() && f.db().exists());
        assert_eq!(fs::read(f.snapshot()).unwrap(), blob);
    }

    #[test]
    fn nonsecret_seed_marker_prevents_even_legacy_hex_loader_fallback() {
        let f = Fixture::new();
        let _ = f.legacy();
        let mut store = f.open();
        store.migrate_legacy(PASSWORD).unwrap();
        store.finish_legacy_retirement(PASSWORD).unwrap();
        drop(store);
        let marker = fs::read(f.seed()).unwrap();
        assert_eq!(marker, LEGACY_TOMBSTONE);
        // The pre-vault client reads an existing seed as hex and refuses errors.
        assert!(hex::decode(std::str::from_utf8(&marker).unwrap().trim()).is_err());
        // Even without the new directory guard, no new seed may be generated.
        // Preserve the encrypted fixture rather than removing its contents.
        fs::rename(
            f.root.join("core.seed.vault"),
            f.root.join("preserved-public-test-vault"),
        )
        .unwrap();
        assert!(Wallet::open(&f.root, NetworkId::Mainnet, SEED).is_err());
        assert_eq!(fs::read(f.seed()).unwrap(), LEGACY_TOMBSTONE);
        assert!(!f.db().exists());
    }

    #[cfg(unix)] // symlinks / permission bits
    #[test]
    fn dangling_legacy_seed_link_is_not_a_new_wallet() {
        let f = Fixture::new();
        let target = f.root.join("missing-target");
        symlink(&target, f.seed()).unwrap();
        assert!(Wallet::open(&f.root, NetworkId::Mainnet, SEED).is_err());
        assert!(!target.exists());
        let phrase = test_wallet().recovery_phrase();
        assert!(
            Wallet::restore_from_phrase(&f.root, NetworkId::Mainnet, SEED, &phrase, 0).is_err()
        );
        assert!(!target.exists() && !f.db().exists());
    }

    #[test]
    fn missing_snapshot_with_retired_seed_marker_cannot_be_reinitialized() {
        let f = Fixture::new();
        let store = f.initialized();
        drop(store);
        fs::rename(
            f.snapshot(),
            f.root.join("preserved-public-test-snapshot.nfv"),
        )
        .unwrap();
        let mut store = f.open();
        assert!(!store.has_snapshot());
        assert!(store.initialize(&test_wallet(), NEW_PASSWORD).is_err());
        assert!(store.migrate_legacy(PASSWORD).is_err());
        assert_eq!(fs::read(f.seed()).unwrap(), LEGACY_TOMBSTONE);
        assert!(!f.snapshot().exists());
    }

    fn crash_point(name: &str) -> Checkpoint {
        match name {
            "partial" => Checkpoint::PartialWritten,
            "written" => Checkpoint::TemporaryWritten,
            "synced" => Checkpoint::FileSynced,
            "renamed" => Checkpoint::Renamed,
            "directory" => Checkpoint::DirectorySynced,
            "database" => Checkpoint::DatabaseRetired,
            "seed" => Checkpoint::SeedRetired,
            _ => panic!("unknown public crash test case"),
        }
    }

    #[test]
    #[ignore = "Subprocess helper; invoked only by process_crashes_release_locks_and_resume_without_plaintext_fallback"]
    fn crash_child() {
        let root = PathBuf::from(
            std::env::var_os("NIGHTFALL_VAULT_TEST_DIR").expect("isolated test directory required"),
        );
        assert!(root
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("nightfall-vault-store-test-"));
        let name = std::env::var("NIGHTFALL_VAULT_TEST_POINT").unwrap();
        let kind = std::env::var("NIGHTFALL_VAULT_TEST_KIND").unwrap();
        let lock = Arc::new(dirlock::acquire(&root).unwrap());
        let mut store = VaultStore::open(lock, SEED, NetworkId::Mainnet).unwrap();
        if kind == "update" || kind == "retire" {
            store.migrate_legacy(PASSWORD).unwrap();
        }
        if kind == "update" {
            store.finish_legacy_retirement(PASSWORD).unwrap();
            store.unlock(PASSWORD).unwrap();
        }
        store.crash = Some(crash_point(&name));
        match kind.as_str() {
            "migrate" => store.migrate_legacy(PASSWORD).unwrap(),
            "retire" => store.finish_legacy_retirement(PASSWORD).unwrap(),
            "update" => store
                .update(|wallet| {
                    wallet.db.scanned_to = 99;
                    Ok(())
                })
                .unwrap(),
            _ => panic!("unknown test workflow"),
        }
        panic!("child did not stop at its checkpoint");
    }

    #[test]
    fn process_crashes_release_locks_and_resume_without_plaintext_fallback() {
        for (kind, points) in [
            (
                "migrate",
                &["partial", "written", "synced", "renamed", "directory"][..],
            ),
            (
                "update",
                &["partial", "written", "synced", "renamed", "directory"][..],
            ),
            ("retire", &["database", "seed"][..]),
        ] {
            for point in points {
                let f = Fixture::new();
                let root = f.root.join("child");
                fs::create_dir(&root).unwrap();
                let original = test_wallet();
                fs::write(root.join(SEED), hex::encode(original.keys.seed)).unwrap();
                fs::write(
                    root.join("core.seed.outputs.json"),
                    serde_json::to_vec(&original.db).unwrap(),
                )
                .unwrap();
                let child = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "vault_store::tests::crash_child", "--ignored"])
                    .env("NIGHTFALL_VAULT_TEST_DIR", &root)
                    .env("NIGHTFALL_VAULT_TEST_POINT", point)
                    .env("NIGHTFALL_VAULT_TEST_KIND", kind)
                    .output()
                    .unwrap();
                assert_eq!(
                    child.status.code(),
                    Some(73),
                    "child must exit without destructors: {kind}/{point}"
                );
                assert!(Wallet::open(&root, NetworkId::Mainnet, SEED).is_err());
                if kind == "migrate" {
                    assert_eq!(
                        fs::read_to_string(root.join(SEED)).unwrap(),
                        hex::encode(original.keys.seed)
                    );
                    let db =
                        parse_database(&fs::read(root.join("core.seed.outputs.json")).unwrap())
                            .unwrap();
                    assert!(same_database(&original.db, &db).unwrap());
                }
                // Locks must be free even though the child ran no Drop code.
                let lock = Arc::new(dirlock::acquire(&root).unwrap());
                let mut store = VaultStore::open(lock, SEED, NetworkId::Mainnet).unwrap();
                if !store.has_snapshot() {
                    store.migrate_legacy(PASSWORD).unwrap();
                }
                store.finish_legacy_retirement(PASSWORD).unwrap();
                store.unlock(PASSWORD).unwrap();
                let expected_height =
                    if kind == "update" && matches!(*point, "renamed" | "directory") {
                        99
                    } else {
                        75
                    };
                assert_eq!(store.wallet().unwrap().scanned_to(), expected_height);
                assert_eq!(store.wallet().unwrap().address(), original.address());
                // A killed writer may leave ciphertext-only temps. They are not
                // hidden plaintext backups and are never treated as committed.
                for entry in fs::read_dir(root.join("core.seed.vault")).unwrap() {
                    let entry = entry.unwrap();
                    if entry.file_name().to_str().unwrap().starts_with("pending-") {
                        Vault::from_bytes(&fs::read(entry.path()).unwrap()).unwrap();
                    }
                }
            }
        }
    }
}
