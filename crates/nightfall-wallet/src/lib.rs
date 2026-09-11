//! Wallet state: key storage, output scanning, coin selection, spending.
//!
//! Shared by the CLI wallet and the Core GUI so the two can never drift apart.

use anyhow::{bail, Context};
use curve25519_dalek::scalar::Scalar;
use nightfall_consensus::Block;
use nightfall_crypto::{
    scan_candidate, scan_output, Address, Commitment, ScanCandidate, WalletKeys,
};
use nightfall_ledger::{build_transfer, Payment, Spendable, Transaction};
use nightfall_storage::write_secret_file;
use nightfall_types::{Amount, NetworkId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use zeroize::{Zeroize, Zeroizing};

mod receipt;
pub use receipt::{verify_receipt, PaymentReceipt};
/// The payment request: what a payee asks for, as one checkable line.
pub mod amount_input;
pub mod payment_request;
pub mod recovery;
pub mod vault;
/// Per-platform filesystem primitives for the vault adapter. Private: the
/// order of operations belongs to `vault_store`, not to its callers — and it
/// follows `vault_store` off the browser, where there is no filesystem for any
/// of it to mean anything.
#[cfg(not(target_arch = "wasm32"))]
mod vault_fs;
#[cfg(not(target_arch = "wasm32"))]
pub mod vault_store;

/// An encrypted-store directory is a permanent no-downgrade marker, even if
/// provisioning was interrupted before its first snapshot. Errors fail closed.
pub fn vault_required(datadir: &Path, seed_name: &str) -> std::io::Result<bool> {
    match fs::symlink_metadata(datadir.join(format!("{seed_name}.vault"))) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// An output this wallet owns, as persisted to disk.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnedOutput {
    pub commit: Commitment,
    pub value: u64,
    /// Blinding factor, hex-encoded scalar.
    pub blind_hex: String,
    /// One-time key offset, hex-encoded scalar.
    pub key_offset_hex: String,
    pub memo: String,
    pub height: u64,
    pub spent: bool,
    /// Coinbase outputs are subject to a maturity delay before they can be
    /// spent. The GUI shows them separately so a miner is never confused by a
    /// balance they cannot yet use.
    #[serde(default)]
    pub is_coinbase: bool,
}

impl OwnedOutput {
    fn scalar(hex_str: &str) -> anyhow::Result<Scalar> {
        let raw = hex::decode(hex_str).context("bad scalar hex")?;
        if raw.len() != 32 {
            bail!("scalar must be 32 bytes");
        }
        let mut b = [0u8; 32];
        b.copy_from_slice(&raw);
        Option::<Scalar>::from(Scalar::from_canonical_bytes(b))
            .context("non-canonical scalar in wallet file")
    }

    pub fn to_spendable(&self, keys: &WalletKeys) -> anyhow::Result<Spendable> {
        let blind = Self::scalar(&self.blind_hex)?;
        let offset = Self::scalar(&self.key_offset_hex)?;
        Ok(Spendable {
            commit: self.commit,
            value: self.value,
            blind,
            spend_secret: keys.spend_secret() + offset,
        })
    }
}

/// One output from the light-client `scan_feed`.
#[derive(Clone, Debug)]
pub struct LightOutput {
    pub height: u64,
    pub timestamp: u64,
    pub commit: String,
    pub ephemeral_pk: String,
    pub output_pk: String,
    pub view_tag: u8,
    pub payload: String,
    pub coinbase: bool,
}

impl LightOutput {
    fn as_candidate(&self) -> Option<ScanCandidate> {
        let commit = decode32(&self.commit)?;
        let ephemeral_pk = decode32(&self.ephemeral_pk)?;
        let output_pk = decode32(&self.output_pk)?;
        let payload = hex::decode(&self.payload).ok()?;
        Some(ScanCandidate {
            commit: Commitment(commit),
            ephemeral_pk,
            output_pk,
            view_tag: self.view_tag,
            payload,
        })
    }
}

fn decode32(hex_str: &str) -> Option<[u8; 32]> {
    let raw = hex::decode(hex_str).ok()?;
    (raw.len() == 32).then(|| {
        let mut a = [0u8; 32];
        a.copy_from_slice(&raw);
        a
    })
}

fn outputs_share_spent_with_us(spent: &BTreeSet<[u8; 32]>, known: &BTreeSet<[u8; 32]>) -> bool {
    spent.iter().any(|c| known.contains(c))
}

/// Which way value moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Received,
    Sent,
    /// Block subsidy earned by mining.
    Mined,
}

impl Direction {
    pub fn label(self) -> &'static str {
        match self {
            Direction::Received => "Received",
            Direction::Sent => "Sent",
            Direction::Mined => "Mined",
        }
    }
}

/// One line in the activity list.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEntry {
    pub direction: Direction,
    pub amount: u64,
    pub fee: u64,
    pub memo: String,
    /// `None` while the transaction is still in the mempool.
    pub height: Option<u64>,
    /// Local identifier. For a send this is the pre-aggregation txid; for a
    /// receive it is the output commitment. Neither survives on chain as a
    /// transaction — aggregation is the point.
    pub txid: String,
    pub timestamp: u64,
    /// Commitments this entry spent. Used to recognise the send as confirmed
    /// once a block consumes them.
    #[serde(default)]
    pub spent_commits: Vec<[u8; 32]>,
    /// The transaction as it was broadcast, so it can be sent again.
    ///
    /// A new payment is handed to exactly one randomly chosen peer, which is
    /// what stops an observer attributing it to this node. Nothing re-sends it.
    /// Before 0.8.2 the transaction was thrown away the moment it was handed
    /// over, so a single dropped hop meant the payment simply ceased to exist
    /// while the wallet said "pending" for ever. It happened; see
    /// `docs/HISTORY.md`.
    ///
    /// `None` on entries written by older versions. Those cannot be re-sent —
    /// a rescan releases the coins instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    /// This payment came out of a restored backup and must never go back on
    /// the wire by itself.
    ///
    /// A backup is a photograph of the past. A payment that was pending when
    /// it was taken may since have confirmed, expired, or had its coins spent
    /// another way — the backup cannot know which. Re-broadcasting on the
    /// wallet's own initiative would mean acting on that stale photograph, so
    /// every imported outgoing entry is kept out of [`Wallet::resendable`].
    /// This includes currently confirmed entries: a later reorg can make them
    /// pending again, but cannot provide fresh permission to broadcast them.
    ///
    /// This does not freeze the entry: a scan that finds the payment on chain
    /// still confirms it normally. Only the automatic re-send is withheld.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub quarantined: bool,
}

impl HistoryEntry {
    pub fn is_pending(&self) -> bool {
        self.height.is_none()
    }

    /// Pending, imported from a backup, and therefore not acted on alone.
    pub fn needs_owner_decision(&self) -> bool {
        self.quarantined && self.is_pending()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WalletFile {
    outputs: Vec<OwnedOutput>,
    /// Height already scanned, so a re-sync does not restart from genesis.
    scanned_to: u64,
    #[serde(default)]
    history: Vec<HistoryEntry>,
    /// Chain height when this wallet was created, or the height its owner
    /// named when restoring from a phrase.
    ///
    /// A wallet cannot own an output that predates its keys, so scanning below
    /// this height is guaranteed to find nothing. That guarantee is what makes
    /// a mobile wallet feasible at all: there is no index from an address to
    /// its outputs — deliberately — so discovering a payment costs one scalar
    /// multiplication per output on chain. Starting at the tip makes a fresh
    /// wallet's initial scan free; starting at zero makes it grow without
    /// bound.
    ///
    /// Defaults to 0, which is the only safe value for wallets written before
    /// this field existed: they may well hold coins from genesis.
    #[serde(default)]
    birth_height: u64,
    /// Commitments held by an in-progress swap. Must not be selected for a
    /// normal payment — otherwise the lock is spent out from under the swap.
    #[serde(default)]
    reserved: Vec<String>,
    /// Hash of the block at `scanned_to`, as the chain looked when it was read.
    ///
    /// `scanned_to` alone is a height, and a height does not identify a chain.
    /// After a reorg the node's block at that height is a different block, and
    /// an incremental scan resuming there walks a history this wallet never
    /// saw: outputs from the discarded branch stay as spendable balance no node
    /// will accept, and a payment that was undone still reads "confirmed".
    /// `reconcile_with` already repairs all of that, but only on a rescan from
    /// genesis, where absence really means absence. Nothing detected that a
    /// rescan was needed.
    ///
    /// This is the missing half: one hash, compared against the first block of
    /// the next page, which is by construction the block at `scanned_to`.
    ///
    /// Empty means unknown — every wallet written before this field existed,
    /// and a fresh one before its first scan. Historical state without an
    /// anchor requires canonical reconciliation, never blind adoption of the
    /// current block hash. A fresh birth-height-only wallet may start normally.
    #[serde(default)]
    scanned_tip: String,
}

/// Balance split by what the user can actually do with it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Balances {
    /// Spendable right now.
    pub available: u64,
    /// Mined but still inside the coinbase maturity window.
    pub immature: u64,
    /// Outgoing, waiting for a block.
    pub pending_out: u64,
}

impl Balances {
    pub fn total(&self) -> u64 {
        self.available.saturating_add(self.immature)
    }
}

pub struct Wallet {
    pub keys: WalletKeys,
    pub network: NetworkId,
    pub seed_path: PathBuf,
    db_path: PathBuf,
    db: WalletFile,
    /// Desktop/CLI write the output file. The web wallet keeps state in the
    /// browser and must not touch a filesystem that is not there.
    persist: bool,
}

impl Drop for Wallet {
    fn drop(&mut self) {
        self.keys.zeroize();
        for output in &mut self.db.outputs {
            output.blind_hex.zeroize();
            output.key_offset_hex.zeroize();
            output.memo.zeroize();
            output.value.zeroize();
        }
        for entry in &mut self.db.history {
            entry.memo.zeroize();
            entry.amount.zeroize();
            entry.fee.zeroize();
            if let Some(raw) = &mut entry.raw {
                raw.zeroize();
            }
        }
    }
}

/// True when `blocks` is a contiguous history from this wallet's beginning
/// through at least everything it has already seen.
///
/// Reconcile is destructive: anything not in the slice is treated as gone.
/// That is only safe when the slice cannot simply have omitted later blocks.
fn covers_canonical_history(
    blocks: &[Block],
    birth_height: u64,
    scanned_to: u64,
    outputs: &[OwnedOutput],
) -> bool {
    let Some(first) = blocks.first() else {
        return false;
    };
    let start = first.header.height.0;
    if start != 0 && start != birth_height {
        return false;
    }
    for (i, b) in blocks.iter().enumerate() {
        if b.header.height.0 != start + i as u64 {
            return false;
        }
    }
    let last = blocks.last().map(|b| b.header.height.0).unwrap_or(start);
    let max_out = outputs.iter().map(|o| o.height).max().unwrap_or(0);
    last >= scanned_to && last >= max_out
}

impl Wallet {
    /// Open an existing wallet or create a new one, scanning from genesis.
    pub fn open(datadir: &Path, network: NetworkId, seed_name: &str) -> anyhow::Result<Self> {
        Self::open_at(datadir, network, seed_name, None, None)
    }

