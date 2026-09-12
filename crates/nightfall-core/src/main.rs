//! NIGHTFALLCOIN Core Wallet — desktop GUI.
//!
//! Runs a full node in-process and a wallet on top of it. Node state and
//! wallet scanning both live off the UI thread; the interface only ever takes
//! short locks to read.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod address_book;
mod app;
mod app_swap;
mod app_swap_drive;
mod app_swap_lock;
mod app_swap_night;
mod app_swap_send;
mod backup_recovery;
mod onboarding;
mod recovery_studio;
#[cfg(test)]
mod swap_live_tests;
mod swap_worker;
mod theme;
mod tray;
#[cfg(all(test, unix))]
mod vault_node_tests;
mod vault_ui;
#[cfg(test)]
mod view_layout_tests;
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
    if app::IS_DEV_BUILD && !app::IS_DEV_MAINNET && network != NetworkId::Devnet {
        eprintln!("This development build only supports devnet. No wallet was opened.");
        std::process::exit(2);
    }
    let datadir = parse_datadir_arg().unwrap_or_else(|| {
        let path = default_data_dir(network);
        // A mainnet development build uses the real directory on purpose —
        // opening the operator's own wallet is the whole reason it exists — so
        // it must not be sent to the `wallet-1.0-dev` sandbox, which would
        // silently present an empty wallet instead.
        if app::IS_DEV_BUILD && !app::IS_DEV_MAINNET {
            path.join("wallet-1.0-dev")
        } else {
            path
        }
    });
    if app::IS_DEV_MAINNET {
        eprintln!(
            "Development build with mainnet enabled. Once this saves, the released \
             0.9.5 app can no longer read this wallet file — restore an encrypted \
             backup if you need to go back."
        );
    }

    tracing::info!("{COIN_NAME} Core — {network} — {}", datadir.display());

    // One writer per data directory, before anything opens a file in it.
    //
    // `close_to_tray` defaults to on, so the window's X leaves the wallet
    // running; the next launch used to become a *second* process writing the
    // same `blocks.bin`. Two writers produce a chain file neither of them
    // wrote. The guard is held for the whole run — binding it to `_` would
    // drop it here and lock nothing.
    let dir_lock = match nightfall_storage::dirlock::acquire(&datadir) {
        Ok(lock) => std::sync::Arc::new(lock),
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
            .with_title(format!(
                "{COIN_NAME} Core {} — {network}",
                app::WALLET_VERSION
            )),
        ..Default::default()
    };

    eframe::run_native(
        "nightfall-core",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(App::with_data_lock(network, dir_lock)))
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
    default_network(app::WALLET_VERSION)
}

fn default_network(version: &str) -> NetworkId {
    if version.contains('-') {
        NetworkId::Devnet
    } else {
        NetworkId::Mainnet
    }
}

#[test]
fn prereleases_default_to_isolated_devnet() {
    assert_eq!(default_network("0.9.4-dev.2"), NetworkId::Devnet);
    assert_eq!(default_network("0.9.2"), NetworkId::Mainnet);
}

/// The mainnet permission belongs to a build, not to a command line.
///
/// An ordinary development build must refuse `--network mainnet` however it is
/// spelled, so a copy that leaves the operator's machine cannot be talked onto
/// the real network by an argument. Only a binary compiled with
/// `NIGHTFALL_DEV_MAINNET` may, and that one says so on every page.
#[test]
fn a_development_build_opens_mainnet_only_when_it_was_built_to() {
    // This test binary is itself built without the flag, which is the ordinary
    // case and the one worth pinning.
    assert!(
        !app::IS_DEV_MAINNET,
        "the default build must not carry the mainnet permission",
    );
    // …and the gate is written in terms of that constant, not of an argument.
    let gate = |is_dev: bool, dev_mainnet: bool, network: NetworkId| {
        is_dev && !dev_mainnet && network != NetworkId::Devnet
    };
    assert!(gate(true, false, NetworkId::Mainnet), "dev build must refuse mainnet");
    assert!(gate(true, false, NetworkId::Testnet), "dev build must refuse testnet");
    assert!(!gate(true, false, NetworkId::Devnet));
    assert!(!gate(true, true, NetworkId::Mainnet), "an opted-in build may");
    assert!(!gate(false, false, NetworkId::Mainnet), "a release build may");
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
