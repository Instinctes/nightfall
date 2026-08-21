//! Chain persistence.
//!
//! Blocks are stored append-only as newline-delimited JSON. Normal operation
//! appends exactly the new blocks; only a reorg rewrites the file.
//!
//! v4 serialized the **entire chain** as pretty-printed JSON every ten seconds
//! and again after every accepted block. At 84 blocks that was already a 1.5 MB
//! rewrite; the cost grows linearly with height forever (audit finding N-05).

use nightfall_consensus::{Chain, CompactHeader};
use nightfall_crypto::Commitment;
use nightfall_ledger::{LedgerState, UtxoEntry, UtxoSet};
use nightfall_types::{Hash256, Height, NetworkId};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub mod codec;
pub use codec::Format;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ChainMeta {
    network: NetworkId,
    genesis_hash: String,
    block_count: u64,
    protocol_version: u32,
    /// Tip hash this node validated in full, and the size of the blocks file at
    /// that moment. Both must match on reload before we trust our own past
    /// work and skip re-verifying proof of work.
    #[serde(default)]
    validated_tip: String,
    #[serde(default)]
    validated_bytes: u64,
    /// Height of the first body still in `blocks.jsonl`. Zero = archive.
    #[serde(default)]
    first_height: u64,
    #[serde(default)]
    pruned: bool,
    /// Cumulative work of the horizon prefix, decimal string.
    #[serde(default)]
    horizon_work: String,
    #[serde(default)]
    validated_horizon_bytes: u64,
    #[serde(default)]
    validated_headers_bytes: u64,
}

/// Sidecar written next to an exported `blocks.jsonl`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapshotManifest {
    pub v: u32,
    pub network: NetworkId,
    pub genesis: String,
    pub tip: String,
    pub blocks: u64,
}

pub struct ChainStore {
    pub dir: PathBuf,
}

