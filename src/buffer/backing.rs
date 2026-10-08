//! Shared descriptor identity and typed drift for file-backed storage.

use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::time::{Duration, Instant};
use std::{fmt, io};

/// Upper bound for waiting out a just-changed file's timestamp granularity.
/// Covers two-second filesystem timestamps; ordinary Linux ticks take 1-10 ms.
const SETTLE_LIMIT: Duration = Duration::from_millis(2_100);
const SETTLE_POLL: Duration = Duration::from_millis(1);
/// Linux stamps inode times from a coarse clock that lags the precise clock by
/// at most one scheduler tick (10 ms at the lowest common HZ of 100).
const CLOCK_TICK_GRANULARITY: Duration = Duration::from_millis(11);
const COARSE_SECONDS_GRANULARITY: Duration = Duration::from_secs(2);

/// Page loading and file-original reads must validate the same source revision.
/// ctime catches in-place rewrites even when their length and mtime are restored,
/// provided the baseline was captured with [`DescriptorSnapshot::capture_settled`].
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

    /// Capture a baseline that a later change cannot reproduce. Linux stamps
    /// ctime from a coarse clock, so a same-length rewrite with a restored mtime
    /// inside the same tick as the change before open keeps every compared field
    /// identical. Waiting until that tick has passed, before any bytes are read
    /// against the baseline, guarantees later changes receive a newer ctime.
    /// A timestamp that does not settle within the limit (for example one in
    /// the future) is returned as captured.
    pub(crate) fn capture_settled(file: &File) -> io::Result<Self> {
        let deadline = Instant::now() + SETTLE_LIMIT;
        loop {
            let snapshot = Self::capture(file)?;
            let now = Instant::now();
            if snapshot.change_time_settled() || now >= deadline {
                return Ok(snapshot);
            }
            std::thread::sleep(SETTLE_POLL.min(deadline - now));
        }
    }

    /// Whether any later change must receive a ctime different from this one.
    fn change_time_settled(&self) -> bool {
        let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
            return true;
        };
        let Ok(changed_seconds) = u64::try_from(self.changed_seconds) else {
            return true;
        };
        let changed = Duration::new(
            changed_seconds,
            u32::try_from(self.changed_nanoseconds).unwrap_or(0),
        );
        let granularity = if self.changed_nanoseconds == 0 {
            // Possibly a whole-second (or two-second) timestamp filesystem.
            COARSE_SECONDS_GRANULARITY
        } else {
            CLOCK_TICK_GRANULARITY
        };
        now >= changed + granularity
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, Write};

    #[test]
    fn settled_baseline_exposes_an_immediate_same_length_rewrite_with_restored_mtime() {
        let path = std::env::temp_dir().join(format!(
            "catomic_settled_snapshot_{}.txt",
            std::process::id()
        ));
        // Without settling, a rewrite in the same coarse clock tick as the
        // fixture write keeps len, mtime, and ctime identical; repeat to make
        // that same-tick timing overwhelmingly likely on every run.
        for _ in 0..50 {
            std::fs::write(&path, "first\nsecond\nthird").unwrap();
            let mut external = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .unwrap();
            let modified = external.metadata().unwrap().modified().unwrap();
            let started = Instant::now();
            let baseline = DescriptorSnapshot::capture_settled(&external).unwrap();
            assert!(
                started.elapsed() <= SETTLE_LIMIT + Duration::from_millis(100),
                "settling is bounded"
            );

            external.rewind().unwrap();
            external.write_all(b"FIRST\nSECOND\nTHIRD").unwrap();
            external.set_modified(modified).unwrap();

            let after = DescriptorSnapshot::capture(&external).unwrap();
            assert_eq!(after.len, baseline.len);
            assert_eq!(after.modified, baseline.modified);
            assert_ne!(after, baseline, "the rewrite must receive a newer ctime");
        }
        std::fs::remove_file(path).unwrap();
    }
}
