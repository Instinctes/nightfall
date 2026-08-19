//! Close-to-tray so mining survives the window being closed.
//!
//! Windows has no dock: hide the window without a tray icon and the process
//! looks dead. macOS keeps the dock icon; the tray is still the obvious
//! "I am mining" tell. Linux Core users typically leave the window open.

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod imp {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    };

    pub enum TrayAction {
        Show,
        Quit,
    }

    pub struct Tray {
        _icon: TrayIcon,
        show: MenuItem,
        quit: MenuItem,
    }

    impl Tray {
        pub fn new() -> Option<Self> {
            let icon = load_icon()?;
            let show = MenuItem::new("Show NIGHTFALL Core", true, None);
            let quit = MenuItem::new("Quit", true, None);
            let menu = Menu::new();
            menu.append(&show).ok()?;
            menu.append(&PredefinedMenuItem::separator()).ok()?;
            menu.append(&quit).ok()?;
            let tray = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_tooltip("NIGHTFALLCOIN Core")
                .with_icon(icon)
                .build()
                .ok()?;
            Some(Self {
                _icon: tray,
                show,
                quit,
            })
        }

        pub fn poll(&self) -> Option<TrayAction> {
            if let Ok(event) = MenuEvent::receiver().try_recv() {
                if event.id == self.show.id() {
                    return Some(TrayAction::Show);
                }
                if event.id == self.quit.id() {
                    return Some(TrayAction::Quit);
                }
            }
            if let Ok(
                TrayIconEvent::DoubleClick { .. }
                | TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                },
            ) = TrayIconEvent::receiver().try_recv()
            {
                return Some(TrayAction::Show);
            }
            None
        }
    }

    fn load_icon() -> Option<Icon> {
        const BYTES: &[u8] = include_bytes!("../assets/logo.png");
        let img = image::load_from_memory(BYTES).ok()?.into_rgba8();
        let resized = image::imageops::resize(&img, 32, 32, image::imageops::FilterType::Triangle);
        let (w, h) = resized.dimensions();
        Icon::from_rgba(resized.into_raw(), w, h).ok()
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub use imp::{Tray, TrayAction};

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
#[allow(dead_code)]
pub enum TrayAction {
    Show,
    Quit,
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub struct Tray;

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
impl Tray {
    pub fn new() -> Option<Self> {
        None
    }
    pub fn poll(&self) -> Option<TrayAction> {
        None
    }
}
