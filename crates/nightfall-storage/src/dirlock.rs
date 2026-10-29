//! One writer per data directory.
//!
//! Nothing stopped two processes opening the same folder and both writing
//! `blocks.bin`. On Windows that is not a hypothetical: closing the window
//! keeps the wallet running in the tray by default, so the next launch is a
//! *second* process, and the two of them interleave writes into one file.
//! The result is a chain file that neither wrote and neither can read.
//!
//! The lock is held by the operating system, not by a file we write and hope
//! to clean up. That distinction is the whole design:
//!
//! - A PID file survives a crash. The next honest start finds it, cannot
//!   tell a live owner from a dead one, and either refuses to open a wallet
//!   that is perfectly fine, or ignores the file and defeats the point.
//! - An advisory lock (`flock` on Unix, `LockFileEx` on Windows) is released
//!   by the kernel when the holding process dies, however it dies. A crashed
//!   wallet starts again immediately; a running one is still refused.
//!
//! The lock file itself is never deleted. Removing it would race with
//! another process about to lock it. It is a few bytes and it is meant to
//! stay.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Name inside the data directory. Dot-prefixed so it does not clutter a
/// listing next to `blocks.bin` and `wallet.json`.
pub const LOCK_FILE: &str = ".nightfall-lock";

#[derive(Debug)]
pub enum LockError {
    /// Another live process holds this directory.
    Busy { path: PathBuf },
    /// The lock file could not be created or opened at all.
    Io(std::io::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy { path } => write!(
                f,
                "another NIGHTFALLCOIN process is already using {}.\n\
                 Two of them writing the same chain file will corrupt it.\n\
                 Quit the running one first — on Windows it may still be in \
                 the system tray, so use Quit there rather than the window's X.",
                path.display()
            ),
            Self::Io(e) => write!(f, "could not open the data directory lock: {e}"),
        }
    }
}

impl std::error::Error for LockError {}

impl From<std::io::Error> for LockError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Proof that this process owns the directory. Dropping it, or dying, frees
/// the lock.
#[derive(Debug)]
pub struct DirLock {
    _file: File,
    path: PathBuf,
}

impl DirLock {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Take the lock, or say who has it.
///
/// Call this before touching anything in `dir`. The returned guard must be
/// held for as long as the process writes there — binding it to `_` would
/// drop it immediately and lock nothing.
pub fn acquire(dir: &Path) -> Result<DirLock, LockError> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(LOCK_FILE);
    let mut file = open_lock_file(&path)?;

    if !try_lock(&file) {
        return Err(LockError::Busy {
            path: dir.to_path_buf(),
        });
    }

    // Purely for a human reading the file; the lock does not depend on it.
    let _ = file.set_len(0);
    let _ = writeln!(file, "pid {}", std::process::id());
    let _ = file.flush();

    Ok(DirLock { _file: file, path })
}

/// Open the lock entry itself, without following a substituted symlink/reparse
/// point. Validate the open handle before writing any PID, including hardlinks.
fn open_lock_file(path: &Path) -> Result<File, LockError> {
    if let Ok(md) = std::fs::symlink_metadata(path) {
        if md.file_type().is_symlink() {
            return Err(symlink_refused(path));
        }
    }
    let mut opts = OpenOptions::new();
    opts.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        opts.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = opts.open(path).map_err(|e| {
        #[cfg(unix)]
        if e.raw_os_error() == Some(libc::ELOOP) {
            return symlink_refused(path);
        }
        LockError::Io(e)
    })?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(symlink_refused(path));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(symlink_refused(path));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::{fs::MetadataExt, io::AsRawHandle};
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT,
        };
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(symlink_refused(path));
        }
        // SAFETY: file owns this live handle; info is writable and outlives the call.
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0 {
            return Err(LockError::Io(std::io::Error::last_os_error()));
        }
        if info.nNumberOfLinks != 1 {
            return Err(symlink_refused(path));
        }
    }
    Ok(file)
}

fn symlink_refused(path: &Path) -> LockError {
    LockError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!(
            "{} is a symlink, hard link or non-regular lock; refusing to write it",
            path.display()
        ),
    ))
}

#[cfg(unix)]
fn try_lock(file: &File) -> bool {
    use std::os::unix::io::AsRawFd;
    // LOCK_EX | LOCK_NB: exclusive, and fail rather than wait.
    //
    // Waiting would be worse than refusing: a user who double-clicks the icon
    // would get a window that hangs with no explanation instead of a sentence
    // telling them what to do.
    const LOCK_EX_NB: i32 = 2 | 4;
    // SAFETY: `fd` is owned by `file` and stays open for the call.
    unsafe { libc::flock(file.as_raw_fd(), LOCK_EX_NB) == 0 }
}

