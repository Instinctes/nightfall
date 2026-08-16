//! Wallet state: key storage, output scanning, coin selection, spending.
//!
//! Shared by the CLI wallet and the Core GUI so the two can never drift apart.

use anyhow::{bail, Context};
use curve25519_dalek::scalar::Scalar;
use nightfall_consensus::Block;
use nightfall_crypto::{scan_candidate, scan_output, Address, Commitment, ScanCandidate, WalletKeys};
use nightfall_ledger::{build_transfer, Payment, Spendable, Transaction};
use nightfall_storage::write_secret_file;
use nightfall_types::{Amount, NetworkId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

mod receipt;
pub use receipt::{verify_receipt, PaymentReceipt};

/// An output this wallet owns, as persisted to disk.
#[derive(Clone, Debug, Serialize, Deserialize)]
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
}

impl HistoryEntry {
    pub fn is_pending(&self) -> bool {
        self.height.is_none()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
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
        fs::create_dir_all(datadir)?;
        let seed_path = datadir.join(seed_name);
        let db_path = datadir.join(format!("{seed_name}.outputs.json"));

        if let Some(keys) = provided {
            if seed_path.exists() {
                bail!(
                    "{} already exists — refusing to overwrite a seed. \
                     Move it aside first if you really mean to replace it.",
                    seed_path.display()
                );
            }
            write_secret_file(&seed_path, &hex::encode(keys.seed))?;
        }

        let keys = if seed_path.exists() {
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

        let existed = db_path.exists();
        let mut db: WalletFile = if existed {
            serde_json::from_str(&fs::read_to_string(&db_path)?).unwrap_or_default()
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
        Amount(
            self.db
                .outputs
                .iter()
                .filter(|o| !o.spent)
                .map(|o| o.value)
                .sum(),
        )
    }

    pub fn spendable_count(&self) -> usize {
        self.db.outputs.iter().filter(|o| !o.spent).count()
    }

    pub fn outputs(&self) -> &[OwnedOutput] {
        &self.db.outputs
    }

    pub fn scanned_to(&self) -> u64 {
        self.db.scanned_to
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
        Ok(serde_json::to_string(&serde_json::json!({
            "v": 1,
            "network": self.network.as_str(),
            "seed": hex::encode(self.keys.seed),
            "db": self.db,
        }))?)
    }

    pub fn import_state(blob: &str) -> anyhow::Result<Self> {
        let v: serde_json::Value = serde_json::from_str(blob)?;
        let seed_hex = v
            .get("seed")
            .and_then(|x| x.as_str())
            .context("state missing seed")?;
        let raw = hex::decode(seed_hex).context("seed hex")?;
        if raw.len() != 32 {
            anyhow::bail!("seed must be 32 bytes");
        }
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&raw);
        let keys = WalletKeys::from_seed(seed);
        let network = match v.get("network").and_then(|x| x.as_str()).unwrap_or("mainnet") {
            "testnet" => NetworkId::Testnet,
            "devnet" => NetworkId::Devnet,
            _ => NetworkId::Mainnet,
        };
        let db: WalletFile = match v.get("db") {
            Some(d) => serde_json::from_value(d.clone())?,
            None => WalletFile::default(),
        };
        Ok(Self {
            keys,
            network,
            seed_path: PathBuf::new(),
            db_path: PathBuf::new(),
            db,
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
            if spent_commits.iter().any(|c| pending.iter().any(|(_, s)| s.contains(c))) {
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
        self.save()?;
        Ok(found)
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

    /// Balance split into spendable, immature and pending-outgoing.
    ///
    /// `tip_height` and `maturity` come from the node, so the wallet never
    /// guesses whether a coinbase can be spent yet.
    pub fn balances(&self, tip_height: u64, maturity: u64) -> Balances {
        let mut b = Balances::default();
        for o in self.db.outputs.iter().filter(|o| !o.spent) {
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

    /// Spendable outputs, respecting coinbase maturity.
    pub fn spendable_outputs(&self, tip_height: u64, maturity: u64) -> Vec<&OwnedOutput> {
        self.db
            .outputs
            .iter()
            .filter(|o| !o.spent)
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
        let mut available: Vec<&OwnedOutput> = self
            .db
            .outputs
            .iter()
            .filter(|o| !o.spent)
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
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
                    spent_commits: tx.inputs.iter().map(|i| i.commit.0).collect(),
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
