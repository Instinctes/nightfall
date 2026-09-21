//! Public, unfunded synthetic wallet for browser/WASM regression checks only.
//! Never deploy this fixture or use its publicly known recovery words for funds.
use nightfall_crypto::{create_output, WalletKeys};
use nightfall_types::{GenesisConfig, NetworkId, DARKS_PER_NIGHT};
use nightfall_wallet::Wallet;
use serde_json::json;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn main() {
    let keys = WalletKeys::from_seed([59; 32]);
    let address = keys.address();
    let phrase = keys.to_mnemonic();
    let wallet = Wallet::in_memory(NetworkId::Mainnet, keys, 0);
    let recipient = WalletKeys::from_seed([60; 32]).address();
    let (output, _) = create_output(
        &address,
        10 * DARKS_PER_NIGHT,
        "public fixture",
        b"webwallet-test",
    )
    .expect("public output");
    let config = serde_json::to_vec(&GenesisConfig::fair_launch(NetworkId::Mainnet)).unwrap();
    println!(
        "{}",
        json!({
            "warning": "PUBLIC SYNTHETIC FIXTURE. NO REAL FUNDS. NEVER DEPLOY.",
            "phrase": phrase, "address": address.encode(), "recipient": recipient.encode(),
            "state": wallet.export_state().unwrap(),
            "genesis": nightfall_crypto::genesis_commitment(&config).to_hex(),
            "output": {"height": 0, "timestamp": 1750000000,
                "commit": hex(&output.commit.0), "ephemeral_pk": hex(&output.ephemeral_pk),
                "output_pk": hex(&output.output_pk), "view_tag": output.view_tag,
                "payload": hex(&output.payload), "coinbase": false}
        })
    );
}
