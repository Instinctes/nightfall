//! Quiet feedback for money arriving. No extra audio crate: the OS chime
//! is enough, and tests/UI shots must stay silent.

/// A short, quiet coin-in sound. macOS uses the system Glass chime at low
/// volume. Other platforms stay silent rather than pull in an audio stack.
///
/// Everything platform-specific lives inside the `cfg` block, including the
/// import and the silence guard. Written the obvious way — a top-level `use`
/// and an early `return` — this file compiled cleanly on macOS and failed CI
/// on Linux with two errors, because there the `cfg` block disappears and
/// leaves an unused import above a function whose whole body is a bare
/// trailing `return`. A guard that only guards code which does not exist on
/// this platform is not a guard; it is the thing being linted.
pub fn play_coin_chime() {
    #[cfg(target_os = "macos")]
    {
        use std::process::{Command, Stdio};

        if cfg!(test) || std::env::var_os("NIGHTFALL_UI_SHOTS").is_some() {
            return;
        }
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