    /// Create a wallet that begins life at `birth_height`.
    ///
    /// Only meaningful when the wallet is actually new — an existing wallet
    /// keeps the birth height it was created with, because lowering it would
    /// not re-scan and raising it could skip coins already received.
    pub fn create_at_height(
        datadir: &Path,
        network: NetworkId,
        seed_name: &str,
        birth_height: u64,
    ) -> anyhow::Result<Self> {
        Self::open_at(datadir, network, seed_name, None, Some(birth_height))
    }

    /// Restore a wallet from a BIP-39 recovery phrase.
    ///
    /// `birth_height` should be a height the wallet certainly did not exist
    /// before. **Guessing too high silently loses coins**: the scan skips the
    /// blocks that contain them and the balance is simply wrong, with nothing
    /// to indicate why. Guessing too low only costs time. When the owner is
    /// unsure, pass `0`.
    pub fn restore_from_phrase(
        datadir: &Path,
        network: NetworkId,
        seed_name: &str,
        phrase: &str,
        birth_height: u64,
    ) -> anyhow::Result<Self> {
        let keys = WalletKeys::from_mnemonic(phrase)?;
        Self::open_at(datadir, network, seed_name, Some(keys), Some(birth_height))
    }

    fn open_at(
        datadir: &Path,
        network: NetworkId,
        seed_name: &str,
        provided: Option<WalletKeys>,
        birth_height: Option<u64>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !vault_required(datadir, seed_name)?,
            "Encrypted wallet or interrupted vault migration found. Open the vault; plaintext fallback and replacement seeds are disabled."
        );
        fs::create_dir_all(datadir)?;
        let seed_path = datadir.join(seed_name);
        let db_path = datadir.join(format!("{seed_name}.outputs.json"));
        let mut seed_present = match fs::symlink_metadata(&seed_path) {
            Ok(_) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(e.into()),
        };
        let db_present = match fs::symlink_metadata(&db_path) {
            Ok(_) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(e.into()),
        };
        anyhow::ensure!(seed_present || !db_present,
            "Wallet database exists but its seed is missing. Preserve the database and recover the original seed; no replacement seed was created.");
        match fs::symlink_metadata(db_path.with_extension("json.tmp")) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(e.into()),
            Ok(_) => bail!("Unfinished wallet database save found. Preserve both database files and resolve the interrupted save before opening the wallet."),
        }

        if let Some(keys) = provided {
            if seed_present {
                bail!(
                    "{} already exists — refusing to overwrite a seed. \
                     Move it aside first if you really mean to replace it.",
                    seed_path.display()
                );
            }
            write_secret_file(&seed_path, &hex::encode(keys.seed))?;
            seed_present = true;
        }

        let keys = if seed_present {
            let hex_seed = fs::read_to_string(&seed_path)?;
            let bytes = hex::decode(hex_seed.trim()).context("seed file is not hex")?;
            if bytes.len() != 32 {
                bail!("seed must be 32 bytes of hex");
            }
            let mut seed = [0u8; 32];
            seed.copy_from_slice(&bytes);
            // Repair permissions on wallets created by older builds.
            let _ = nightfall_storage::harden_permissions(&seed_path);
            WalletKeys::from_seed(seed)
        } else {
            let keys = WalletKeys::generate();
            write_secret_file(&seed_path, &hex::encode(keys.seed))?;
            keys
        };

        let existed = db_present;
        let mut db: WalletFile = if existed {
            serde_json::from_str(&fs::read_to_string(&db_path)?)
                .context("wallet database is damaged or unsupported; original file preserved")?
        } else {
            WalletFile::default()
        };

        // A birth height applies to a wallet being created, not to one being
        // reopened. Changing it later would either skip blocks that were never
        // scanned or claim to have scanned blocks that were not.
        if !existed {
            if let Some(h) = birth_height {
                db.birth_height = h;
                db.scanned_to = h;
            }
        }

