use std::fs;
use std::path::PathBuf;

fn directly_attached_attributes_before(source: &str, needle: &str) -> String {
    let lines: Vec<&str> = source.lines().collect();

    let fn_index = lines
        .iter()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("missing item `{needle}`"));

    let mut attrs = Vec::new();
    let mut i = fn_index;

    while i > 0 {
        i -= 1;
        let trimmed = lines[i].trim();

        if trimmed.is_empty() {
            continue;
        }

        if trimmed.starts_with("#[") {
            attrs.push(trimmed.to_string());
            continue;
        }

        break;
    }

    attrs.reverse();
    attrs.join("\n")
}

#[test]
fn legacy_raw_state_helpers_are_not_wasm_exports() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    let lib = fs::read_to_string(manifest.join("src/lib.rs"))
        .expect("read nightfall-web src/lib.rs");

    let vault = fs::read_to_string(manifest.join("src/vault.rs"))
        .expect("read nightfall-web src/vault.rs");

    let hidden = [
        "create_wallet",
        "restore_wallet",
        "wallet_address",
        "wallet_phrase",
        "wallet_view_key",
        "wallet_scan_from",
        "wallet_info",
        "reset_scan",
        "ingest_page",
        "wallet_balance",
        "wallet_history",
        "build_send",
    ];

    for name in hidden {
        let needle = format!("pub fn {name}");
        let attrs = directly_attached_attributes_before(&lib, &needle);

        assert!(
            !attrs.contains("wasm_bindgen"),
            "`{name}` must remain internal Rust API, not a generated JS/WASM export; attached attrs were: {attrs:?}"
        );
    }

    for required in [
        "#[wasm_bindgen(start)]",
        "pub fn wasm_start",
        "pub fn request_qr_svg",
        "pub fn probe_crypto",
    ] {
        assert!(
            lib.contains(required),
            "expected retained lib.rs public API marker `{required}`"
        );
    }

    for required in [
        "#[wasm_bindgen]",
        "pub struct BrowserVault",
        "pub fn prepare_send",
        "pub fn ingest_anchored_page",
        "pub fn from_backup",
        "pub fn restore",
    ] {
        assert!(
            vault.contains(required),
            "expected retained BrowserVault API marker `{required}`"
        );
    }
}
