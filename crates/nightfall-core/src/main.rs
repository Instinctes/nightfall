//! NIGHTFALLCOIN Core Wallet — desktop GUI.
//!
//! Runs a full node in-process and a wallet on top of it. Node state and
//! wallet scanning both live off the UI thread; the interface only ever takes
//! short locks to read.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod address_book;
mod app;
mod app_swap;
mod app_swap_lock;
mod app_swap_send;
mod theme;
mod tray;
mod views;
mod views_swap;
mod wallet_state;
mod widgets;
mod widgets_swap;

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

    // One writer per data directory, before anything opens a file in it.
    //
    // `close_to_tray` defaults to on, so the window's X leaves the wallet
    // running; the next launch used to become a *second* process writing the
    // same `blocks.bin`. Two writers produce a chain file neither of them
    // wrote. The guard is held for the whole run — binding it to `_` would
    // drop it here and lock nothing.
    let _dir_lock = match nightfall_storage::dirlock::acquire(&datadir) {
        Ok(lock) => lock,
        Err(e) => {
            tracing::error!("{e}");
            already_running_dialog(&e.to_string());
            std::process::exit(1);
        }
    };

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

/// Say why we are not starting, in a window rather than only in a log.
///
/// A user who double-clicks the icon and sees nothing happen concludes the
/// wallet is broken. The console message reaches nobody on Windows, where
/// this is the platform the problem actually bites on: the binary is built
/// with `windows_subsystem = "windows"` and has no console at all.
fn already_running_dialog(message: &str) {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([520.0, 260.0])
            .with_resizable(false)
            .with_icon(load_window_icon())
            .with_title(format!("{COIN_NAME} Core — already running")),
        ..Default::default()
    };
    let text = message.to_string();
    let _ = eframe::run_simple_native("nightfall-core-busy", options, move |ctx, _frame| {
        theme::apply(ctx);
        eframe::egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(18.0);
            ui.label(
                eframe::egui::RichText::new("Already running")
                    .size(20.0)
                    .strong(),
            );
            ui.add_space(10.0);
            ui.label(eframe::egui::RichText::new(&text).size(13.0));
            ui.add_space(16.0);
            if ui.button("  Close  ").clicked() {
                ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
            }
        });
    });
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