        let wallet = Self {
            keys,
            network,
            seed_path,
            db_path,
            db,
            persist: true,
        };
        if !existed {
            wallet.save()?;
        }
        Ok(wallet)
    }

    pub fn address(&self) -> Address {
        self.keys.address()
    }

    /// The string users share to receive funds.
    pub fn address_string(&self) -> String {
        self.address().encode()
    }

    /// Watch-only credential. Reveals all incoming and outgoing amounts and
    /// memos, but cannot spend.
    pub fn view_key_string(&self) -> String {
        self.keys.view_key().encode()
    }

    pub fn balance(&self) -> Amount {
        let pending = self.pending_input_commits();
        Amount(
            self.db
                .outputs
                .iter()
                .filter(|o| !o.spent)
                .filter(|o| !pending.contains(&o.commit.0))
                .map(|o| o.value)
                .sum(),
        )
    }

    pub fn spendable_count(&self) -> usize {
        let pending = self.pending_input_commits();
        self.db
            .outputs
            .iter()
            .filter(|o| !o.spent && !pending.contains(&o.commit.0))
            .count()
    }

    pub fn outputs(&self) -> &[OwnedOutput] {
        &self.db.outputs
    }

    pub fn scanned_to(&self) -> u64 {
        self.db.scanned_to
    }

    /// Distinguish observations from a fresh wallet's requested birth height.
    /// An old empty scan can also have missed payments on a replacement chain.
    pub fn has_scanned_history(&self) -> bool {
        self.db.scanned_to > self.db.birth_height
            || !self.db.scanned_tip.is_empty()
            || !self.db.outputs.is_empty()
            || !self.db.history.is_empty()
            || !self.db.reserved.is_empty()
    }

    /// Read-only canonical-anchor preflight for native scanning and sending.
    /// Unknown historical provenance is not established by adopting today's
    /// block hash: old wallets need a complete canonical reconciliation first.
    pub fn check_scan_anchor(&self, block: &Block) -> anyhow::Result<()> {
        anyhow::ensure!(
            block.header.height.0 == self.db.scanned_to,
            "The node did not provide the wallet's scan anchor at height {}.",
            self.db.scanned_to
        );
        if self.db.scanned_tip.is_empty() {
            anyhow::ensure!(!self.has_scanned_history(),
                "Wallet scan history has no canonical block anchor. Preserve a backup and rebuild the scan before sending.");
            return Ok(());
        }
        let seen = block.hash().to_hex();
        anyhow::ensure!(seen == self.db.scanned_tip,
            "the chain changed below the scan position: block {} is now {}, this wallet last saw {}. Preserve a backup and reconcile the wallet before sending.",
            self.db.scanned_to, Self::short(&seen), Self::short(&self.db.scanned_tip));
        Ok(())
    }

    /// Height below which this wallet cannot own anything.
    pub fn birth_height(&self) -> u64 {
        self.db.birth_height
    }

    /// The height a sync should start requesting blocks from.
    ///
    /// Never below the birth height even if `scanned_to` somehow is — a
    /// truncated or hand-edited wallet file should cost time, not correctness.
    pub fn scan_from(&self) -> u64 {
        self.db.scanned_to.max(self.db.birth_height)
    }

    /// This wallet's seed as a BIP-39 recovery phrase. Secret — anyone holding
    /// these words holds the funds.
    pub fn recovery_phrase(&self) -> String {
        self.keys.to_mnemonic()
    }

    fn save(&self) -> anyhow::Result<()> {
        if self.persist {
            let datadir = self
                .seed_path
                .parent()
                .context("wallet folder unavailable")?;
            let seed_name = self
                .seed_path
                .file_name()
                .and_then(|s| s.to_str())
                .context("invalid wallet seed filename")?;
            anyhow::ensure!(
                !vault_required(datadir, seed_name)?,
                "Vault migration has started; writing legacy plaintext is disabled."
            );
        }
        if !self.persist {
            return Ok(());
        }
        let tmp = self.db_path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(&self.db)?)?;
        fs::rename(&tmp, &self.db_path)?;
        // The output file contains blinding factors — treat it as secret.
        let _ = nightfall_storage::harden_permissions(&self.db_path);
        Ok(())
    }

    /// Wallet that never writes disk. Used by the browser build.
    pub fn in_memory(network: NetworkId, keys: WalletKeys, birth_height: u64) -> Self {
        Self {
            keys,
            network,
            seed_path: PathBuf::new(),
            db_path: PathBuf::new(),
            db: WalletFile {
                birth_height,
                scanned_to: birth_height,
                ..WalletFile::default()
            },
            persist: false,
        }
    }

    /// Seed + scan database as JSON, for the browser to store.
    pub fn export_state(&self) -> anyhow::Result<String> {
        #[derive(Serialize)]
        struct Export<'a> {
            v: u32,
            network: &'a str,
            seed: &'a str,
            db: &'a WalletFile,
        }
        let mut seed_hex = Zeroizing::new([0u8; 64]);
        hex::encode_to_slice(self.keys.seed, &mut seed_hex[..])?;
        Ok(serde_json::to_string(&Export {
            v: 1,
            network: self.network.as_str(),
            seed: std::str::from_utf8(&seed_hex[..])?,
            db: &self.db,
        })?)
    }

    pub fn import_state(blob: &str) -> anyhow::Result<Self> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Import<'a> {
            v: u32,
            network: &'a str,
            seed: &'a str,
            db: WalletFile,
        }
        if blob.len() > vault::MAX_PLAINTEXT_BYTES {
            bail!("wallet state is too large");
        }
        let state: Import<'_> = serde_json::from_str(blob).context("invalid wallet state")?;
        if state.v != 1 {
            bail!("unsupported wallet state version");
        }
        let network = match state.network {
            "mainnet" => NetworkId::Mainnet,
            "testnet" => NetworkId::Testnet,
            "devnet" => NetworkId::Devnet,
            _ => bail!("unknown wallet network"),
        };
        let mut seed = Zeroizing::new([0u8; 32]);
        hex::decode_to_slice(state.seed, &mut seed[..]).context("seed must be 32 bytes of hex")?;
        let keys = WalletKeys::from_seed(*seed);
        Ok(Self {
            keys,
            network,
            seed_path: PathBuf::new(),
            db_path: PathBuf::new(),
            db: state.db,
            persist: false,
        })
    }

    /// Ingest one light-client `scan_feed` page. Used by phones.
    pub fn ingest_scan_page(
        &mut self,
        outputs: &[LightOutput],
        spent_hex: &[String],
        scanned_to: u64,
    ) -> anyhow::Result<u32> {
        let view = self.keys.view_key();
        let known: BTreeSet<[u8; 32]> = self.db.outputs.iter().map(|o| o.commit.0).collect();
        let mut found = 0u32;
        let mut spent_commits: BTreeSet<[u8; 32]> = BTreeSet::new();
        for h in spent_hex {
            if let Ok(raw) = hex::decode(h) {
                if raw.len() == 32 {
                    let mut c = [0u8; 32];
                    c.copy_from_slice(&raw);
                    spent_commits.insert(c);
                }
            }
        }
        let mut new_history: Vec<HistoryEntry> = Vec::new();
        let pending: Vec<(String, BTreeSet<[u8; 32]>)> = self
            .db
            .history
            .iter()
            .filter(|e| e.is_pending() && e.direction == Direction::Sent)
            .map(|e| (e.txid.clone(), e.spent_commits.iter().copied().collect()))
            .collect();
        let mut confirmed: Vec<(String, u64, u64)> = Vec::new();

        for out in outputs {
            let Some(cand) = out.as_candidate() else {
                continue;
            };
            if spent_commits
                .iter()
                .any(|c| pending.iter().any(|(_, s)| s.contains(c)))
            {
                // fall through — confirmation checked below
            }
            if known.contains(&cand.commit.0) {
                continue;
            }
            if let Some(d) = scan_candidate(&view, &cand) {
                self.db.outputs.push(OwnedOutput {
                    commit: d.commit,
                    value: d.value,
                    blind_hex: hex::encode(d.blind.to_bytes()),
                    key_offset_hex: hex::encode(d.key_offset.to_bytes()),
                    memo: d.memo.clone(),
                    height: out.height,
                    spent: false,
                    is_coinbase: out.coinbase,
                });
                if out.coinbase || !outputs_share_spent_with_us(&spent_commits, &known) {
                    new_history.push(HistoryEntry {
                        direction: if out.coinbase {
                            Direction::Mined
                        } else {
                            Direction::Received
                        },
                        amount: d.value,
                        fee: 0,
                        memo: d.memo,
                        height: Some(out.height),
                        txid: hex::encode(d.commit.0),
                        timestamp: out.timestamp,
                        spent_commits: Vec::new(),
                        raw: None,
                        // Found on chain by this wallet, not read out of a
                        // backup, and incoming besides: nothing to withhold.
                        quarantined: false,
                    });
                }
                found += 1;
            }
        }

        for (txid, commits) in &pending {
            if !commits.is_empty() && commits.iter().all(|c| spent_commits.contains(c)) {
                let h = outputs.iter().map(|o| o.height).max().unwrap_or(scanned_to);
                let ts = outputs.iter().map(|o| o.timestamp).max().unwrap_or(0);
                confirmed.push((txid.clone(), h, ts));
            }
        }

        for o in self.db.outputs.iter_mut() {
            if spent_commits.contains(&o.commit.0) {
                o.spent = true;
            }
        }
        for (txid, height, ts) in confirmed {
            if let Some(e) = self
                .db
                .history
                .iter_mut()
                .find(|e| e.txid == txid && e.height.is_none())
            {
                e.height = Some(height);
                e.timestamp = ts;
            }
        }
        for e in new_history {
            if !self
                .db
                .history
                .iter()
                .any(|x| x.txid == e.txid && x.direction == e.direction)
            {
                self.db.history.push(e);
            }
        }
        self.db.history.sort_by(|a, b| {
            b.height
                .unwrap_or(u64::MAX)
                .cmp(&a.height.unwrap_or(u64::MAX))
        });
        if scanned_to > self.db.scanned_to {
            self.db.scanned_to = scanned_to;
        }
        self.save()?;
        Ok(found)
    }

    /// Scan blocks for outputs belonging to this wallet and mark spent ones.
    ///
    /// Returns how many new outputs were discovered.
    pub fn scan_blocks(&mut self, blocks: &[Block]) -> anyhow::Result<u32> {
        self.check_chain_continuity(blocks)?;
        let view = self.keys.view_key();
        let known: BTreeSet<[u8; 32]> = self.db.outputs.iter().map(|o| o.commit.0).collect();
        let mut found = 0u32;
        let mut spent_commits: BTreeSet<[u8; 32]> = BTreeSet::new();
        let mut highest = self.scan_from();
        let mut new_history: Vec<HistoryEntry> = Vec::new();
        let mut confirmed: Vec<(String, u64, u64)> = Vec::new();

        // Commitments this wallet has spent but not yet seen confirmed, mapped
        // to the pending history entry that is waiting for them. Aggregation
        // removes transaction identity from the chain, so a send is confirmed
        // by observing its *inputs* consumed — not by finding a txid.
        let pending: Vec<(String, BTreeSet<[u8; 32]>)> = self
            .db
            .history
            .iter()
            .filter(|e| e.is_pending() && e.direction == Direction::Sent)
            .map(|e| (e.txid.clone(), e.spent_commits.iter().copied().collect()))
            .collect();

        for block in blocks {
            highest = highest.max(block.header.height.0);
            let body = &block.body;

            for input in &body.inputs {
                spent_commits.insert(input.commit.0);
            }

            // Which of our pending sends did this block settle?
            for (txid, commits) in &pending {
                if !commits.is_empty()
                    && commits.iter().all(|c| spent_commits.contains(c))
                    && !confirmed.iter().any(|(t, _, _)| t == txid)
                {
                    confirmed.push((
                        txid.clone(),
                        block.header.height.0,
                        block.header.timestamp_unix,
                    ));
                }
            }

            // An output we own is *change* if this block also consumed
            // something of ours. Without transaction boundaries that is the
            // best the wallet can determine — and it is enough, because the
            // send is already in the history.
            let block_spent_ours = body.inputs.iter().any(|i| known.contains(&i.commit.0));

            for out in &body.outputs {
                if known.contains(&out.commit.0) {
                    continue;
                }
                if let Some(d) = scan_output(&view, out) {
                    let is_coinbase = out.features.is_coinbase();

                    self.db.outputs.push(OwnedOutput {
                        commit: d.commit,
                        value: d.value,
                        blind_hex: hex::encode(d.blind.to_bytes()),
                        key_offset_hex: hex::encode(d.key_offset.to_bytes()),
                        memo: d.memo.clone(),
                        height: block.header.height.0,
                        spent: false,
                        is_coinbase,
                    });

                    if is_coinbase || !block_spent_ours {
                        new_history.push(HistoryEntry {
                            direction: if is_coinbase {
                                Direction::Mined
                            } else {
                                Direction::Received
                            },
                            amount: d.value,
                            fee: 0,
                            memo: d.memo,
                            height: Some(block.header.height.0),
                            txid: hex::encode(d.commit.0),
                            timestamp: block.header.timestamp_unix,
                            spent_commits: Vec::new(),
                            raw: None,
                            // Found on chain by this wallet, not read out of a
                            // backup, and incoming besides: nothing to withhold.
                            quarantined: false,
                        });
                    }
                    found += 1;
                }
            }
        }

        for o in self.db.outputs.iter_mut() {
            if spent_commits.contains(&o.commit.0) {
                o.spent = true;
            }
        }

        // Everything above only ever adds. That is correct while the chain only
        // grows, and wrong the moment it does not.
        //
        // A reorg replaces blocks that were canonical a second ago. Outputs
        // received in them no longer exist; inputs spent in them are unspent
        // again; sends that were confirmed are back to unconfirmed. None of
        // that was noticed here — a coin received in a discarded block stayed
        // in the wallet as spendable balance that no node would accept, and a
        // payment that was undone still read "confirmed in block N".
        //
        // Absence only means absence when the slice is the whole history this
        // wallet can own. A single 128-block page that happens to start at
        // height 0 is not that — treating it as one dropped every later
        // output, and a sync that stopped after the first page lost them
        // for good.
        if covers_canonical_history(
            blocks,
            self.db.birth_height,
            self.db.scanned_to,
            &self.db.outputs,
        ) {
            self.reconcile_with(blocks, &spent_commits);
        }

        // Promote pending sends to confirmed.
        for (txid, height, ts) in confirmed {
            if let Some(e) = self
                .db
                .history
                .iter_mut()
                .find(|e| e.txid == txid && e.height.is_none())
            {
                e.height = Some(height);
                e.timestamp = ts;
            }
        }

        // Append discoveries, skipping anything already recorded.
        for e in new_history {
            if !self
                .db
                .history
                .iter()
                .any(|x| x.txid == e.txid && x.direction == e.direction && x.amount == e.amount)
            {
                self.db.history.push(e);
            }
        }
        self.db.history.sort_by(|a, b| {
            b.height
                .unwrap_or(u64::MAX)
                .cmp(&a.height.unwrap_or(u64::MAX))
        });

        self.db.scanned_to = highest;
        // Remember which chain that height was on. Written from the block the
        // page actually ended at, not from the node's current tip — those are
        // the same thing only while nothing is moving, and this field exists
        // precisely for when something is.
        if let Some(anchor) = blocks.iter().find(|b| b.header.height.0 == highest) {
            self.db.scanned_tip = anchor.hash().to_hex();
        }
        self.save()?;
        Ok(found)
    }

    /// First twelve characters of a hash, for a message a person will read.
    ///
    /// Not for comparison — the check itself uses the whole hash. A truncated
    /// hash in an error is a courtesy; a truncated hash in a condition is a
    /// collision waiting to be someone's missing balance.
    fn short(hash: &str) -> String {
        hash.chars().take(12).collect::<String>() + "…"
    }

    /// Refuse a page that continues a chain this wallet was never on.
    ///
    /// The page begins at `scan_from()`, so its first block is the block at
    /// `scanned_to` — the very block whose hash was recorded last time. Same
    /// height and a different hash is a reorg below the scan position, and
    /// applying the page would quietly merge two histories.
    ///
    /// This detects; it does not repair. Repair already exists in
    /// `reconcile_with`, and it is only sound from genesis, where absence means
    /// absence. So the honest move is to stop and say a rescan is needed rather
    /// than invent a second, subtly different rewind here.
    fn check_chain_continuity(&self, blocks: &[Block]) -> anyhow::Result<()> {
        if self.db.scanned_tip.is_empty() {
            anyhow::ensure!(
                !self.has_scanned_history()
                    || covers_canonical_history(blocks, self.db.birth_height, self.db.scanned_to, &self.db.outputs),
                "Wallet scan history has no canonical block anchor. An incremental page cannot validate old observations; preserve a backup and rebuild the scan."
            );
            return Ok(());
        }
        let Some(first) = blocks.first() else {
            return Ok(());
        };
        if first.header.height.0 != self.db.scanned_to {
            // Not the overlap block. A caller asking for a different range is
            // not evidence of a reorg, and guessing here would block honest
            // rescans that deliberately start lower.
            return Ok(());
        }
        self.check_scan_anchor(first)
    }

    /// Drop what the chain no longer contains, and un-confirm what it no longer
    /// confirms.
    ///
    /// Only ever called with a chain that starts at genesis, so absence here
    /// really means absence.
    fn reconcile_with(&mut self, blocks: &[Block], spent: &BTreeSet<[u8; 32]>) {
        let on_chain: BTreeSet<[u8; 32]> = blocks
            .iter()
            .flat_map(|b| b.body.outputs.iter())
            .map(|o| o.commit.0)
            .collect();

        // Outputs from blocks that lost a reorg. Keeping them would show a
        // balance no node agrees with, and coin selection would build
        // transactions nobody can accept.
        let before = self.db.outputs.len();
        let dropped: BTreeSet<[u8; 32]> = self
            .db
            .outputs
            .iter()
            .filter(|o| !on_chain.contains(&o.commit.0))
            .map(|o| o.commit.0)
            .collect();
        self.db.outputs.retain(|o| on_chain.contains(&o.commit.0));

        // An input that is no longer consumed is no longer spent. Without this
        // a coin stays invisible after the transaction spending it is undone.
        for o in self.db.outputs.iter_mut() {
            o.spent = spent.contains(&o.commit.0);
        }

        // History for outputs that no longer exist.
        self.db.history.retain(|e| match e.direction {
            Direction::Received | Direction::Mined => hex::decode(&e.txid)
                .ok()
                .and_then(|v| <[u8; 32]>::try_from(v.as_slice()).ok())
                .map(|c| !dropped.contains(&c))
                .unwrap_or(true),
            Direction::Sent => true,
        });

        // A send is confirmed by seeing its inputs consumed. If they are not
        // consumed any more, it is not confirmed any more — it is waiting to be
        // mined again, and saying otherwise is how a payment silently vanishes.
        for e in self
            .db
            .history
            .iter_mut()
            .filter(|e| e.direction == Direction::Sent && e.height.is_some())
        {
            let still_spent =
                !e.spent_commits.is_empty() && e.spent_commits.iter().all(|c| spent.contains(c));
            if !still_spent {
                e.height = None;
            }
        }

        if before != self.db.outputs.len() {
            self.db.scanned_to = blocks.last().map(|b| b.header.height.0).unwrap_or(0);
        }
    }

    /// Sends that were confirmed and are not any more, newest first.
    ///
    /// A caller that still holds the transaction can resubmit these. Nothing
    /// else can: block bodies are aggregated, so a discarded block cannot be
    /// taken apart into the transactions it contained, and the node has no way
    /// to put them back in its own mempool.
    pub fn unconfirmed_sends(&self) -> Vec<&HistoryEntry> {
        self.db
            .history
            .iter()
            .filter(|e| e.direction == Direction::Sent && e.is_pending())
            .collect()
    }

    /// Unconfirmed sends this wallet can put back on the wire, newest first.
    ///
    /// The caller submits them; the wallet only remembers. Entries written
    /// before 0.8.2 have no stored transaction and are skipped — there is
    /// nothing to re-send, and pretending otherwise would be worse than saying
    /// so.
    pub fn resendable(&self) -> Vec<(String, Transaction)> {
        self.db
            .history
            .iter()
            // A payment restored from a backup is never re-broadcast on the
            // wallet's own initiative. See `HistoryEntry::quarantined`.
            .filter(|e| !e.quarantined)
            .filter(|e| e.direction == Direction::Sent && e.is_pending())
            .filter_map(|e| {
                let raw = e.raw.as_ref()?;
                let tx: Transaction = serde_json::from_str(raw).ok()?;
                Some((e.txid.clone(), tx))
            })
            .collect()
    }

    /// Mark every unconfirmed outgoing payment as restored from a backup.
    ///
    /// This only marks entries that are pending now. A full backup import must
    /// use [`Self::quarantine_imported_sends`] instead, because even a confirmed
    /// imported send can become pending after a reorg.
    ///
    /// Unresolved payment inputs remain unavailable independently of the
    /// chain's spent flags. Returns how many entries were newly marked.
    pub fn quarantine_pending_sends(&mut self) -> anyhow::Result<usize> {
        let mut marked = 0;
        for entry in self.db.history.iter_mut() {
            if entry.direction == Direction::Sent && entry.is_pending() && !entry.quarantined {
                entry.quarantined = true;
                marked += 1;
            }
        }
        if marked > 0 {
            self.save()?;
        }
        Ok(marked)
    }

    /// Withhold every imported outgoing payment, including confirmed sends
    /// that a later reorg could make pending again. Never clears an existing
    /// mark. Returns the TOTAL unresolved imported sends, including marks
    /// carried by a backup of a previously restored wallet.
    pub fn quarantine_imported_sends(&mut self) -> anyhow::Result<usize> {
        let mut changed = false;
        for entry in &mut self.db.history {
            if entry.direction == Direction::Sent && !entry.quarantined {
                entry.quarantined = true;
                changed = true;
            }
        }
        if changed {
            self.save()?;
        }
        Ok(self.quarantined().len())
    }

    /// Payments a restored backup left unresolved, for the owner to look at.
    pub fn quarantined(&self) -> Vec<&HistoryEntry> {
        self.db
            .history
            .iter()
            .filter(|e| e.needs_owner_decision())
            .collect()
    }

    /// Balance split into spendable, immature and pending-outgoing.
    ///
    /// `tip_height` and `maturity` come from the node, so the wallet never
    /// guesses whether a coinbase can be spent yet.
    pub fn balances(&self, tip_height: u64, maturity: u64) -> Balances {
        let mut b = Balances::default();
        let pending = self.pending_input_commits();
        for o in self.db.outputs.iter().filter(|o| !o.spent) {
            if self.is_reserved(&o.commit) || pending.contains(&o.commit.0) {
                continue;
            }
            if o.is_coinbase && tip_height < o.height.saturating_add(maturity) {
                b.immature = b.immature.saturating_add(o.value);
            } else {
                b.available = b.available.saturating_add(o.value);
            }
        }
        b.pending_out = self
            .db
            .history
            .iter()
            .filter(|e| e.is_pending() && e.direction == Direction::Sent)
            .map(|e| e.amount.saturating_add(e.fee))
            .sum();
        b
    }

    /// Blocks remaining until the given output matures, if it is immature.
    pub fn blocks_until_mature(
        &self,
        o: &OwnedOutput,
        tip_height: u64,
        maturity: u64,
    ) -> Option<u64> {
        if !o.is_coinbase {
            return None;
        }
        let ready_at = o.height.saturating_add(maturity);
        (tip_height < ready_at).then(|| ready_at - tip_height)
    }

    pub fn history(&self) -> &[HistoryEntry] {
        &self.db.history
    }

    pub fn has_reservations(&self) -> bool {
        !self.db.reserved.is_empty()
    }

    /// Spendable outputs, respecting coinbase maturity.
    pub fn spendable_outputs(&self, tip_height: u64, maturity: u64) -> Vec<&OwnedOutput> {
        let pending = self.pending_input_commits();
        self.db
            .outputs
            .iter()
            .filter(|o| !o.spent)
            .filter(|o| !pending.contains(&o.commit.0))
            .filter(|o| !o.is_coinbase || tip_height >= o.height.saturating_add(maturity))
            .collect()
    }

    /// Pick the fewest outputs that cover `target`, largest first.
    ///
    /// `tip_height`/`maturity` of 0 means "ignore maturity" — used by the CLI,
    /// where the node re-checks anyway.
    pub fn select_coins_at(
        &self,
        target: u64,
        tip_height: u64,
        maturity: u64,
    ) -> anyhow::Result<Vec<Spendable>> {
        let pending = self.pending_input_commits();
        let mut available: Vec<&OwnedOutput> = self
            .db
            .outputs
            .iter()
            .filter(|o| !o.spent)
            .filter(|o| !pending.contains(&o.commit.0))
            .filter(|o| !self.is_reserved(&o.commit))
            .filter(|o| {
                maturity == 0 || !o.is_coinbase || tip_height >= o.height.saturating_add(maturity)
            })
            .collect();
        // Largest first, so a payment consumes as few outputs as it can.
        available.sort_by_key(|o| std::cmp::Reverse(o.value));

        let mut chosen = Vec::new();
        let mut total = 0u64;
        for o in available {
            if total >= target {
                break;
            }
            total = total.saturating_add(o.value);
            chosen.push(o.to_spendable(&self.keys)?);
        }
        if total < target {
            bail!(
                "insufficient funds: have {}, need {}",
                Amount(total),
                Amount(target)
            );
        }
        Ok(chosen)
    }

    pub fn select_coins(&self, target: u64) -> anyhow::Result<Vec<Spendable>> {
        self.select_coins_at(target, 0, 0)
    }

    fn is_reserved(&self, commit: &Commitment) -> bool {
        let h = commit.to_hex();
        self.db.reserved.iter().any(|r| r == &h)
    }

    /// A chain scan can establish that an input is currently unspent, not
    /// that an already signed payment has been cancelled. Keep those inputs
    /// unavailable until the pending entry is actually resolved, whether it
    /// is a live retry or a quarantined backup import.
    fn pending_input_commits(&self) -> BTreeSet<[u8; 32]> {
        self.db
            .history
            .iter()
            .filter(|entry| entry.direction == Direction::Sent && entry.is_pending())
            .flat_map(|entry| entry.spent_commits.iter().copied())
            .collect()
    }

    /// Hold these outputs for a swap. A later ordinary payment will not
    /// select them. Mutation test: drop the filter in `select_coins_at` and
    /// `reserved_output_is_not_spent_as_a_normal_payment` fails.
    pub fn reserve_commits(&mut self, hexes: &[String]) -> anyhow::Result<()> {
        for h in hexes {
            if !self.db.reserved.contains(h) {
                self.db.reserved.push(h.clone());
            }
        }
        self.save()
    }

    pub fn release_commits(&mut self, hexes: &[String]) -> anyhow::Result<()> {
        self.db.reserved.retain(|r| !hexes.contains(r));
        self.save()
    }

    /// Commit hexes of the coins a swap of `target` darks would consume.
    pub fn pick_commit_hexes_at(
        &self,
        target: u64,
        tip_height: u64,
        maturity: u64,
    ) -> anyhow::Result<Vec<String>> {
        Ok(self
            .select_coins_at(target, tip_height, maturity)?
            .into_iter()
            .map(|s| s.commit.to_hex())
            .collect())
    }

    /// Spend specific outputs, including ones currently reserved for a swap.
    // Explicit chain height and maturity keep reserved-output spending on the
    // same validation path as ordinary payments.
    #[allow(clippy::too_many_arguments)]
    pub fn create_payment_from_commits_at(
        &self,
        commits: &[String],
        to: &Address,
        amount: u64,
        fee: u64,
        memo: &str,
        tip_height: u64,
        maturity: u64,
    ) -> anyhow::Result<Transaction> {
        let wanted: BTreeSet<String> = commits.iter().cloned().collect();
        let pending = self.pending_input_commits();
        let mut inputs = Vec::new();
        let mut total = 0u64;
        for o in &self.db.outputs {
            if o.spent || pending.contains(&o.commit.0) || !wanted.contains(&o.commit.to_hex()) {
                continue;
            }
            if maturity != 0 && o.is_coinbase && tip_height < o.height.saturating_add(maturity) {
                bail!("a reserved coinbase is still immature");
            }
            total = total.saturating_add(o.value);
            inputs.push(o.to_spendable(&self.keys)?);
        }
        let target = amount.checked_add(fee).context("amount overflow")?;
        if total < target {
            bail!("reserved outputs cover {total}, need {target}");
        }
        Ok(build_transfer(
            &self.keys,
            &inputs,
            &[Payment {
                to: *to,
                amount,
                memo: memo.to_string(),
            }],
            fee,
            &self.address(),
            0,
            self.network.proof_context(),
        )?)
    }

    #[cfg(test)]
    pub fn test_insert_output(&mut self, o: OwnedOutput) {
        self.db.outputs.push(o);
    }

    /// Build a signed transaction paying `amount` to `to`.
    pub fn create_payment(
        &self,
        to: &Address,
        amount: u64,
        fee: u64,
        memo: &str,
    ) -> anyhow::Result<Transaction> {
        self.create_payment_at(to, amount, fee, memo, 0, 0)
    }

    /// As [`Self::create_payment`], but refuses to select immature coinbase
    /// outputs so the node cannot reject our own transaction.
    pub fn create_payment_at(
        &self,
        to: &Address,
        amount: u64,
        fee: u64,
        memo: &str,
        tip_height: u64,
        maturity: u64,
    ) -> anyhow::Result<Transaction> {
        let target = amount.checked_add(fee).context("amount overflow")?;
        let inputs = self.select_coins_at(target, tip_height, maturity)?;
        let tx = build_transfer(
            &self.keys,
            &inputs,
            &[Payment {
                to: *to,
                amount,
                memo: memo.to_string(),
            }],
            fee,
            &self.address(),
            0,
            self.network.proof_context(),
        )?;
        Ok(tx)
    }

    /// Mark the inputs of a broadcast transaction as spent so they are not
    /// selected again before the next sync, and record it in the history as
    /// pending.
    pub fn mark_pending_spend(&mut self, tx: &Transaction) -> anyhow::Result<()> {
        self.record_send(tx, 0, String::new())
    }

    /// Record a broadcast payment. `amount` is what the recipient gets; the fee
    /// is read from the transaction's kernels.
    pub fn record_send(
        &mut self,
        tx: &Transaction,
        amount: u64,
        memo: String,
    ) -> anyhow::Result<()> {
        self.record_send_at(tx, amount, memo, unix_secs())
    }

    /// Like [`Self::record_send`], but the caller supplies the timestamp.
    ///
    /// `SystemTime::now()` is not implemented on `wasm32-unknown-unknown` and
    /// aborts the browser wallet after the proofs already succeeded.
    pub fn record_send_at(
        &mut self,
        tx: &Transaction,
        amount: u64,
        memo: String,
        timestamp: u64,
    ) -> anyhow::Result<()> {
        let spent: BTreeSet<[u8; 32]> = tx.inputs.iter().map(|i| i.commit.0).collect();
        for o in self.db.outputs.iter_mut() {
            if spent.contains(&o.commit.0) {
                o.spent = true;
            }
        }
        let txid = tx.txid().to_hex();
        if !self.db.history.iter().any(|e| e.txid == txid) {
            self.db.history.insert(
                0,
                HistoryEntry {
                    direction: Direction::Sent,
                    amount,
                    fee: tx.total_fee(),
                    memo,
                    height: None,
                    txid,
                    timestamp,
                    spent_commits: tx.inputs.iter().map(|i| i.commit.0).collect(),
                    raw: serde_json::to_string(tx).ok(),
                    // This wallet is making the payment right now, so it owns
                    // the decision to keep trying until a block takes it.
                    quarantined: false,
                },
            );
        }
        self.save()
    }

    /// Forget everything discovered and rescan.
    ///
    /// Rescans from the birth height, not from genesis: below it there is
    /// provably nothing to find, so scanning there is only a way to spend
    /// time. Pass `0` as the birth height at creation for a wallet that should
    /// always rescan the whole chain.
    pub fn reset_scan(&mut self) -> anyhow::Result<()> {
        let birth = self.db.birth_height;
        self.db = WalletFile {
            birth_height: birth,
            scanned_to: birth,
            ..Default::default()
        };
        self.save()
    }
}

