//! The one hardened atomic file writer and capped file reader of fernjo.
//!
//! Every crate that writes or reads its own files (the data folder, the device folder, the
//! hub store) goes through these helpers, so a fix lands in one place. The crate never depends on
//! Tauri, so every crate may use it.
//!
//! [`write_atomic`] writes to a fresh `<file>.<16 hex>.tmp` opened with `create_new` (never an
//! existing file, never through a symlink at that name; mode 0600 on Unix for
//! [`FileMode::OwnerOnly`]), flushes and syncs it, optionally keeps the previous content as
//! `<file>.bak`, renames the temp file onto the target and, on Unix, syncs the folder (best
//! effort). The `.bak` name is never opened for writing: the previous content is copied into a
//! second fresh temp file that is renamed onto `<file>.bak`, so a symlink planted at that name is
//! replaced instead of followed.
//!
//! Neither helper follows a symlink at the target name itself: [`read_capped`] and the `.bak`
//! copy open only a regular file (`O_NOFOLLOW` and `O_NONBLOCK` on Unix, a `symlink_metadata`
//! check everywhere, then the opened handle's type), and [`write_atomic`] refuses a target that
//! exists but is not a regular file. A planted link to a private file, to `/dev/zero` or to a FIFO
//! is therefore never read, copied into `<file>.bak` or blocked on. The crate depends only on `std`,
//! `getrandom` and, on Unix, `libc` (for the open flags).

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests;

/// Text of the error for a path that is not a regular file.
pub(crate) const NOT_REGULAR: &str =
    "not a regular file (a symbolic link, a folder or a special file)";

/// How many random temp names [`write_atomic`] tries before it gives up.
const TEMP_ATTEMPTS: usize = 8;

/// What happens to a missing parent folder of the target. A folder that already exists is never
/// changed (no chmod), whichever variant is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parent {
    /// Create nothing; a missing parent fails the write with `NotFound`.
    MustExist,
    /// Create missing folders, with mode 0700 on Unix.
    CreatePrivate,
    /// Create missing folders with the default mode (umask on Unix).
    CreateDefault,
}

/// Mode of the written file (and of its `.bak`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileMode {
    /// Mode 0600 on Unix; elsewhere the folder's inherited ACL applies.
    OwnerOnly,
    /// The default mode (umask on Unix).
    Default,
}

/// What differs between the callers of [`write_atomic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteOptions {
    pub parent: Parent,
    /// Keep the previous content as `<file>.bak` when the target exists.
    pub backup: bool,
    pub file_mode: FileMode,
}

impl WriteOptions {
    /// The common case for a data file: private parent folders, a `.bak`, owner-only mode.
    pub const fn data_file() -> WriteOptions {
        WriteOptions {
            parent: Parent::CreatePrivate,
            backup: true,
            file_mode: FileMode::OwnerOnly,
        }
    }
}

