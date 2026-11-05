//! Minimal OS signal bridge for the node daemon.
//!
//! A signal handler only sets an atomic flag. Blocking, I/O, lock acquisition,
//! and persistence happen later in the daemon's normal Rust execution context.
use std::sync::atomic::{AtomicBool, Ordering};

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

pub(crate) fn requested() -> bool {
    STOP_REQUESTED.load(Ordering::SeqCst)
}

pub(crate) fn install() -> anyhow::Result<()> {
    STOP_REQUESTED.store(false, Ordering::SeqCst);
    platform::install()
}

#[cfg(unix)]
mod platform {
    use super::{Ordering, STOP_REQUESTED};

    // POSIX signal() takes and returns a signal-handler function pointer.
    // The exact SIGINT/SIGTERM numbers are common across supported Unix
    // targets; no external dependency or lockfile update is needed.
    extern "C" {
        fn signal(number: i32, handler: usize) -> usize;
    }

    extern "C" fn on_signal(_number: i32) {
        STOP_REQUESTED.store(true, Ordering::SeqCst);
    }

    pub(super) fn install() -> anyhow::Result<()> {
        const SIGINT: i32 = 2;
        const SIGTERM: i32 = 15;
        const SIG_ERR: usize = usize::MAX;

        // SAFETY: on_signal is an extern C function with the expected
        // signature and only executes a lock-free atomic store.
        unsafe {
            if signal(SIGINT, on_signal as *const () as usize) == SIG_ERR {
                anyhow::bail!("failed to register SIGINT handler");
            }
            if signal(SIGTERM, on_signal as *const () as usize) == SIG_ERR {
                anyhow::bail!("failed to register SIGTERM handler");
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
mod platform {
    use super::{Ordering, STOP_REQUESTED};

    #[link(name = "kernel32")]
    extern "system" {
        fn SetConsoleCtrlHandler(
            handler: Option<unsafe extern "system" fn(u32) -> i32>,
            add: i32,
        ) -> i32;
    }

    unsafe extern "system" fn on_console_event(event: u32) -> i32 {
        // CTRL_C_EVENT, CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT,
        // CTRL_LOGOFF_EVENT and CTRL_SHUTDOWN_EVENT.
        if matches!(event, 0 | 1 | 2 | 5 | 6) {
            STOP_REQUESTED.store(true, Ordering::SeqCst);
            1
        } else {
            0
        }
    }

    pub(super) fn install() -> anyhow::Result<()> {
        // SAFETY: the callback follows the Win32 console-handler ABI
        // and only changes an atomic flag.
        let success = unsafe { SetConsoleCtrlHandler(Some(on_console_event), 1) };
        if success == 0 {
            anyhow::bail!("failed to register Windows console stop handler");
        }
        Ok(())
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    pub(super) fn install() -> anyhow::Result<()> {
        anyhow::bail!("stop signals are unsupported on this target")
    }
}