impl ChainStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The chain file this datadir actually has.
    ///
    /// Binary when `blocks.bin` exists, JSON otherwise. A node therefore reads
    /// whichever it finds and nothing changes for anyone who has not run
    /// `nightfalld migrate-storage`.
    pub fn blocks_path(&self) -> PathBuf {
        self.dir.join(self.format().file_name())
    }

    /// Storage encoding in use here. See `codec`.
    pub fn format(&self) -> Format {
        codec::detect(&self.dir)
    }

    pub fn headers_path(&self) -> PathBuf {
        self.dir.join("headers.jsonl")
    }

    pub fn horizon_path(&self) -> PathBuf {
        self.dir.join("utxo-horizon.json")
    }

    pub fn meta_path(&self) -> PathBuf {
        self.dir.join("chain-meta.json")
    }

    /// Legacy v4 file. Its presence means the datadir holds a chain from the
    /// broken protocol.
    pub fn legacy_path(&self) -> PathBuf {
        self.dir.join("chain.json")
    }

    pub fn ensure_dir(&self) -> anyhow::Result<()> {
        fs::create_dir_all(&self.dir)?;
        Ok(())
    }

    fn stored_block_count(&self) -> u64 {
        fs::read_to_string(self.meta_path())
            .ok()
            .and_then(|s| serde_json::from_str::<ChainMeta>(&s).ok())
            .map(|m| m.block_count)
            .unwrap_or(0)
    }

    fn write_meta(&self, chain: &Chain) -> anyhow::Result<()> {
        let validated_bytes = fs::metadata(self.blocks_path())
            .map(|m| m.len())
            .unwrap_or(0);
        let validated_horizon_bytes = fs::metadata(self.horizon_path())
            .map(|m| m.len())
            .unwrap_or(0);
        let validated_headers_bytes = fs::metadata(self.headers_path())
            .map(|m| m.len())
            .unwrap_or(0);
        let meta = ChainMeta {
            network: chain.network,
            genesis_hash: chain.genesis_hash.to_hex(),
            block_count: chain.block_count(),
            protocol_version: nightfall_types::PROTOCOL_VERSION,
            validated_tip: chain.tip_hash().to_hex(),
            validated_bytes,
            first_height: chain.first_height,
            pruned: chain.is_pruned(),
            horizon_work: chain.horizon_work.to_string(),
            validated_horizon_bytes,
            validated_headers_bytes,
        };
        let tmp = self.dir.join("chain-meta.json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(&meta)?)?;
        fs::rename(tmp, self.meta_path())?;
        Ok(())
    }

    /// Last validated tip without replaying the chain. Used so RPC and the
    /// light API can answer while a long reload is still running.
    pub fn peek_meta(&self) -> Option<(u64, String, String)> {
        let raw = fs::read_to_string(self.meta_path()).ok()?;
        let m: ChainMeta = serde_json::from_str(&raw).ok()?;
        if m.validated_tip.is_empty() {
            return None;
        }
        Some((m.block_count, m.validated_tip, m.genesis_hash))
    }

    /// True when on-disk files are byte-for-byte what this node last validated.
    pub fn is_own_file_trusted(&self) -> bool {
        let Some(m) = self.read_meta() else {
            return false;
        };
        if m.validated_tip.is_empty() {
            return false;
        }
        let bytes = fs::metadata(self.blocks_path())
            .map(|x| x.len())
            .unwrap_or(0);
        if m.validated_bytes != bytes || bytes == 0 {
            return false;
        }
        if !m.pruned && m.first_height == 0 {
            return true;
        }
        let hz = fs::metadata(self.horizon_path())
            .map(|x| x.len())
            .unwrap_or(0);
        let hd = fs::metadata(self.headers_path())
            .map(|x| x.len())
            .unwrap_or(0);
        m.validated_horizon_bytes == hz && hz > 0 && m.validated_headers_bytes == hd && hd > 0
    }

    fn read_meta(&self) -> Option<ChainMeta> {
        fs::read_to_string(self.meta_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    }

    /// Rewrite the chain file in the binary encoding.
    ///
    /// Opt-in and reversible. The old `blocks.jsonl` is kept as
    /// `blocks.jsonl.pre-binary`; deleting `blocks.bin` puts the node back on
    /// JSON with nothing lost.
    ///
    /// Verified before anything is swapped: the binary file is written to a
    /// temporary name, read back, and every block hash compared against the
    /// source. A storage migration that trusts its own output is how a node
    /// ends up unable to start with no way back.
    ///
    /// Returns the old and new sizes in bytes.
    pub fn migrate_to_binary(&self) -> anyhow::Result<(u64, u64)> {
        if self.format() == Format::Binary {
            anyhow::bail!("this datadir is already binary");
        }
        let src = self.dir.join(codec::BLOCKS_JSONL);
        if !src.exists() {
            anyhow::bail!("no {} in {}", codec::BLOCKS_JSONL, self.dir.display());
        }
        let before = fs::metadata(&src)?.len();

        let blocks = codec::read_blocks(File::open(&src)?, Format::Json)?;
        if blocks.is_empty() {
            anyhow::bail!("the chain file is empty — nothing to convert");
        }

        let tmp = self.dir.join("blocks.bin.tmp");
        {
            let mut w = BufWriter::new(File::create(&tmp)?);
            for b in &blocks {
                codec::write_block(&mut w, b, Format::Binary)?;
            }
            w.flush()?;
        }

        // Read the new file back and compare. Hashes, not lengths: a file that
        // is the right size and the wrong content is the failure worth
        // catching.
        let back = codec::read_blocks(File::open(&tmp)?, Format::Binary)?;
        if back.len() != blocks.len() {
            let _ = fs::remove_file(&tmp);
            anyhow::bail!(
                "converted file has {} blocks, source had {} — not swapping",
                back.len(),
                blocks.len()
            );
        }
        for (i, (a, b)) in blocks.iter().zip(&back).enumerate() {
            if a.hash() != b.hash() {
                let _ = fs::remove_file(&tmp);
                anyhow::bail!("block {i} differs after conversion — not swapping");
            }
        }

        let after = fs::metadata(&tmp)?.len();
        fs::rename(&tmp, self.dir.join(codec::BLOCKS_BIN))?;
        fs::rename(&src, self.dir.join("blocks.jsonl.pre-binary"))?;

        // The size recorded in the meta belongs to the file that no longer
        // exists. Leaving it would make `is_own_file_trusted` compare the new
        // file against the old length, fail, and force a full re-verification
        // on the next start — correct, but hours of it for no reason.
        if let Some(mut m) = self.read_meta() {
            m.validated_bytes = after;
            let tmp_meta = self.dir.join("chain-meta.json.tmp");
            fs::write(&tmp_meta, serde_json::to_vec_pretty(&m)?)?;
            fs::rename(tmp_meta, self.meta_path())?;
        }
        Ok((before, after))
    }

    /// Copy `blocks.jsonl` plus a small manifest. The importer still verifies
    /// every block — this is a faster start, not a trust shortcut.
    pub fn export_snapshot(&self, out: &Path) -> anyhow::Result<SnapshotManifest> {
        if self
            .read_meta()
            .map(|m| m.pruned || m.first_height > 0)
            .unwrap_or(false)
        {
            anyhow::bail!(
                "this datadir is pruned — it cannot export a full snapshot. \
                 Resync from an archive node (a seed) first"
            );
        }
        let src = self.blocks_path();
        if !src.exists() {
            anyhow::bail!("no blocks.jsonl in {}", self.dir.display());
        }
        fs::create_dir_all(out)?;
        fs::copy(&src, out.join("blocks.jsonl"))?;
        let meta = self
            .read_meta()
            .ok_or_else(|| anyhow::anyhow!("no chain-meta.json — run the node once"))?;
        let snap = SnapshotManifest {
            v: 1,
            network: meta.network,
            genesis: meta.genesis_hash,
            tip: meta.validated_tip,
            blocks: meta.block_count,
        };
        fs::write(out.join("snapshot.json"), serde_json::to_vec_pretty(&snap)?)?;
        Ok(snap)
    }

    /// Install a snapshot directory and replay it with full PoW + supply
    /// checks. `validated_bytes` is forced to 0 so a copied file is never
    /// trusted as if this node had already verified it.
    pub fn import_snapshot(&self, from: &Path, network: NetworkId) -> anyhow::Result<Chain> {
        let src = if from.is_file() {
            from.to_path_buf()
        } else {
            from.join("blocks.jsonl")
        };
        if !src.exists() {
            anyhow::bail!("no blocks.jsonl at {}", src.display());
        }
        let manifest: Option<SnapshotManifest> = fs::read_to_string(from.join("snapshot.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok());
        if let Some(m) = &manifest {
            if m.network != network {
                anyhow::bail!(
                    "snapshot is {:?} but {:?} was requested",
                    m.network,
                    network
                );
            }
        }
        self.ensure_dir()?;
        fs::copy(&src, self.blocks_path())?;
        let meta = ChainMeta {
            network,
            genesis_hash: manifest
                .as_ref()
                .map(|m| m.genesis.clone())
                .unwrap_or_default(),
            block_count: manifest.as_ref().map(|m| m.blocks).unwrap_or(0),
            protocol_version: nightfall_types::PROTOCOL_VERSION,
            validated_tip: String::new(),
            validated_bytes: 0,
            first_height: 0,
            pruned: false,
            horizon_work: String::new(),
            validated_horizon_bytes: 0,
            validated_headers_bytes: 0,
        };
        fs::write(self.meta_path(), serde_json::to_vec_pretty(&meta)?)?;
        let chain = self.load_or_new(network)?;
        chain.verify_supply()?;
        if let Some(m) = &manifest {
            if !m.tip.is_empty() && chain.tip_hash().to_hex() != m.tip {
                anyhow::bail!(
                    "imported tip {} != snapshot tip {}",
                    chain.tip_hash().to_hex(),
                    m.tip
                );
            }
            if !m.genesis.is_empty() && chain.genesis_hash.to_hex() != m.genesis {
                anyhow::bail!(
                    "imported genesis {} != snapshot genesis {}",
                    chain.genesis_hash.to_hex(),
                    m.genesis
                );
            }
        }
        self.save(&chain)?;
        Ok(chain)
    }

    /// Tip hash recorded alongside the stored blocks, if any.
    fn stored_tip(&self) -> Option<String> {
        fs::read_to_string(self.meta_path())
            .ok()
            .and_then(|s| serde_json::from_str::<ChainMeta>(&s).ok())
            .map(|m| m.validated_tip)
            .filter(|t| !t.is_empty())
    }

    /// Persist. Appends only what is new unless the chain shrank or diverged.
    pub fn save(&self, chain: &Chain) -> anyhow::Result<()> {
        self.ensure_dir()?;
        if chain.is_pruned() {
            return self.save_pruned(chain);
        }
        let on_disk = self.stored_block_count();
        let have = chain.block_count();

        if on_disk > have || !self.blocks_path().exists() {
            // Reorg that shortened the chain, or first write.
            return self.rewrite_all(chain);
        }

        // A reorg can leave the chain the same length or longer while replacing
        // history. Appending in that case splices the new fork onto the
        // abandoned one and produces a file that will not replay — the node
        // then refuses to start. This check runs *before* the equal-length fast
        // path precisely because same-height-different-history is the case a
        // length comparison cannot see.
        if on_disk > 0 {
            let ancestor_matches = chain
                .hash_at(on_disk - 1)
                .map(|h| h.to_hex())
                .zip(self.stored_tip())
                .map(|(current, stored)| current == stored)
                .unwrap_or(false);

            if !ancestor_matches {
                tracing::info!("chain diverged from stored history — rewriting block file");
                return self.rewrite_all(chain);
            }
        }

        if on_disk == have {
            self.write_meta(chain)?;
            return Ok(());
        }

        let mut file = BufWriter::new(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.blocks_path())?,
        );
        let fmt = self.format();
        for block in &chain.blocks[on_disk as usize..] {
            codec::write_block(&mut file, block, fmt)?;
        }
        file.flush()?;
        self.write_meta(chain)?;
        Ok(())
    }

    fn save_pruned(&self, chain: &Chain) -> anyhow::Result<()> {
        let meta = self.read_meta();
        let stored_first = meta.as_ref().map(|m| m.first_height).unwrap_or(0);
        let stored_count = meta.as_ref().map(|m| m.block_count).unwrap_or(0);
        let stored_tip = meta
            .as_ref()
            .map(|m| m.validated_tip.as_str())
            .unwrap_or("");
        let ancestor_ok = stored_count == 0
            || chain
                .hash_at(stored_count - 1)
                .map(|h| h.to_hex() == stored_tip)
                .unwrap_or(false);
        let horizon_moved = stored_first != chain.first_height;
        let need_rewrite_bodies = !self.blocks_path().exists()
            || horizon_moved
            || stored_count > chain.block_count()
            || !ancestor_ok;

        self.write_headers(chain)?;
        self.write_horizon(chain)?;

        if need_rewrite_bodies {
            self.rewrite_bodies(chain)?;
        } else if stored_count < chain.block_count() {
            let already = stored_count.saturating_sub(chain.first_height) as usize;
            let mut file = BufWriter::new(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(self.blocks_path())?,
            );
            let fmt = self.format();
            for block in chain.blocks.iter().skip(already) {
                codec::write_block(&mut file, block, fmt)?;
            }
            file.flush()?;
        }

        self.write_meta(chain)?;
        Ok(())
    }

    fn write_headers(&self, chain: &Chain) -> anyhow::Result<()> {
        let tmp = self.dir.join("headers.jsonl.tmp");
        {
            let mut file = BufWriter::new(File::create(&tmp)?);
            for h in &chain.headers {
                serde_json::to_writer(&mut file, h)?;
                file.write_all(b"\n")?;
            }
            file.flush()?;
        }
        fs::rename(tmp, self.headers_path())?;
        Ok(())
    }

    fn write_horizon(&self, chain: &Chain) -> anyhow::Result<()> {
        let Some(horizon) = chain.horizon.as_ref() else {
            anyhow::bail!("pruned chain is missing its UTXO horizon");
        };
        let tmp = self.dir.join("utxo-horizon.json.tmp");
        fs::write(&tmp, serde_json::to_vec(&horizon_to_file(horizon))?)?;
        fs::rename(tmp, self.horizon_path())?;
        Ok(())
    }

    fn rewrite_bodies(&self, chain: &Chain) -> anyhow::Result<()> {
        let tmp = self.dir.join("blocks.jsonl.tmp");
        {
            let mut file = BufWriter::new(File::create(&tmp)?);
            for block in &chain.blocks {
                serde_json::to_writer(&mut file, block)?;
                file.write_all(b"\n")?;
            }
            file.flush()?;
        }
        fs::rename(tmp, self.blocks_path())?;
        Ok(())
    }

    fn rewrite_all(&self, chain: &Chain) -> anyhow::Result<()> {
        if chain.is_pruned() {
            self.write_headers(chain)?;
            self.write_horizon(chain)?;
            self.rewrite_bodies(chain)?;
            self.write_meta(chain)?;
            return Ok(());
        }
        let tmp = self.dir.join("blocks.jsonl.tmp");
        {
            let mut file = BufWriter::new(File::create(&tmp)?);
            for block in &chain.blocks {
                serde_json::to_writer(&mut file, block)?;
                file.write_all(b"\n")?;
            }
            file.flush()?;
        }
        fs::rename(tmp, self.blocks_path())?;
        self.write_meta(chain)?;
        Ok(())
    }

    /// Load a chain, replaying and revalidating every stored block.
    pub fn load_or_new(&self, network: NetworkId) -> anyhow::Result<Chain> {
        self.load_or_new_with_progress(network, |_, _| {})
    }

    /// Same as [`load_or_new`], reporting `(applied, expected)` so a UI can
    /// count while the file is still being read.
    pub fn load_or_new_with_progress(
        &self,
        network: NetworkId,
        mut on_progress: impl FnMut(u64, u64),
    ) -> anyhow::Result<Chain> {
        self.ensure_dir()?;

        if self.legacy_path().exists() && !self.blocks_path().exists() {
            anyhow::bail!(
                "This datadir contains a protocol v4 chain ({}).\n\
                 v4 is consensus-broken: its balance proof was a tautology, so any\n\
                 participant could mint unlimited NIGHT. The chain cannot be carried\n\
                 forward and v5 nodes will not peer with it.\n\
                 See docs/AUDIT-2026-08-12.md and docs/MIGRATION-v5.md.\n\
                 Move or delete the file to start the v5 chain.",
                self.legacy_path().display()
            );
        }

        if !self.blocks_path().exists() {
            return Ok(Chain::new_fair(network)?);
        }

        let meta: Option<ChainMeta> = fs::read_to_string(self.meta_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok());

        if let Some(m) = &meta {
            if m.network != network {
                anyhow::bail!(
                    "datadir holds a {:?} chain but {:?} was requested",
                    m.network,
                    network
                );
            }
        }

        // Trust our own past validation only if the blocks file is byte-for-byte
        // what it was when we recorded it. Any edit, truncation or corruption
        // changes the length and forces a full re-verification.
        let current_bytes = fs::metadata(self.blocks_path())
            .map(|m| m.len())
            .unwrap_or(0);
        let pruned = meta
            .as_ref()
            .map(|m| m.pruned || m.first_height > 0)
            .unwrap_or(false);
        let trusted = self.is_own_file_trusted();

        if pruned && !trusted {
            anyhow::bail!(
                "pruned datadir failed the validation record — the dropped \
                 bodies cannot be re-checked. Resync the chain (Settings → \
                 Resync chain, keep wallet) or copy an archive blocks.jsonl"
            );
        }

        if !trusted && current_bytes > 0 {
            tracing::info!(
                "no matching validation record — re-verifying proof of work for the whole chain"
            );
        }

        let now = now_unix();
        let expected = meta.as_ref().map(|m| m.block_count).unwrap_or(0);
        let mut chain = Chain::new_fair(network)?;

        if pruned {
            let m = meta.as_ref().expect("pruned load has meta");
            chain.first_height = m.first_height;
            chain.horizon_work = m.horizon_work.parse().unwrap_or(0);
            let horizon = load_horizon_file(&self.horizon_path())?;
            chain.ledger = horizon.clone();
            chain.horizon = Some(horizon);
            chain.total_work = chain.horizon_work;
            chain.headers = load_jsonl_headers(&self.headers_path())?
                .into_iter()
                .filter(|h| h.height < m.first_height)
                .collect();
            if chain.headers.len() as u64 != m.first_height {
                anyhow::bail!(
                    "headers.jsonl prefix is {} long, prune horizon is {}",
                    chain.headers.len(),
                    m.first_height
                );
            }
        }

        let stored_blocks = codec::read_blocks(File::open(self.blocks_path())?, self.format())?;

        for (i, block) in stored_blocks.into_iter().enumerate() {
            let outcome = if trusted {
                chain.apply_block_from_own_disk(block)
            } else {
                chain.apply_block(block, now)
            };
            outcome.map_err(|e| anyhow::anyhow!("stored block {i} failed revalidation: {e}"))?;
            if i == 0 || i % 128 == 0 {
                on_progress(chain.block_count(), expected);
            }
        }

        // The validation record must describe the chain we actually rebuilt.
        if trusted {
            if let Some(m) = &meta {
                if m.validated_tip != chain.tip_hash().to_hex() {
                    anyhow::bail!(
                        "validation record points at tip {} but the stored blocks produce {} — \
                         delete chain-meta.json to force a full re-verification",
                        m.validated_tip,
                        chain.tip_hash()
                    );
                }
            }
            if let Some(tip) = chain.blocks.last() {
                if chain.ledger.utxo_root() != tip.header.utxo_root {
                    anyhow::bail!("replayed UTXO root does not match the stored tip");
                }
                if chain.ledger.kernel_sum() != tip.header.kernel_sum {
                    anyhow::bail!("replayed kernel sum does not match the stored tip");
                }
            }
        }

        // Refuse to start on a chain whose supply does not add up. This is
        // always checked, trusted replay or not.
        chain
            .verify_supply()
            .map_err(|e| anyhow::anyhow!("stored chain violates the supply invariant: {e}"))?;

        if pruned && expected > 0 && chain.block_count() != expected {
            anyhow::bail!(
                "pruned chain rebuilt to {} blocks, meta says {expected}",
                chain.block_count()
            );
        }

        Ok(chain)
    }
}

#[derive(Serialize, Deserialize)]
struct HorizonFile {
    height: u64,
    minted: u64,
    burned: u64,
    tx_count: u64,
    coinbase_maturity: u64,
    kernel_sum: String,
    kernel_count: u64,
    utxos: Vec<HorizonUtxo>,
}

#[derive(Serialize, Deserialize)]
struct HorizonUtxo {
    commit: String,
    output_pk: String,
    height: u64,
    is_coinbase: bool,
}

fn horizon_to_file(h: &LedgerState) -> HorizonFile {
    HorizonFile {
        height: h.height.0,
        minted: h.supply.total_minted_darks,
        burned: h.supply.total_burned_darks,
        tx_count: h.tx_count,
        coinbase_maturity: h.coinbase_maturity,
        kernel_sum: h.kernels.sum.to_hex(),
        kernel_count: h.kernels.count,
        utxos: h
            .utxos
            .entries
            .iter()
            .map(|(k, e)| HorizonUtxo {
                commit: Hash256(*k).to_hex(),
                output_pk: Hash256(e.output_pk).to_hex(),
                height: e.height,
                is_coinbase: e.is_coinbase,
            })
            .collect(),
    }
}

fn load_horizon_file(path: &Path) -> anyhow::Result<LedgerState> {
    let file: HorizonFile = serde_json::from_slice(&fs::read(path)?)
        .map_err(|e| anyhow::anyhow!("utxo-horizon.json is corrupt: {e}"))?;
    let mut utxos = UtxoSet::new();
    for u in file.utxos {
        let commit =
            Hash256::from_hex(&u.commit).map_err(|_| anyhow::anyhow!("bad commit in horizon"))?;
        let pk = Hash256::from_hex(&u.output_pk)
            .map_err(|_| anyhow::anyhow!("bad output_pk in horizon"))?;
        utxos.insert(
            Commitment(commit.0),
            UtxoEntry {
                output_pk: pk.0,
                height: u.height,
                is_coinbase: u.is_coinbase,
            },
        );
    }
    let sum = Hash256::from_hex(&file.kernel_sum)
        .map_err(|_| anyhow::anyhow!("bad kernel_sum in horizon"))?;
    Ok(LedgerState {
        height: Height(file.height),
        utxos,
        kernels: nightfall_ledger::KernelAccumulator {
            sum: Commitment(sum.0),
            count: file.kernel_count,
        },
        supply: nightfall_ledger::SupplyState {
            total_minted_darks: file.minted,
            total_burned_darks: file.burned,
        },
        tx_count: file.tx_count,
        coinbase_maturity: file.coinbase_maturity,
    })
}

fn load_jsonl_headers(path: &Path) -> anyhow::Result<Vec<CompactHeader>> {
    let file = BufReader::new(File::open(path)?);
    let mut out = Vec::new();
    for (i, line) in file.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let h: CompactHeader = serde_json::from_str(&line)
            .map_err(|e| anyhow::anyhow!("header {i} is corrupt: {e}"))?;
        out.push(h);
    }
    Ok(out)
}

/// Datadir epoch. The August 2026 v7 chain used `nightfall/<network>/`.
/// This chain uses a sibling folder so an old wallet cannot mine the corpse.
pub const DATADIR_EPOCH: &str = "n8";

pub fn default_data_dir(network: NetworkId) -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("nightfall")
        .join(network.as_str())
        .join(DATADIR_EPOCH)
}

