//! The filesystem primitives the vault adapter needs, one implementation per
//! platform.
//!
//! Everything else in `vault_store` — the order of operations, the checkpoints,
//! the `unusable` transitions, the verification after every write — is shared.
//! Only the calls below differ, and keeping the difference to this list is the
//! point: a platform cannot quietly acquire its own idea of when a write is
//! safe.
//!
//! ## What Windows cannot do, stated rather than papered over
//!
//! | Unix | Windows |
//! |---|---|
//! | `create_new` + `O_EXCL` | `CREATE_NEW`, equivalent |
//! | `fsync` | `FlushFileBuffers`, equivalent — `File::sync_all` maps to it |
//! | `fsync` on the directory | **no equivalent.** A directory cannot be
//!   opened for writing, so there is nothing to flush. The rename asks for
//!   `MOVEFILE_WRITE_THROUGH` instead, and Microsoft documents that flag's
//!   guarantee for a copy-and-delete move — *not* for a rename within one
//!   volume. So the commit is atomic on Windows and its durability is weaker
//!   than on Unix. That is a real difference and it is not hidden. |
//! | `mode(0o600)` / `mode(0o700)` | **no equivalent here.** Files inherit the
//!   ACL of the user profile directory. That keeps other *users* out on a
//!   normal installation and does not keep out an administrator, and this
//!   module does not set a DACL of its own: an access-control implementation
//!   that cannot be tested is worse than a documented gap, because it reads
//!   like a guarantee. See `docs/WALLET-1.0.0-PLAN.md`. |
//! | `O_NOFOLLOW` | no equivalent when opening. The symlink and reparse-point
//!   check in `ensure_regular` runs first; the gap is the moment between the
//!   check and the open, which `O_NOFOLLOW` closes and Windows leaves open. |
//! | `nlink == 1` | `GetFileInformationByHandleEx(FileStandardInfo)`,
//!   equivalent. NTFS has hard links and unprivileged users can create them,
//!   so this check matters just as much there. |
//!
//! None of this is a power-loss test on either platform.

use std::fs::{self, File, Metadata, OpenOptions};
use std::path::Path;

/// Whether this platform has a real implementation below.
///
/// The adapter refuses to open a wallet at all when this is false, rather than
/// running with silently weaker rules.
pub(crate) const SUPPORTED: bool = cfg!(any(unix, windows));

/// Create a directory that other users of the machine should not read.
pub(crate) fn create_private_dir(path: &Path) -> std::io::Result<()> {
    #[allow(unused_mut)] // Only Unix carries a creation mode.
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

/// Create a file that must not exist yet, as privately as the platform allows.
///
/// `create_new` is the load-bearing part on both: an existing file makes this
/// fail rather than be replaced, which is what keeps a backup export from
/// overwriting someone's only copy.
pub(crate) fn create_private_new(path: &Path) -> std::io::Result<File> {
    #[allow(unused_mut)]
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    options.open(path)
}

/// Open for reading without following a link, where the platform can say so.
pub(crate) fn open_read_no_links(path: &Path) -> std::io::Result<File> {
    #[allow(unused_mut)]
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    options.open(path)
}

/// True when this path is a link, a junction, or anything else that is not
/// simply the file it appears to be.
///
/// On Windows this deliberately rejects every reparse point, not only the ones
/// std reports as symlinks. A cloud-storage placeholder is not a symlink and is
/// not a plain file either, and a wallet inside one is a wallet whose contents
/// another program decides to materialise or evict.
pub(crate) fn is_link_like(meta: &Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

/// How many names this open file has. More than one means someone else can
/// reach the same bytes under a name we never checked.
#[cfg(unix)]
pub(crate) fn link_count(file: &File) -> std::io::Result<u64> {
    use std::os::unix::fs::MetadataExt;
    Ok(file.metadata()?.nlink())
}

#[cfg(windows)]
pub(crate) fn link_count(file: &File) -> std::io::Result<u64> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileStandardInfo, GetFileInformationByHandleEx, FILE_STANDARD_INFO,
    };
    // SAFETY: the handle comes from a live `File`, the information class
    // matches the struct being filled, and the size given is that struct's.
    let mut info: FILE_STANDARD_INFO = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle() as _,
            FileStandardInfo,
            std::ptr::addr_of_mut!(info).cast(),
            std::mem::size_of::<FILE_STANDARD_INFO>() as u32,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(u64::from(info.NumberOfLinks))
}

/// Replace `to` with `from` in one step.
///
/// The rename is what publishes a new snapshot, so it must be all-or-nothing:
/// a reader either sees the old file or the new one, never a half-written one.
/// Both platforms give that. They do not give the same durability — see the
/// table at the top of this file.
#[cfg(not(windows))]
pub(crate) fn commit_rename(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::rename(from, to)
}

#[cfg(windows)]
pub(crate) fn commit_rename(from: &Path, to: &Path) -> std::io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    fn wide(path: &Path) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }
    let (source, target) = (wide(from), wide(to));
    // SAFETY: both buffers are null-terminated and outlive the call.
    let ok = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok != 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    // ERROR_SHARING_VIOLATION. It has no Unix counterpart and reads as
    // gibberish on its own, so say what it means: something else holds the
    // file open in a way that forbids replacing it.
    if error.raw_os_error() == Some(32) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "another program is holding the wallet file open, so it could not \
             be replaced. Antivirus, backup and search indexing tools do this. \
             Nothing was changed; close them or exclude the wallet folder, \
             then try again.",
        ));
    }
    Err(error)
}

/// Make the rename itself durable, where the platform offers a way.
///
/// On Unix that is an `fsync` of the directory. On Windows there is nothing to
/// call: the request went with the rename. Returning `Ok` here is not a claim
/// that the two are equal, and the caller's checkpoint fires either way so the
/// failure-injection tests cover the same points on both.
#[cfg(unix)]
pub(crate) fn sync_dir_after_commit(path: &Path) -> std::io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
pub(crate) fn sync_dir_after_commit(_path: &Path) -> std::io::Result<()> {
    Ok(())
}
