//! Capped reads and atomic writes for the device-local sync files.
//!
//! `fernjo_core::persist::read_capped` is crate-private and `write_exact` uses a fixed
//! `<file>.tmp` name opened with `File::create`, which a planted symlink could redirect. These
//! are small copies of the hardened helpers in `fernjo-vault` (random `create_new` temp name,
//! mode 0600 on Unix, directory fsync on Unix); `fernjo-sync` has no access to the private ones.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use fernjo_core::MAX_DATA_FILE_LEN;

/// Reads `path` unless it is longer than [`MAX_DATA_FILE_LEN`] (`InvalidData`); the read itself
/// is bounded in case the file grows meanwhile.
pub(crate) fn read_capped(path: &Path) -> io::Result<Vec<u8>> {
    let too_large = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "the file is larger than {} MiB",
                MAX_DATA_FILE_LEN / (1024 * 1024)
            ),
        )
    };
    let file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len > MAX_DATA_FILE_LEN {
        return Err(too_large());
    }
    let mut bytes = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
    file.take(MAX_DATA_FILE_LEN.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_DATA_FILE_LEN {
        return Err(too_large());
    }
    Ok(bytes)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn create_private_dir_all(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

fn create_new_private_file(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn create_temp_file(path: &Path) -> io::Result<(PathBuf, fs::File)> {
    let mut last = None;
    for _ in 0..8 {
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).map_err(|e| io::Error::other(e.to_string()))?;
        let hex: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let tmp = with_suffix(path, &format!(".{hex}.tmp"));
        match create_new_private_file(&tmp) {
            Ok(file) => return Ok((tmp, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("no free temp file name")))
}

/// Writes `bytes` to `path` through a random temp file and a rename; no `.bak` (the files are
/// device-local and rebuilt by the next sync).
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(dir) = dir {
        create_private_dir_all(dir)?;
    }
    let (tmp, mut file) = create_temp_file(path)?;
    let result = (|| -> io::Result<()> {
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
        return result;
    }
    #[cfg(unix)]
    if let Some(dir) = dir {
        let _ = fs::File::open(dir).and_then(|d| d.sync_all());
    }
    Ok(())
}
