//! Chain persistence.
//!
//! Blocks are stored append-only as newline-delimited JSON. Normal operation
//! appends exactly the new blocks; only a reorg rewrites the file.
//!
//! v4 serialized the **entire chain** as pretty-printed JSON every ten seconds
//! and again after every accepted block. At 84 blocks that was already a 1.5 MB
//! rewrite; the cost grows linearly with height forever (audit finding N-05).

use nightfall_consensus::{Block, Chain};
use nightfall_types::NetworkId;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

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
}

pub struct ChainStore {
    pub dir: PathBuf,
}

impl ChainStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn blocks_path(&self) -> PathBuf {
        self.dir.join("blocks.jsonl")
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
        let meta = ChainMeta {
            network: chain.network,
            genesis_hash: chain.genesis_hash.to_hex(),
            block_count: chain.block_count(),
            protocol_version: nightfall_types::PROTOCOL_VERSION,
            validated_tip: chain.tip_hash().to_hex(),
            validated_bytes,
        };
        let tmp = self.dir.join("chain-meta.json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(&meta)?)?;
        fs::rename(tmp, self.meta_path())?;
        Ok(())
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
                .blocks
                .get(on_disk as usize - 1)
                .map(|b| b.hash().to_hex())
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
        for block in &chain.blocks[on_disk as usize..] {
            serde_json::to_writer(&mut file, block)?;
            file.write_all(b"\n")?;
        }
        file.flush()?;
        self.write_meta(chain)?;
        Ok(())
    }

    fn rewrite_all(&self, chain: &Chain) -> anyhow::Result<()> {
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
        let trusted = meta
            .as_ref()
            .map(|m| m.validated_bytes == current_bytes && !m.validated_tip.is_empty())
            .unwrap_or(false);

        if !trusted && current_bytes > 0 {
            tracing::info!(
                "no matching validation record — re-verifying proof of work for the whole chain"
            );
        }

        let now = now_unix();
        let mut chain = Chain::new_fair(network)?;
        let file = BufReader::new(File::open(self.blocks_path())?);

        for (i, line) in file.lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let block: Block = serde_json::from_str(&line)
                .map_err(|e| anyhow::anyhow!("block {i} is corrupt: {e}"))?;

            let outcome = if trusted {
                chain.apply_block_locally_trusted(block, now)
            } else {
                chain.apply_block(block, now)
            };
            outcome.map_err(|e| anyhow::anyhow!("stored block {i} failed revalidation: {e}"))?;
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
        }

        // Refuse to start on a chain whose supply does not add up. This is
        // always checked, trusted replay or not.
        chain
            .verify_supply()
            .map_err(|e| anyhow::anyhow!("stored chain violates the supply invariant: {e}"))?;

        Ok(chain)
    }
}

/// Datadir epoch. The August 2026 v7 chain used `nightfall/<network>/`.
/// This chain uses a sibling folder so an old wallet cannot mine the corpse.
pub const DATADIR_EPOCH: &str = "n8";

pub fn default_data_dir(network: NetworkId) -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("nightfall").join(network.as_str()).join(DATADIR_EPOCH)
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
}