fn unix_secs() -> u64 {
    // wasm32-unknown-unknown has no clock. SystemTime::now() panics there
    // (Safari reports that as "Unreachable code" in build_send).
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nightfall_ledger::{build_coinbase, BlockBody, LedgerState};
    use nightfall_types::{Height, DARKS_PER_NIGHT};

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "nf-wallet-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn new_wallet_writes_a_protected_seed() {
        let d = tmpdir("seed");
        let w = Wallet::open(&d, NetworkId::Devnet, "test.seed").unwrap();
        assert!(w.seed_path.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&w.seed_path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn reopening_recovers_the_same_address() {
        let d = tmpdir("reopen");
        let a = Wallet::open(&d, NetworkId::Devnet, "w.seed")
            .unwrap()
            .address_string();
        let b = Wallet::open(&d, NetworkId::Devnet, "w.seed")
            .unwrap()
            .address_string();
        assert_eq!(a, b);
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_fresh_wallet_starts_at_genesis_unless_told_otherwise() {
        let d = tmpdir("birth-default");
        let w = Wallet::open(&d, NetworkId::Devnet, "w.seed").unwrap();
        assert_eq!(w.birth_height(), 0);
        assert_eq!(w.scan_from(), 0);
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_birth_height_survives_reopening() {
        // The height is written at creation. If it were only held in memory,
        // the second run would rescan the entire chain — which is the exact
        // cost this field exists to avoid.
        let d = tmpdir("birth-persist");
        let created = Wallet::create_at_height(&d, NetworkId::Devnet, "w.seed", 5_000).unwrap();
        assert_eq!(created.birth_height(), 5_000);
        assert_eq!(created.scan_from(), 5_000);
        drop(created);

        let reopened = Wallet::open(&d, NetworkId::Devnet, "w.seed").unwrap();
        assert_eq!(reopened.birth_height(), 5_000);
        assert_eq!(reopened.scan_from(), 5_000);
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn reopening_cannot_move_the_birth_height() {
        // Lowering it would claim blocks were scanned that never were;
        // raising it would skip blocks that may hold coins. Neither is a
        // reopen's business.
        let d = tmpdir("birth-fixed");
        Wallet::create_at_height(&d, NetworkId::Devnet, "w.seed", 900).unwrap();
        let again = Wallet::create_at_height(&d, NetworkId::Devnet, "w.seed", 100).unwrap();
        assert_eq!(again.birth_height(), 900);
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_rescan_returns_to_the_birth_height_not_to_genesis() {
        let d = tmpdir("birth-reset");
        let mut w = Wallet::create_at_height(&d, NetworkId::Devnet, "w.seed", 4_242).unwrap();
        w.reset_scan().unwrap();
        assert_eq!(w.birth_height(), 4_242);
        assert_eq!(w.scan_from(), 4_242);
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_phrase_restores_the_same_wallet() {
        let d1 = tmpdir("phrase-a");
        let original = Wallet::open(&d1, NetworkId::Devnet, "w.seed").unwrap();
        let phrase = original.recovery_phrase();
        let address = original.address_string();

        let d2 = tmpdir("phrase-b");
        let restored =
            Wallet::restore_from_phrase(&d2, NetworkId::Devnet, "w.seed", &phrase, 1_234).unwrap();

        assert_eq!(restored.address_string(), address);
        assert_eq!(restored.birth_height(), 1_234);

        fs::remove_dir_all(&d1).ok();
        fs::remove_dir_all(&d2).ok();
    }

    #[test]
    fn restoring_over_an_existing_seed_is_refused() {
        // Silently overwriting a seed file is how a wallet destroys funds it
        // was trusted with. Refuse and let the caller decide.
        let d = tmpdir("phrase-clobber");
        let existing = Wallet::open(&d, NetworkId::Devnet, "w.seed").unwrap();
        let other = WalletKeys::generate().to_mnemonic();
        let before = existing.address_string();
        drop(existing);

        assert!(Wallet::restore_from_phrase(&d, NetworkId::Devnet, "w.seed", &other, 0).is_err());
        assert_eq!(
            Wallet::open(&d, NetworkId::Devnet, "w.seed")
                .unwrap()
                .address_string(),
            before,
            "the original seed must be untouched"
        );
        fs::remove_dir_all(&d).ok();
    }

    /// An incremental scan must notice that the chain moved under it.
    ///
    /// `reconcile_with` already repairs a reorg, but only on a rescan from
    /// genesis. Nothing detected that the rescan was needed: the wallet
    /// resumed at a height, the node offered a different block at that height,
    /// and the two histories were silently merged — coins from the discarded
    /// branch stayed spendable and undone payments stayed "confirmed".
    #[test]
    fn a_page_from_a_different_chain_is_refused_not_merged() {
        let d = tmpdir("reorg-detect");
        let mut w = Wallet::open(&d, NetworkId::Devnet, "w.seed").unwrap();
        let ctx = NetworkId::Devnet.proof_context();
        let reward = 20 * DARKS_PER_NIGHT;

        let mk = |owner: &Address, height: u64, nonce: u64| {
            let cb = build_coinbase(owner, reward, height, ctx).unwrap();
            let body = BlockBody::aggregate(&[cb]);
            let mut ledger = LedgerState::genesis();
            ledger
                .apply_block(&body, Height(height), reward, ctx)
                .unwrap();
            Block {
                header: nightfall_consensus::BlockHeader {
                    version: nightfall_types::PROTOCOL_VERSION,
                    height: Height(height),
                    prev_hash: nightfall_types::Hash256::ZERO,
                    body_root: body.hash(),
                    utxo_root: ledger.utxo_root(),
                    kernel_sum: ledger.kernel_sum(),
                    timestamp_unix: 1_800_000_000 + height + nonce,
                    difficulty: 1,
                    nonce,
                    reward_darks: reward,
                },
                body,
            }
        };

        let mine = w.address();
        let first = mk(&mine, 7, 1);
        w.scan_blocks(std::slice::from_ref(&first)).unwrap();
        assert_eq!(w.scanned_to(), 7);
        assert_eq!(w.outputs().len(), 1);

        // The next page starts at the scan position, as sync_from_node builds
        // it — but height 7 is a different block now. Same height, different
        // chain.
        let replaced = mk(&mine, 7, 2);
        assert_ne!(replaced.hash(), first.hash(), "fixture must differ");
        let err = w
            .scan_blocks(&[replaced])
            .expect_err("a page from another chain must not be applied");
        let msg = err.to_string();
        assert!(
            msg.contains("chain changed below the scan position"),
            "the refusal must say what happened, got: {msg}"
        );

        // And it must refuse rather than half-apply: nothing moved.
        assert_eq!(w.scanned_to(), 7);
        assert_eq!(w.outputs().len(), 1);

        // The same page on the same chain still works — the check must not be
        // a blanket ban on rescanning the overlap block.
        assert_eq!(w.scan_blocks(&[first]).unwrap(), 0);
    }

    /// A fresh wallet's requested birth position is not historical scan data.
    #[test]
    fn an_unknown_anchor_does_not_block_the_first_scan() {
        let d = tmpdir("reorg-legacy");
        let mut w = Wallet::open(&d, NetworkId::Devnet, "w.seed").unwrap();
        let ctx = NetworkId::Devnet.proof_context();
        let reward = 20 * DARKS_PER_NIGHT;

        // A new wallet requested a starting height but observed no blocks yet.
        w.db.birth_height = 5;
        w.db.scanned_to = 5;
        w.db.scanned_tip = String::new();

        let cb = build_coinbase(&w.address(), reward, 5, ctx).unwrap();
        let body = BlockBody::aggregate(&[cb]);
        let mut ledger = LedgerState::genesis();
        ledger.apply_block(&body, Height(5), reward, ctx).unwrap();
        let block = Block {
            header: nightfall_consensus::BlockHeader {
                version: nightfall_types::PROTOCOL_VERSION,
                height: Height(5),
                prev_hash: nightfall_types::Hash256::ZERO,
                body_root: body.hash(),
                utxo_root: ledger.utxo_root(),
                kernel_sum: ledger.kernel_sum(),
                timestamp_unix: 1_800_000_005,
                difficulty: 1,
                nonce: 5,
                reward_darks: reward,
            },
            body,
        };

        w.scan_blocks(std::slice::from_ref(&block))
            .expect("an unknown anchor must not stop the scan");
        // …and from now on the check is armed.
        assert_eq!(w.db.scanned_tip, block.hash().to_hex());
    }

    #[test]
    fn historical_wallet_without_anchor_cannot_adopt_a_new_chain_incrementally() {
        let network = NetworkId::Devnet;
        let keys = WalletKeys::from_seed([0; 32]);
        let mut chain = nightfall_consensus::Chain::new_fair(network).unwrap();
        for time in 1_800_000_000..1_800_000_003 {
            chain.mine_block(&keys.address(), vec![], time).unwrap();
        }
        let blocks = chain.blocks_from(0, usize::MAX);
        let mut wallet = Wallet::in_memory(network, keys, 0);
        wallet.scan_blocks(&blocks).unwrap();
        assert!(!wallet.outputs().is_empty());
        let mut legacy: serde_json::Value =
            serde_json::from_str(&wallet.export_state().unwrap()).unwrap();
        legacy["db"].as_object_mut().unwrap().remove("scanned_tip");
        let mut restored = Wallet::import_state(&legacy.to_string()).unwrap();
        let before = restored.export_state().unwrap();
        let mut replaced = blocks.last().unwrap().clone();
        replaced.header.nonce += 1;
        assert!(restored.check_scan_anchor(&replaced).is_err());
        assert!(restored.scan_blocks(&[replaced]).is_err());
        assert_eq!(restored.export_state().unwrap(), before);
        // Even an empty historical scan must not assume earlier replacement
        // blocks contain no newly received funds.
        restored.db.outputs.clear();
        restored.db.history.clear();
        assert!(restored.has_scanned_history());
        assert!(restored.scan_blocks(&blocks[blocks.len() - 1..]).is_err());
        // Complete canonical history may establish provenance and re-add data.
        restored.scan_blocks(&blocks).unwrap();
        restored.check_scan_anchor(blocks.last().unwrap()).unwrap();
    }

    /// Reading the chain in pages must land in the same place as one big read.
    ///
    /// `sync_from_node` used to ask for every block at once, which copies the
    /// whole chain into memory before scanning a single output. It now walks
    /// pages, and each page restarts at the scan position — so the block that
    /// ended the previous page is handed to the wallet a second time. That
    /// overlap is what arms the reorg anchor, and it must cost nothing: the
    /// coin in it may not be counted twice, and the balance may not drift.
    #[test]
    fn scanning_in_overlapping_pages_matches_scanning_all_at_once() {
        let ctx = NetworkId::Devnet.proof_context();
        let reward = 20 * DARKS_PER_NIGHT;

        let whole = tmpdir("page-whole");
        let paged = tmpdir("page-paged");
        let a = Wallet::open(&whole, NetworkId::Devnet, "w.seed").unwrap();
        let phrase = a.recovery_phrase();
        let mut a = a;
        // The same wallet twice, so both see the same coins.
        let mut b =
            Wallet::restore_from_phrase(&paged, NetworkId::Devnet, "w.seed", &phrase, 0).unwrap();
        assert_eq!(a.address_string(), b.address_string());

        let chain: Vec<Block> = (0..10u64)
            .map(|height| {
                let cb = build_coinbase(&a.address(), reward, height, ctx).unwrap();
                let body = BlockBody::aggregate(&[cb]);
                let mut ledger = LedgerState::genesis();
                ledger
                    .apply_block(&body, Height(height), reward, ctx)
                    .unwrap();
                Block {
                    header: nightfall_consensus::BlockHeader {
                        version: nightfall_types::PROTOCOL_VERSION,
                        height: Height(height),
                        prev_hash: nightfall_types::Hash256::ZERO,
                        body_root: body.hash(),
                        utxo_root: ledger.utxo_root(),
                        kernel_sum: ledger.kernel_sum(),
                        timestamp_unix: 1_800_000_000 + height,
                        difficulty: 1,
                        nonce: height,
                        reward_darks: reward,
                    },
                    body,
                }
            })
            .collect();

        let at_once = a.scan_blocks(&chain).unwrap();

        // Exactly the loop in `WalletState::sync_from_node`, at a size a test
        // can afford: page from the scan position, stop when a page runs short.
        const PAGE: usize = 3;
        let mut in_pages = 0u32;
        let mut rounds = 0;
        loop {
            let from = b.scan_from();
            let page: Vec<Block> = chain
                .iter()
                .filter(|blk| blk.header.height.0 >= from)
                .take(PAGE)
                .cloned()
                .collect();
            if page.is_empty() {
                break;
            }
            let was_full = page.len() == PAGE;
            in_pages += b.scan_blocks(&page).unwrap();
            rounds += 1;
            assert!(rounds < 20, "the page loop must terminate");
            if !was_full {
                break;
            }
            assert!(
                b.scan_from() > from,
                "a full page must move the scan position"
            );
        }

        assert!(rounds > 1, "the fixture must actually need several pages");
        assert_eq!(
            in_pages, at_once,
            "the overlap block must not be counted a second time"
        );
        assert_eq!(a.scanned_to(), b.scanned_to(), "same scan position");
        assert_eq!(
            a.outputs().len(),
            b.outputs().len(),
            "same coins, no duplicates from the overlap"
        );
        assert_eq!(
            a.balances(9, 0).available,
            b.balances(9, 0).available,
            "same balance"
        );

        fs::remove_dir_all(&whole).ok();
        fs::remove_dir_all(&paged).ok();
    }

    /// A reorg that discards a block must discard what it contained.
    ///
    /// Before this, a coin received in a block that lost a reorg stayed in the
    /// wallet as spendable balance no node would accept, and a payment that had
    /// been undone still read "confirmed".
    #[test]
    fn a_discarded_block_takes_its_outputs_with_it() {
        let d = tmpdir("reorg-drop");
        let mut w = Wallet::open(&d, NetworkId::Devnet, "w.seed").unwrap();
        let ctx = NetworkId::Devnet.proof_context();
        let reward = 20 * DARKS_PER_NIGHT;

        let mk = |w: &Wallet, height: u64| {
            let cb = build_coinbase(&w.address(), reward, height, ctx).unwrap();
            let body = BlockBody::aggregate(&[cb]);
            let mut ledger = LedgerState::genesis();
            ledger
                .apply_block(&body, Height(height), reward, ctx)
                .unwrap();
            Block {
                header: nightfall_consensus::BlockHeader {
                    version: nightfall_types::PROTOCOL_VERSION,
                    height: Height(height),
                    prev_hash: nightfall_types::Hash256::ZERO,
                    body_root: body.hash(),
                    utxo_root: ledger.utxo_root(),
                    kernel_sum: ledger.kernel_sum(),
                    timestamp_unix: 1_800_000_000 + height,
                    difficulty: 1,
                    nonce: height,
                    reward_darks: reward,
                },
                body,
            }
        };

        // Two blocks, both ours.
        let a = mk(&w, 0);
        let b = mk(&w, 1);
        w.scan_blocks(&[a.clone(), b]).unwrap();
        assert_eq!(w.outputs().len(), 2);
        assert_eq!(w.balance().darks(), 2 * reward);

        // A reorg replaces height 1 with a block that is not ours. Rescanning
        // the canonical chain from genesis must let the old coin go.
        let stranger = WalletKeys::generate();
        let cb = build_coinbase(&stranger.address(), reward, 1, ctx).unwrap();
        let body = BlockBody::aggregate(&[cb]);
        let mut ledger = LedgerState::genesis();
        ledger.apply_block(&body, Height(1), reward, ctx).unwrap();
        let b2 = Block {
            header: nightfall_consensus::BlockHeader {
                version: nightfall_types::PROTOCOL_VERSION,
                height: Height(1),
                prev_hash: nightfall_types::Hash256::ZERO,
                body_root: body.hash(),
                utxo_root: ledger.utxo_root(),
                kernel_sum: ledger.kernel_sum(),
                timestamp_unix: 1_800_000_099,
                difficulty: 1,
                nonce: 99,
                reward_darks: reward,
            },
            body,
        };

        w.scan_blocks(&[a, b2]).unwrap();
        assert_eq!(w.outputs().len(), 1, "the replaced block's output must go");
        assert_eq!(
            w.balance().darks(),
            reward,
            "balance must not include a coin the chain no longer has"
        );
        fs::remove_dir_all(&d).ok();
    }

    /// The CLI fetches 128-block pages. Reconcile used to fire on any slice
    /// whose first block was height 0, so page one wiped every later output.
    /// A sync that stopped there — or a caller that re-scanned only genesis —
    /// lost those coins until something happened to look at them again.
    #[test]
    fn a_partial_page_from_genesis_does_not_drop_later_outputs() {
        let d = tmpdir("partial-page");
        let mut w = Wallet::open(&d, NetworkId::Devnet, "w.seed").unwrap();
        let ctx = NetworkId::Devnet.proof_context();
        let reward = 20 * DARKS_PER_NIGHT;

        let mk = |w: &Wallet, height: u64| {
            let cb = build_coinbase(&w.address(), reward, height, ctx).unwrap();
            let body = BlockBody::aggregate(&[cb]);
            let mut ledger = LedgerState::genesis();
            ledger
                .apply_block(&body, Height(height), reward, ctx)
                .unwrap();
            Block {
                header: nightfall_consensus::BlockHeader {
                    version: nightfall_types::PROTOCOL_VERSION,
                    height: Height(height),
                    prev_hash: nightfall_types::Hash256::ZERO,
                    body_root: body.hash(),
                    utxo_root: ledger.utxo_root(),
                    kernel_sum: ledger.kernel_sum(),
                    timestamp_unix: 1_800_000_000 + height,
                    difficulty: 1,
                    nonce: height,
                    reward_darks: reward,
                },
                body,
            }
        };

        let a = mk(&w, 0);
        let b = mk(&w, 1);
        w.scan_blocks(&[a.clone(), b]).unwrap();
        assert_eq!(w.outputs().len(), 2);
        assert_eq!(w.balance().darks(), 2 * reward);

        // Same first page, nothing after it. The later coin must stay.
        w.scan_blocks(&[a]).unwrap();
        assert_eq!(
            w.outputs().len(),
            2,
            "a genesis page is not the whole chain"
        );
        assert_eq!(w.balance().darks(), 2 * reward);
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn address_string_roundtrips() {
        let d = tmpdir("addr");
        let w = Wallet::open(&d, NetworkId::Devnet, "w.seed").unwrap();
        let decoded = Address::decode(&w.address_string()).unwrap();
        assert_eq!(decoded, w.address());
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn scanning_finds_a_coinbase_and_reports_balance() {
        let d = tmpdir("scan");
        let mut w = Wallet::open(&d, NetworkId::Devnet, "w.seed").unwrap();
        let ctx = NetworkId::Devnet.proof_context();
        let reward = 20 * DARKS_PER_NIGHT;

        let cb = build_coinbase(&w.address(), reward, 0, ctx).unwrap();
        let body = BlockBody::aggregate(&[cb]);
        let mut ledger = LedgerState::genesis();
        ledger.apply_block(&body, Height(0), reward, ctx).unwrap();

        let block = Block {
            header: nightfall_consensus::BlockHeader {
                version: nightfall_types::PROTOCOL_VERSION,
                height: Height(0),
                prev_hash: nightfall_types::Hash256::ZERO,
                utxo_root: ledger.utxo_root(),
                kernel_sum: ledger.kernel_sum(),
                body_root: body.hash(),
                timestamp_unix: 1,
                difficulty: 1,
                nonce: 0,
                reward_darks: reward,
            },
            body,
        };

        assert_eq!(w.scan_blocks(std::slice::from_ref(&block)).unwrap(), 1);
        assert_eq!(w.balance().darks(), reward);

        // Rescanning must not double-count.
        assert_eq!(w.scan_blocks(&[block]).unwrap(), 0);
        assert_eq!(w.balance().darks(), reward);

        fs::remove_dir_all(&d).ok();
    }

    /// A restored backup must not put old payments back on the wire.
    ///
    /// Every sync hands unconfirmed sends to the node again, which is right
    /// for payments this wallet actually made and wrong for payments read out
    /// of a backup file: that file is a photograph, and the chain has moved
    /// since. Quarantine withholds the automatic re-send without freezing the
    /// entry — and it applies to the imported payments only.
    #[test]
    fn an_imported_pending_send_is_never_resent_on_its_own() {
        let d = tmpdir("quarantine");
        let mut w = Wallet::open(&d, NetworkId::Devnet, "w.seed").unwrap();
        let ctx = NetworkId::Devnet.proof_context();
        let reward = 20 * DARKS_PER_NIGHT;

        // Two mined coins, because a payment consumes its input whole and the
        // change only comes back when the block carrying it is scanned. The
        // second payment at the end of this test needs a coin of its own.
        let blocks: Vec<Block> = (0..2u64)
            .map(|height| {
                let cb = build_coinbase(&w.address(), reward, height, ctx).unwrap();
                let body = BlockBody::aggregate(&[cb]);
                let mut ledger = LedgerState::genesis();
                ledger
                    .apply_block(&body, Height(height), reward, ctx)
                    .unwrap();
                Block {
                    header: nightfall_consensus::BlockHeader {
                        version: nightfall_types::PROTOCOL_VERSION,
                        height: Height(height),
                        prev_hash: nightfall_types::Hash256::ZERO,
                        utxo_root: ledger.utxo_root(),
                        kernel_sum: ledger.kernel_sum(),
                        body_root: body.hash(),
                        timestamp_unix: 1 + height,
                        difficulty: 1,
                        nonce: height,
                        reward_darks: reward,
                    },
                    body,
                }
            })
            .collect();
        assert_eq!(w.scan_blocks(&blocks).unwrap(), 2, "two coins to spend");

        let other = WalletKeys::generate().address();
        let tx = w.create_payment(&other, 1_000, 10, "").unwrap();
        w.record_send(&tx, 1_000, String::new()).unwrap();

        // Baseline: an ordinary pending send is offered for re-sending.
        assert_eq!(w.resendable().len(), 1, "the fixture must be resendable");

        assert_eq!(w.quarantine_pending_sends().unwrap(), 1);
        assert!(
            w.resendable().is_empty(),
            "an imported payment must not be handed to a node"
        );
        assert_eq!(w.quarantined().len(), 1, "and it must be visible instead");

        // Marking twice must not double-count what is waiting for the owner.
        assert_eq!(w.quarantine_pending_sends().unwrap(), 0);

        // The mark has to survive a restart, or the next sync resends anyway.
        drop(w);
        let mut reopened = Wallet::open(&d, NetworkId::Devnet, "w.seed").unwrap();
        assert!(
            reopened.resendable().is_empty(),
            "quarantine must be persisted, not just in memory"
        );
        assert_eq!(reopened.quarantined().len(), 1);

        // And it is per payment, not a switch that disables re-sending: a
        // payment made after the import behaves normally.
        let fresh = reopened.create_payment(&other, 2_000, 10, "").unwrap();
        reopened.record_send(&fresh, 2_000, String::new()).unwrap();
        let live = reopened.resendable();
        assert_eq!(live.len(), 1, "a new payment must still be resendable");
        assert_eq!(live[0].0, fresh.txid().to_hex());

        fs::remove_dir_all(&d).ok();
    }

    // Scanner fixtures, not consensus-validation fixtures: public deterministic
    // keys and genuine signed payments, no node, file, peer or real funds.
    fn pending_scan_fixture() -> (Wallet, Vec<Block>, Transaction) {
        let mut wallet = Wallet::in_memory(NetworkId::Devnet, WalletKeys::from_seed([0; 32]), 0);
        let other = WalletKeys::from_seed([1; 32]).address();
        let context = NetworkId::Devnet.proof_context();
        let mut blocks: Vec<Block> = Vec::new();
        for (height, address) in [wallet.address(), other].iter().enumerate() {
            let reward = 20 * DARKS_PER_NIGHT;
            let coinbase = build_coinbase(address, reward, height as u64, context).unwrap();
            let body = BlockBody::aggregate(&[coinbase]);
            let ledger = LedgerState::genesis();
            blocks.push(Block {
                header: nightfall_consensus::BlockHeader {
                    version: nightfall_types::PROTOCOL_VERSION,
                    height: Height(height as u64),
                    prev_hash: blocks.last().map(Block::hash).unwrap_or_default(),
                    body_root: body.hash(),
                    utxo_root: ledger.utxo_root(),
                    kernel_sum: ledger.kernel_sum(),
                    timestamp_unix: 1_800_000_000 + height as u64,
                    difficulty: 1,
                    nonce: height as u64,
                    reward_darks: reward,
                },
                body,
            });
        }
        wallet.scan_blocks(&blocks).unwrap();
        let tx = wallet
            .create_payment(&other, 1_000, 10, "public fixture")
            .unwrap();
        wallet
            .record_send(&tx, 1_000, "public fixture".into())
            .unwrap();
        (wallet, blocks, tx)
    }

    #[test]
    fn imported_confirmed_sends_stay_quarantined_after_a_reorg() {
        let (mut wallet, canonical, tx) = pending_scan_fixture();
        let mut confirmed = canonical.clone();
        let coinbase = build_coinbase(
            &WalletKeys::from_seed([1; 32]).address(),
            20 * DARKS_PER_NIGHT,
            1,
            NetworkId::Devnet.proof_context(),
        )
        .unwrap();
        confirmed[1].body = BlockBody::aggregate(&[coinbase, tx.clone()]);
        confirmed[1].header.body_root = confirmed[1].body.hash();
        wallet.scan_blocks(&confirmed).unwrap();
        assert!(wallet.unconfirmed_sends().is_empty());
        assert_eq!(wallet.quarantine_imported_sends().unwrap(), 0);
        let sent = wallet
            .history()
            .iter()
            .find(|e| e.direction == Direction::Sent)
            .unwrap();
        assert!(sent.height.is_some() && sent.quarantined);

        let password = "public reorg backup password";
        let sealed = crate::vault::Vault::create(&wallet, password).unwrap();
        let mut reopened = crate::vault::Vault::from_bytes(sealed.sealed_bytes()).unwrap();
        reopened.unlock(password, NetworkId::Devnet).unwrap();
        let restored = reopened.wallet_mut().unwrap();
        restored.scan_blocks(&canonical).unwrap();
        assert_eq!(restored.unconfirmed_sends().len(), 1);
        assert_eq!(restored.quarantined().len(), 1);
        assert!(
            restored.resendable().is_empty(),
            "a reorg is not authorization to send"
        );
        reopened.lock().unwrap();
        reopened.unlock(password, NetworkId::Devnet).unwrap();
        assert!(reopened.wallet().unwrap().resendable().is_empty());
        assert_eq!(reopened.wallet().unwrap().quarantined().len(), 1);
    }

    #[test]
    fn canonical_scans_do_not_release_unresolved_payment_inputs() {
        for imported in [false, true] {
            let (mut wallet, canonical, tx) = pending_scan_fixture();
            if imported {
                assert_eq!(wallet.quarantine_imported_sends().unwrap(), 1);
            }
            // The chain does not contain this payment, but its signed raw
            // transaction still exists and may still arrive at a miner.
            wallet.scan_blocks(&canonical).unwrap();
            let commitment = tx.inputs[0].commit;
            assert!(
                !wallet
                    .outputs()
                    .iter()
                    .find(|o| o.commit == commitment)
                    .unwrap()
                    .spent
            );
            let assert_held = |wallet: &Wallet| {
                assert_eq!(wallet.balance().darks(), 0);
                assert_eq!(wallet.balances(100, 10).available, 0);
                assert_eq!(wallet.balances(100, 10).pending_out, 1_010);
                assert_eq!(wallet.spendable_count(), 0);
                assert!(wallet.spendable_outputs(100, 10).is_empty());
                assert!(wallet.select_coins_at(1, 100, 10).is_err());
                assert!(wallet.pick_commit_hexes_at(1, 100, 10).is_err());
                assert!(wallet
                    .create_payment_from_commits_at(
                        &[commitment.to_hex()],
                        &WalletKeys::from_seed([1; 32]).address(),
                        1,
                        10,
                        "conflicting payment",
                        100,
                        10,
                    )
                    .is_err());
                assert_eq!(wallet.resendable().len(), usize::from(!imported));
            };
            assert_held(&wallet);
            let password = "public pending input password";
            let sealed = crate::vault::Vault::create(&wallet, password).unwrap();
            let mut reopened = crate::vault::Vault::from_bytes(sealed.sealed_bytes()).unwrap();
            reopened.unlock(password, NetworkId::Devnet).unwrap();
            assert_held(reopened.wallet().unwrap());
        }
    }

    #[test]
    fn state_import_refuses_unknown_nested_safety_fields_but_keeps_legacy_defaults() {
        let (wallet, _, _) = pending_scan_fixture();
        let state: serde_json::Value =
            serde_json::from_str(&wallet.export_state().unwrap()).unwrap();
        for pointer in ["/db", "/db/history/0", "/db/outputs/0"] {
            let mut future = state.clone();
            future
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("future_safety_hold".into(), true.into());
            assert!(
                Wallet::import_state(&future.to_string()).is_err(),
                "unknown field at {pointer}"
            );
        }
        let mut legacy = state;
        let db = legacy["db"].as_object_mut().unwrap();
        for field in ["birth_height", "reserved", "scanned_tip"] {
            db.remove(field);
        }
        for entry in db["history"].as_array_mut().unwrap() {
            for field in ["spent_commits", "raw", "quarantined"] {
                entry.as_object_mut().unwrap().remove(field);
            }
        }
        for output in db["outputs"].as_array_mut().unwrap() {
            output.as_object_mut().unwrap().remove("is_coinbase");
        }
        let restored = Wallet::import_state(&legacy.to_string()).unwrap();
        assert_eq!(restored.address(), wallet.address());
        assert_eq!(restored.db.birth_height, 0);
        assert!(restored.db.reserved.is_empty() && restored.db.scanned_tip.is_empty());
        assert!(restored
            .history()
            .iter()
            .all(|entry| !entry.quarantined && entry.raw.is_none()));
    }

    #[test]
    fn spending_more_than_the_balance_fails_clearly() {
        let d = tmpdir("insufficient");
        let w = Wallet::open(&d, NetworkId::Devnet, "w.seed").unwrap();
        let other = WalletKeys::generate().address();
        let e = w.create_payment(&other, 1_000, 10, "").unwrap_err();
        assert!(e.to_string().contains("insufficient funds"), "got: {e}");
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn reserved_output_is_not_spent_as_a_normal_payment() {
        let d = tmpdir("reserved");
        let mut w = Wallet::open(&d, NetworkId::Devnet, "w.seed").unwrap();
        let commit = Commitment::from_point(nightfall_crypto::generator_g());
        w.test_insert_output(OwnedOutput {
            commit,
            value: 1_000_000,
            blind_hex: hex::encode([1u8; 32]),
            key_offset_hex: hex::encode([2u8; 32]),
            memo: String::new(),
            height: 0,
            spent: false,
            is_coinbase: false,
        });
        w.reserve_commits(&[commit.to_hex()]).unwrap();
        let other = WalletKeys::generate().address();
        let e = w.create_payment(&other, 1, 1, "").unwrap_err();
        assert!(
            e.to_string().contains("insufficient funds"),
            "reserved coin was selectable; got {e}"
        );
        w.release_commits(&[commit.to_hex()]).unwrap();
        // After release the selector sees the coin. Building the tx still
        // needs a real spend secret; we only assert selection succeeds.
        assert!(w.select_coins(1).is_ok());
        let picked = w.pick_commit_hexes_at(1, 0, 0).unwrap();
        assert_eq!(picked, vec![commit.to_hex()]);
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_reserved_coin_can_still_fund_the_swap_lock() {
        let d = tmpdir("reserved-lock");
        let mut w = Wallet::open(&d, NetworkId::Devnet, "w.seed").unwrap();
        let ctx = NetworkId::Devnet.proof_context();
        let reward = 20 * DARKS_PER_NIGHT;
        let cb = build_coinbase(&w.address(), reward, 0, ctx).unwrap();
        let body = BlockBody::aggregate(&[cb]);
        let mut ledger = LedgerState::genesis();
        ledger.apply_block(&body, Height(0), reward, ctx).unwrap();
        let block = Block {
            header: nightfall_consensus::BlockHeader {
                version: nightfall_types::PROTOCOL_VERSION,
                height: Height(0),
                prev_hash: nightfall_types::Hash256::ZERO,
                utxo_root: ledger.utxo_root(),
                kernel_sum: ledger.kernel_sum(),
                body_root: body.hash(),
                timestamp_unix: 1,
                difficulty: 1,
                nonce: 0,
                reward_darks: reward,
            },
            body,
        };
        assert_eq!(w.scan_blocks(std::slice::from_ref(&block)).unwrap(), 1);
        let commit = w.outputs()[0].commit.to_hex();
        w.reserve_commits(std::slice::from_ref(&commit)).unwrap();
        let e = w.create_payment(&w.address(), 1, 1, "").unwrap_err();
        assert!(
            e.to_string().contains("insufficient funds"),
            "ordinary spend must not take the reserved coin: {e}"
        );
        let to = WalletKeys::generate().address();
        let fee = DARKS_PER_NIGHT / 1_000;
        let amount = reward - fee;
        let tx = w
            .create_payment_from_commits_at(
                std::slice::from_ref(&commit),
                &to,
                amount,
                fee,
                "swap-lock",
                20,
                10,
            )
            .expect("reserved coin must still fund the lock");
        assert!(tx.inputs.iter().any(|i| i.commit.to_hex() == commit));
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn in_memory_export_import_roundtrips() {
        let keys = WalletKeys::generate();
        let phrase = keys.to_mnemonic();
        let w = Wallet::in_memory(NetworkId::Devnet, keys, 7);
        let blob = w.export_state().unwrap();
        assert!(
            !blob.contains(&phrase),
            "mnemonic must not leak into the export blob"
        );
        let w2 = Wallet::import_state(&blob).unwrap();
        assert_eq!(w.address_string(), w2.address_string());
        assert_eq!(w.scan_from(), 7);
        assert_eq!(w2.scan_from(), 7);
        let restored = WalletKeys::from_mnemonic(&phrase).unwrap();
        assert_eq!(restored.address(), w.address());
    }
}