#[cfg(windows)]
fn try_lock(file: &File) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{
        LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
    };
    use windows_sys::Win32::System::IO::OVERLAPPED;

    let handle = file.as_raw_handle() as HANDLE;
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    // SAFETY: `handle` is owned by `file`; `overlapped` is a valid zeroed
    // structure that outlives the call.
    unsafe {
        LockFileEx(
            handle,
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        ) != 0
    }
}

#[cfg(not(any(unix, windows)))]
fn try_lock(_file: &File) -> bool {
    // No platform lock available. Refusing every start would be worse than
    // the risk on a platform we do not ship.
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir() -> PathBuf {
        // The clock alone is not a unique name: these run in parallel threads
        // of one process and the platform does not hand each of them a
        // distinct nanosecond, so two fixtures race for the same directory and
        // one loses. The counter makes the name unique by construction; the
        // clock stays so that anything left behind is still readable.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!(
            "nf-lock-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_free_directory_can_be_locked() {
        let d = tmpdir();
        let lock = acquire(&d).expect("an unused directory must lock");
        assert!(lock.path().exists());
        let _ = std::fs::remove_dir_all(d);
    }

    /// The bug this exists for: a second process on the same folder.
    ///
    /// Two writers on `blocks.bin` produce a file neither of them wrote.
    /// The second start has to fail, and it has to say why.
    #[test]
    fn a_second_holder_is_refused_while_the_first_lives() {
        let d = tmpdir();
        let first = acquire(&d).unwrap();

        match acquire(&d) {
            Err(LockError::Busy { .. }) => {}
            Err(e) => panic!("expected Busy, got {e}"),
            Ok(_) => panic!("two locks on one directory — the corruption path is open"),
        }

        // The message has to tell a human what to do, not just that
        // something is wrong. The tray is where the running process hides.
        let msg = acquire(&d).unwrap_err().to_string();
        assert!(msg.contains("already using"), "{msg}");
        assert!(
            msg.contains("tray"),
            "the message must point at where the other process is: {msg}"
        );

        drop(first);
        let _ = std::fs::remove_dir_all(d);
    }

    /// A crash must not brick the wallet.
    ///
    /// This is why the lock is held by the kernel rather than by a PID file
    /// we clean up ourselves: dropping the guard — or dying — frees it, and
    /// the stale lock file left behind is not an obstacle.
    #[test]
    fn releasing_the_lock_lets_the_next_start_in() {
        let d = tmpdir();
        let first = acquire(&d).unwrap();
        drop(first);

        let again = acquire(&d).expect("after the holder is gone the directory must lock again");
        assert!(
            again.path().exists(),
            "the lock file stays; deleting it would race with another start"
        );
        let _ = std::fs::remove_dir_all(d);
    }

    /// Two different directories are two different locks.
    #[test]
    fn separate_directories_do_not_block_each_other() {
        let a = tmpdir();
        let b = tmpdir();
        let _la = acquire(&a).unwrap();
        let _lb = acquire(&b).expect("a different directory is a different lock");
        let _ = std::fs::remove_dir_all(a);
        let _ = std::fs::remove_dir_all(b);
    }

    /// A symlinked lock file must not be followed: acquisition would
    /// otherwise write through to the link's target.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_lock_file_is_refused() {
        let d = tmpdir();
        let victim = tmpdir().join("victim");
        std::fs::write(&victim, b"do not touch").unwrap();
        std::os::unix::fs::symlink(&victim, d.join(LOCK_FILE)).unwrap();
        let err = acquire(&d).unwrap_err();
        assert!(err.to_string().contains("symlink"), "{err}");
        assert_eq!(std::fs::read(&victim).unwrap(), b"do not touch");
        let _ = std::fs::remove_dir_all(d);
    }

    /// A directory that does not exist yet is the normal first run.
    #[test]
    fn a_hardlinked_lock_never_overwrites_its_other_name() {
        let dir = tmpdir();
        let victim = dir.join("unrelated.txt");
        std::fs::write(&victim, b"must remain unchanged").unwrap();
        std::fs::hard_link(&victim, dir.join(LOCK_FILE)).unwrap();
        assert!(acquire(&dir).is_err());
        assert_eq!(std::fs::read(&victim).unwrap(), b"must remain unchanged");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_lock_is_refused_without_waiting_for_a_writer() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let dir = tmpdir();
        let path = CString::new(dir.join(LOCK_FILE).as_os_str().as_bytes()).unwrap();
        // SAFETY: path is a NUL-terminated pathname in this disposable directory.
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        assert!(acquire(&dir).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_missing_directory_is_created() {
        let d = tmpdir().join("deeper").join("still-deeper");
        assert!(!d.exists());
        let _lock = acquire(&d).expect("first run must not fail on a missing folder");
        assert!(d.exists());
        let _ = std::fs::remove_dir_all(d);
    }
}
