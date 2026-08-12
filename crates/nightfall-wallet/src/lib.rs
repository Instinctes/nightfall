//! Wallet state: key storage, output scanning, coin selection, spending.
//!
//! Shared by the CLI wallet and the Core GUI so the two can never drift apart.

use anyhow::{bail, Context};
use curve25519_dalek::scalar::Scalar;
use nightfall_consensus::Block;
use nightfall_crypto::{scan_output, Address, Commitment, WalletKeys};
use nightfall_ledger::{build_transfer, Payment, Spendable, Transaction};
use nightfall_storage::write_secret_file;
use nightfall_types::{Amount, NetworkId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

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
}

impl Wallet {
    /// Open an existing wallet or create a new one.
    pub fn open(datadir: &Path, network: NetworkId, seed_name: &str) -> anyhow::Result<Self> {
        fs::create_dir_all(datadir)?;
        let seed_path = datadir.join(seed_name);
        let db_path = datadir.join(format!("{seed_name}.outputs.json"));

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

        let db = if db_path.exists() {
            serde_json::from_str(&fs::read_to_string(&db_path)?).unwrap_or_default()
        } else {
            WalletFile::default()
        };

        Ok(Self {
            keys,
            network,
            seed_path,
            db_path,
            db,
        })
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

    fn save(&self) -> anyhow::Result<()> {
        let tmp = self.db_path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(&self.db)?)?;
        fs::rename(&tmp, &self.db_path)?;
        // The output file contains blinding factors — treat it as secret.
        let _ = nightfall_storage::harden_permissions(&self.db_path);
        Ok(())
    }

    /// Scan blocks for outputs belonging to this wallet and mark spent ones.
    ///
    /// Returns how many new outputs were discovered.
    pub fn scan_blocks(&mut self, blocks: &[Block]) -> anyhow::Result<u32> {
        let view = self.keys.view_key();
        let known: BTreeSet<[u8; 32]> = self.db.outputs.iter().map(|o| o.commit.0).collect();
        let mut found = 0u32;
        let mut spent_commits: BTreeSet<[u8; 32]> = BTreeSet::new();
        let mut highest = self.db.scanned_to;
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

    /// Forget everything discovered and rescan from scratch.
    pub fn reset_scan(&mut self) -> anyhow::Result<()> {
        self.db = WalletFile::default();
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

        assert_eq!(w.scan_blocks(&[block.clone()]).unwrap(), 1);
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
}
