//! NIGHTFALLCOIN Core Wallet — desktop GUI.
//!
//! Runs a full node in-process and a wallet on top of it. Node state and
//! wallet scanning both live off the UI thread; the interface only ever takes
//! short locks to read.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod theme;
mod views;
mod wallet_state;
mod widgets;

use app::App;
use nightfall_storage::default_data_dir;
use nightfall_types::{NetworkId, COIN_NAME};
use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()))
        .with_target(false)
        .init();

    let network = parse_network_arg();
    let datadir = parse_datadir_arg().unwrap_or_else(|| default_data_dir(network));

    tracing::info!("{COIN_NAME} Core — {network} — {}", datadir.display());

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([940.0, 620.0])
            .with_icon(load_window_icon())
            .with_title(format!("{COIN_NAME} Core — {network}")),
        ..Default::default()
    };

    eframe::run_native(
        "nightfall-core",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(App::new(network, datadir)))
        }),
    )
}

/// Window and taskbar icon. On macOS the bundle's `.icns` takes precedence,
/// but on Windows and Linux this is the only icon the window manager sees.
fn load_window_icon() -> eframe::egui::IconData {
    const BYTES: &[u8] = include_bytes!("../assets/logo-512.png");
    let image = image::load_from_memory(BYTES)
        .expect("bundled logo is valid PNG")
        .into_rgba8();
    let (width, height) = image.dimensions();
    eframe::egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    }
}

fn parse_network_arg() -> NetworkId {
    let args: Vec<String> = std::env::args().collect();
    for (i, a) in args.iter().enumerate() {
        if a == "--network" {
            if let Some(v) = args.get(i + 1) {
                return match v.as_str() {
                    "mainnet" => NetworkId::Mainnet,
                    "testnet" => NetworkId::Testnet,
                    _ => NetworkId::Devnet,
                };
            }
        }
        if let Some(v) = a.strip_prefix("--network=") {
            return match v {
                "mainnet" => NetworkId::Mainnet,
                "testnet" => NetworkId::Testnet,
                _ => NetworkId::Devnet,
            };
        }
    }
    NetworkId::Mainnet
}

fn parse_datadir_arg() -> Option<PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    for (i, a) in args.iter().enumerate() {
        if a == "--datadir" {
            return args.get(i + 1).map(PathBuf::from);
        }
        if let Some(v) = a.strip_prefix("--datadir=") {
            return Some(PathBuf::from(v));
        }
    }
    None
}
