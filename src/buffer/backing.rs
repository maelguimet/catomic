//! Shared descriptor identity and typed drift for file-backed storage.

use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::{fmt, io};

/// Page loading and file-original reads must validate the same source revision.
/// ctime catches in-place rewrites even when their length and mtime are restored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DescriptorSnapshot {
    pub(crate) len: u64,
    modified: Option<std::time::SystemTime>,
    device: u64,
    inode: u64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

impl DescriptorSnapshot {
    pub(crate) fn capture(file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        Ok(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            device: metadata.dev(),
            inode: metadata.ino(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        })
    }
}

#[derive(Debug)]
pub(crate) struct BackingFileChanged(&'static str);

impl BackingFileChanged {
    pub(crate) fn error(message: &'static str) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, Self(message))
    }

    pub(crate) fn is(error: &io::Error) -> bool {
        error.get_ref().is_some_and(|cause| cause.is::<Self>())
    }
}

impl fmt::Display for BackingFileChanged {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for BackingFileChanged {}