/// `path` with `suffix` appended to its last component (`x.json` + `.bak` is `x.json.bak`).
pub fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Reads a whole file, but never more than `max` bytes. The length is checked up front (so an
/// oversized file is refused without allocating it) and the read itself is bounded too, so a
/// file that grows between the check and the read cannot bypass the cap. Oversize is an
/// `InvalidData` error with the text "the file is too large"; a missing file keeps its
/// `NotFound` kind.
///
/// The file is opened with [`open_regular`]: a symlink, a folder or a special file at `path` is
/// refused with the error "not a regular file …" (never `InvalidData`, never `NotFound`, also for
/// a dangling link), so a planted link to another file, to `/dev/zero` or to a FIFO is never read.
pub fn read_capped(path: &Path, max: u64) -> io::Result<Vec<u8>> {
    let too_large = || io::Error::new(io::ErrorKind::InvalidData, "the file is too large");
    let file = open_regular(path)?;
    let len = file.metadata()?.len();
    if len > max {
        return Err(too_large());
    }
    // `len <= max`, so the reservation is bounded by the cap (a smaller reservation only costs a
    // reallocation)
    let mut bytes = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
    file.take(max.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(too_large());
    }
    Ok(bytes)
}

/// Opens `path` for reading only when it is a regular file, without following a symlink at its
/// last component (links in the folders above are followed as usual):
/// - every platform: `symlink_metadata` must say "regular file" before the open (on Windows a
///   symlink or junction is refused; other reparse points such as OneDrive placeholders count as
///   regular files, as `std` reports them);
/// - Unix: the open itself uses `O_NOFOLLOW` (a link swapped in after the check fails with
///   `ELOOP`) and `O_NONBLOCK` (a FIFO or device swapped in does not block the open; the flag has
///   no effect on reading a regular file);
/// - every platform: the opened handle's metadata must say "regular file" again, which is the
///   race-free part on Unix.
///
/// The Windows check is a check-then-open (a link swapped in between is followed); creating a
/// symlink there needs admin rights or developer mode.
pub(crate) fn open_regular(path: &Path) -> io::Result<File> {
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(not_regular());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.file_type().is_file() {
        return Err(not_regular());
    }
    Ok(file)
}

fn not_regular() -> io::Error {
    io::Error::other(NOT_REGULAR)
}

/// Writes `bytes` exactly (nothing added) to `path` atomically; see the crate documentation for
/// the steps. On every failure the temp files this call created are removed and the target is
/// left as it was. A failed folder sync after the rename does not fail the write.
///
/// A target that exists but is not a regular file (a symlink, also a dangling one, a folder, a
/// FIFO) is refused with the "not a regular file …" error before anything is created: a link is
/// neither followed nor silently replaced. The `.bak` step opens the current file with
/// [`open_regular`] as well, so a link swapped in after this check is still never copied.
pub fn write_atomic(path: &Path, bytes: &[u8], options: WriteOptions) -> io::Result<()> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(dir) = dir {
        match options.parent {
            Parent::MustExist => {}
            Parent::CreatePrivate => create_dir_all(dir, true)?,
            Parent::CreateDefault => create_dir_all(dir, false)?,
        }
    }
    // an error here (a missing parent, no permission) is left to the steps below, as before
    if fs::symlink_metadata(path).is_ok_and(|meta| !meta.file_type().is_file()) {
        return Err(not_regular());
    }
    let (tmp, mut file) = create_temp_file(path, options.file_mode)?;
    let result = (|| -> io::Result<()> {
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        if options.backup {
            keep_backup(path, options.file_mode)?;
        }
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
        return result;
    }
    // the file is in place; a failed folder sync must not report the write as failed
    if let Some(dir) = dir {
        let _ = sync_dir(dir);
    }
    Ok(())
}

/// Copies the current content of `path` (when it exists) into a fresh temp file and renames that
/// onto `<path>.bak`. The `.bak` name itself is never opened, so a symlink there is replaced by
/// the rename, not followed. The current file is opened with [`open_regular`], so a symlink or
/// special file at `path` itself is refused instead of copied.
fn keep_backup(path: &Path, mode: FileMode) -> io::Result<()> {
    let mut current = match open_regular(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let (tmp, mut file) = create_temp_file(path, mode)?;
    let result = (|| -> io::Result<()> {
        io::copy(&mut current, &mut file)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, with_suffix(path, ".bak"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Creates `dir` and its missing parents; with `private`, mode 0700 on Unix. Folders that already
/// exist keep their mode.
fn create_dir_all(dir: &Path, private: bool) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    #[cfg(not(unix))]
    let _ = private;
    builder.create(dir)
}

/// Creates a new file (`create_new`: never an existing file, never through a symlink at that
/// name); for [`FileMode::OwnerOnly`] readable and writable by its owner only on Unix (mode
/// 0600), elsewhere the folder's inherited ACL applies.
fn create_new_file(path: &Path, mode: FileMode) -> io::Result<File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if mode == FileMode::OwnerOnly {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = mode;
    options.open(path)
}

/// A temp file next to `path` under a random `<file>.<16 hex>.tmp` name, so nobody can plant the
/// name beforehand; a name that is taken anyway is never reused.
fn create_temp_file(path: &Path, mode: FileMode) -> io::Result<(PathBuf, File)> {
    create_temp_file_from(
        || {
            let mut random = [0u8; 8];
            getrandom::fill(&mut random).map_err(|e| io::Error::other(e.to_string()))?;
            let hex: String = random.iter().map(|b| format!("{b:02x}")).collect();
            Ok(with_suffix(path, &format!(".{hex}.tmp")))
        },
        mode,
    )
}

/// Tries up to [`TEMP_ATTEMPTS`] names from `next_name`, skipping every name that is taken
/// (a file or a symlink); any other error ends the attempt. Split out so a test can force the
/// names.
fn create_temp_file_from(
    mut next_name: impl FnMut() -> io::Result<PathBuf>,
    mode: FileMode,
) -> io::Result<(PathBuf, File)> {
    let mut last = None;
    for _ in 0..TEMP_ATTEMPTS {
        let tmp = next_name()?;
        match create_new_file(&tmp, mode) {
            Ok(file) => return Ok((tmp, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("no free temp file name")))
}

/// Unix: flushes the folder entry of the rename to disk. Elsewhere nothing to do.
fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(dir)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}
