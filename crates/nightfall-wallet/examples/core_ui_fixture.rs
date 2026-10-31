//! Public disposable Devnet UI fixture. Never use this known seed for funds.
use anyhow::{Context, Result};
use nightfall_crypto::WalletKeys;
use nightfall_types::NetworkId;
use nightfall_wallet::{counter::Invoice, vault_store::VaultStore, Wallet};
use std::{path::PathBuf, sync::Arc};
fn main() -> Result<()> {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .context("Name a NEW disposable directory")?,
    );
    std::fs::create_dir(&dir)?; // Refuse any existing directory, including real wallets.
    let lock = Arc::new(nightfall_storage::dirlock::acquire(&dir)?);
    let mut wallet = Wallet::in_memory(NetworkId::Devnet, WalletKeys::from_seed([0x3b; 32]), 0);
    wallet.add_invoice(Invoice {
        reference: "PUBLIC-UI-1".into(),
        amount_darks: Some(125000000),
        description: "Public fixture coffee".into(),
        created_unix: 1,
        expires_unix: None,
        closed_note: None,
    })?;
    let mut store = VaultStore::open(lock, "core.seed", NetworkId::Devnet)?;
    store.initialize(&wallet, "PUBLIC disposable test password")?;
    println!("PUBLIC SYNTHETIC Devnet wallet created. Existing test password: PUBLIC disposable test password. No real funds.");
    Ok(())
}
