//! Public test fixtures only. Never point this tool at a real wallet file.
//!
//! `create-funded` mints coins into its fixture. They are real outputs in the
//! cryptographic sense and worth nothing in every other one: the key is the
//! public zero-entropy vector, and the blocks holding them exist only inside
//! this process and are never submitted anywhere.
use nightfall_crypto::WalletKeys;
use nightfall_types::NetworkId;
use nightfall_wallet::{
    vault::{Vault, MAX_VAULT_BYTES},
    Wallet,
};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::Path,
};
const PASSWORD: &str = "Nightfall public interop test password";

/// A fixture wallet holding two mined coins, built entirely offline.
///
/// The browser side needs spendable coins to exercise the prepare/commit
/// discipline at all, and it has no way to mint them: a light wallet receives
/// coins by scanning, and a scan page has to be real output cryptography, not
/// a hand-written object. So the coins are minted here, where the ledger
/// builder lives, and travel across in the sealed vault.
///
/// Public zero-entropy key, blocks that exist only in this process, never
/// submitted anywhere. Heights 0 and 1, so a browser-side `tip` above the
/// maturity window makes them spendable.
fn funded_fixture() -> anyhow::Result<Wallet> {
    use nightfall_consensus::{Block, BlockHeader};
    use nightfall_ledger::{build_coinbase, BlockBody, LedgerState};
    use nightfall_types::{Hash256, Height, DARKS_PER_NIGHT, PROTOCOL_VERSION};

    let mut wallet = Wallet::in_memory(NetworkId::Mainnet, WalletKeys::from_seed([0; 32]), 0);
    let ctx = NetworkId::Mainnet.proof_context();
    let reward = 20 * DARKS_PER_NIGHT;
    let blocks: Vec<Block> = (0..2u64)
        .map(|height| -> anyhow::Result<Block> {
            let coinbase = build_coinbase(&wallet.address(), reward, height, ctx)?;
            let body = BlockBody::aggregate(&[coinbase]);
            let mut ledger = LedgerState::genesis();
            ledger.apply_block(&body, Height(height), reward, ctx)?;
            Ok(Block {
                header: BlockHeader {
                    version: PROTOCOL_VERSION,
                    height: Height(height),
                    prev_hash: Hash256::ZERO,
                    utxo_root: ledger.utxo_root(),
                    kernel_sum: ledger.kernel_sum(),
                    body_root: body.hash(),
                    timestamp_unix: 1_800_000_000 + height,
                    difficulty: 1,
                    nonce: height,
                    reward_darks: reward,
                },
                body,
            })
        })
        .collect::<anyhow::Result<_>>()?;
    let found = wallet.scan_blocks(&blocks)?;
    anyhow::ensure!(
        found == 2,
        "funded fixture must hold two coins, found {found}"
    );
    Ok(wallet)
}

fn write_new(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    anyhow::ensure!(
        args.len() == 3,
        "usage: vault_interop create|create-funded|check <temporary-fixture-path>"
    );
    let fixture = Wallet::in_memory(NetworkId::Mainnet, WalletKeys::from_seed([0; 32]), 42);
    let path = Path::new(&args[2]);
    match args[1].as_str() {
        "create-funded" => {
            let funded = funded_fixture()?;
            let vault = Vault::create(&funded, PASSWORD)?;
            write_new(path, vault.sealed_bytes())?;
            println!(
                "Created public funded interoperability fixture: {} coins, scanned to {}.",
                funded.spendable_count(),
                funded.scanned_to()
            );
        }
        "create" => {
            let vault = Vault::create(&fixture, PASSWORD)?;
            write_new(path, vault.sealed_bytes())?;
            println!("Created public unfunded interoperability fixture.");
        }
        "check" => {
            let mut bytes = Vec::new();
            std::fs::File::open(path)?
                .take((MAX_VAULT_BYTES + 1) as u64)
                .read_to_end(&mut bytes)?;
            let mut vault = Vault::from_bytes(&bytes)?;
            vault.unlock(PASSWORD, NetworkId::Mainnet)?;
            anyhow::ensure!(
                vault.wallet()?.export_state()? == fixture.export_state()?,
                "fixture state mismatch"
            );
            println!("Native verifier accepted the WASM-generated vault with exact fixture state.");
        }
        _ => anyhow::bail!("expected create or check"),
    }
    Ok(())
}