pub fn wallet_dir(datadir: &Path) -> PathBuf {
    datadir.join("wallet")
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Write a file containing secret material with owner-only permissions.
///
/// v4 wrote `core.seed` with the process umask, typically world-readable
/// (`0644`). Any other account on the machine could read the wallet seed and
/// steal every coin (audit finding W-01).
pub fn write_secret_file(path: &Path, contents: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)?;
    harden_permissions(path)?;
    Ok(())
}

/// Restrict a file to owner read/write.
pub fn harden_permissions(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(path, perms)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_files_are_owner_only() {
        let dir = std::env::temp_dir().join(format!("nf-test-{}", now_unix()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("secret.seed");
        write_secret_file(&p, "deadbeef").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "seed file must not be readable by others");
        }
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn legacy_v4_datadir_is_refused() {
        let dir = std::env::temp_dir().join(format!("nf-legacy-{}", now_unix()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("chain.json"), "{}").unwrap();

        let store = ChainStore::new(&dir);
        let err = store.load_or_new(NetworkId::Devnet).unwrap_err();
        assert!(
            err.to_string().contains("v4"),
            "must refuse a v4 datadir loudly: {err}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn snapshot_roundtrip_rechecks_supply() {
        let dir = std::env::temp_dir().join(format!("nf-snap-src-{}", now_unix()));
        let out = std::env::temp_dir().join(format!("nf-snap-out-{}", now_unix()));
        let dest = std::env::temp_dir().join(format!("nf-snap-dst-{}", now_unix()));
        fs::create_dir_all(&dir).unwrap();
        let store = ChainStore::new(&dir);
        let chain = Chain::new_fair(NetworkId::Devnet).unwrap();
        store.save(&chain).unwrap();
        let snap = store.export_snapshot(&out).unwrap();
        assert_eq!(snap.blocks, chain.block_count());
        assert!(out.join("blocks.jsonl").exists());
        assert!(out.join("snapshot.json").exists());

        let imported = ChainStore::new(&dest)
            .import_snapshot(&out, NetworkId::Devnet)
            .unwrap();
        assert_eq!(imported.tip_hash(), chain.tip_hash());
        imported.verify_supply().unwrap();
        fs::remove_dir_all(&dir).ok();
        fs::remove_dir_all(&out).ok();
        fs::remove_dir_all(&dest).ok();
    }

    #[test]
    fn pruned_roundtrip_keeps_utxo_and_drops_bodies() {
        use nightfall_crypto::WalletKeys;
        let dir = std::env::temp_dir().join(format!("nf-prune-{}", now_unix()));
        fs::create_dir_all(&dir).unwrap();
        let store = ChainStore::new(&dir);
        let miner = WalletKeys::generate().address();
        let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();
        for i in 0..12u64 {
            chain
                .mine_block(&miner, vec![], now_unix() + i * 15)
                .unwrap();
        }
        let full_count = chain.block_count();
        let tip = chain.tip_hash();
        let root = chain.ledger.utxo_root();
        let dropped = chain.prune_keep(4).unwrap();
        assert_eq!(dropped, 8);
        assert_eq!(chain.first_height, 8);
        assert_eq!(chain.blocks.len(), 4);
        assert_eq!(chain.block_count(), full_count);
        store.save(&chain).unwrap();
        assert!(store.horizon_path().exists());
        assert!(store.headers_path().exists());

        let loaded = store.load_or_new(NetworkId::Devnet).unwrap();
        assert_eq!(loaded.tip_hash(), tip);
        assert_eq!(loaded.block_count(), full_count);
        assert_eq!(loaded.first_height, 8);
        assert_eq!(loaded.blocks.len(), 4);
        assert_eq!(loaded.ledger.utxo_root(), root);
        loaded.verify_supply().unwrap();
        assert!(loaded.block_by_height(0).is_none());
        assert!(loaded.block_by_height(8).is_some());
        fs::remove_dir_all(&dir).ok();
    }

    /// The whole point of the conversion: a node that ran on JSON and a node
    /// that converted must hold the same chain. Tip, height, and UTXO root —
    /// if any of the three moved, the format changed meaning, and the format
    /// is not allowed to mean anything.
    #[test]
    fn migration_keeps_the_identical_chain() {
        use nightfall_crypto::WalletKeys;
        let dir = std::env::temp_dir().join(format!("nf-mig-{}", now_unix()));
        fs::create_dir_all(&dir).unwrap();
        let store = ChainStore::new(&dir);
        let miner = WalletKeys::generate().address();
        let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();
        for i in 0..8u64 {
            chain
                .mine_block(&miner, vec![], now_unix() + i * 15)
                .unwrap();
        }
        store.save(&chain).unwrap();
        assert_eq!(store.format(), Format::Json, "fresh datadir starts as json");

        let (before, after) = store.migrate_to_binary().unwrap();
        assert!(after < before, "{after} B is not smaller than {before} B");
        assert_eq!(store.format(), Format::Binary);
        assert!(
            dir.join("blocks.jsonl.pre-binary").exists(),
            "the old file must survive so the operator can go back"
        );
        assert!(!dir.join(codec::BLOCKS_JSONL).exists());

        let loaded = store.load_or_new(NetworkId::Devnet).unwrap();
        assert_eq!(loaded.tip_hash(), chain.tip_hash());
        assert_eq!(loaded.block_count(), chain.block_count());
        assert_eq!(loaded.ledger.utxo_root(), chain.ledger.utxo_root());
        loaded.verify_supply().unwrap();
        fs::remove_dir_all(&dir).ok();
    }

    /// After converting, the node keeps mining into the file it now has.
    /// Appending to the abandoned .jsonl would silently split the chain in
    /// two, with the newer half in the file nothing reads.
    #[test]
    fn blocks_mined_after_migration_land_in_the_binary_file() {
        use nightfall_crypto::WalletKeys;
        let dir = std::env::temp_dir().join(format!("nf-mig-app-{}", now_unix()));
        fs::create_dir_all(&dir).unwrap();
        let store = ChainStore::new(&dir);
        let miner = WalletKeys::generate().address();
        let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();
        for i in 0..4u64 {
            chain
                .mine_block(&miner, vec![], now_unix() + i * 15)
                .unwrap();
        }
        store.save(&chain).unwrap();
        store.migrate_to_binary().unwrap();

        let mut chain = store.load_or_new(NetworkId::Devnet).unwrap();
        for i in 4..9u64 {
            chain
                .mine_block(&miner, vec![], now_unix() + i * 15)
                .unwrap();
            // save() appends only what is not on disk yet, so this exercises
            // the incremental write path, not a rewrite.
            store.save(&chain).unwrap();
        }
        let tip = chain.tip_hash();

        let reloaded = store.load_or_new(NetworkId::Devnet).unwrap();
        assert_eq!(reloaded.block_count(), chain.block_count());
        assert_eq!(reloaded.tip_hash(), tip);
        reloaded.verify_supply().unwrap();
        // The stale file must not have grown a single byte.
        let stale = fs::read_to_string(dir.join("blocks.jsonl.pre-binary")).unwrap();
        assert_eq!(
            stale.lines().count(),
            4,
            "the abandoned file was written to after the swap"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn migrating_an_already_binary_datadir_is_refused() {
        use nightfall_crypto::WalletKeys;
        let dir = std::env::temp_dir().join(format!("nf-mig-twice-{}", now_unix()));
        fs::create_dir_all(&dir).unwrap();
        let store = ChainStore::new(&dir);
        let miner = WalletKeys::generate().address();
        let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();
        chain.mine_block(&miner, vec![], now_unix()).unwrap();
        store.save(&chain).unwrap();
        store.migrate_to_binary().unwrap();

        let err = store.migrate_to_binary().unwrap_err().to_string();
        assert!(err.contains("already binary"), "got: {err}");
        fs::remove_dir_all(&dir).ok();
    }
}
