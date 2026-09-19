//! Quiet feedback for money arriving. No extra audio crate: the OS chime
//! is enough, and tests/UI shots must stay silent.

use std::process::{Command, Stdio};

/// A short, quiet coin-in sound. macOS uses the system Glass chime at low
/// volume. Other platforms stay silent rather than pull in an audio stack.
pub fn play_coin_chime() {
    if cfg!(test) || std::env::var_os("NIGHTFALL_UI_SHOTS").is_some() {
        return;
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("afplay")
            .args(["-v", "0.28", "/System/Library/Sounds/Glass.aiff"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn coin_chime_is_silent_in_tests() {
        super::play_coin_chime();
    }
}
