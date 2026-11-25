//! Capped reads and atomic writes for the device-local sync files, through `fernjo-fs`.
//!
//! The files are device-local and rebuilt by the next sync, so there is no `.bak`; they are
//! owner-only and their folder is created private.

use std::io;
use std::path::Path;

use fernjo_core::MAX_DATA_FILE_LEN;
use fernjo_fs::{FileMode, Parent, WriteOptions};

/// Reads `path` unless it is longer than [`MAX_DATA_FILE_LEN`] (`InvalidData`, "the file is larger
/// than 64 MiB": the text reaches the user as the reason of an unusable state file).
pub(crate) fn read_capped(path: &Path) -> io::Result<Vec<u8>> {
    fernjo_fs::read_capped(path, MAX_DATA_FILE_LEN).map_err(|e| {
        // `fernjo_fs` reports only oversize as `InvalidData`
        if e.kind() == io::ErrorKind::InvalidData {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "the file is larger than {} MiB",
                    MAX_DATA_FILE_LEN / (1024 * 1024)
                ),
            )
        } else {
            e
        }
    })
}

/// Writes `bytes` to `path` through a random temp file and a rename; no `.bak`.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    fernjo_fs::write_atomic(
        path,
        bytes,
        WriteOptions {
            parent: Parent::CreatePrivate,
            backup: false,
            file_mode: FileMode::OwnerOnly,
        },
    )
}
