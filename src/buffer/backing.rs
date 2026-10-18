//! Shared descriptor identity and typed drift for file-backed storage.

use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::time::Duration;
use std::{fmt, io};

/// Longest wait spent letting a just-changed baseline's ctime tick pass before
/// a read that depends on it. Covers one Linux scheduler tick; anything longer
/// (whole-second filesystems, future timestamps) is refused instead of waited.
const SHORT_SETTLE_LIMIT: Duration = Duration::from_millis(15);
/// Linux stamps inode times from a coarse clock that lags the precise clock by
/// at most one scheduler tick (10 ms at the lowest common HZ of 100).
const CLOCK_TICK_GRANULARITY: Duration = Duration::from_millis(11);
const COARSE_SECONDS_GRANULARITY: Duration = Duration::from_secs(2);
const VERIFY_CHUNK_BYTES: usize = 64 * 1024;

#[cfg(test)]
thread_local! {
    /// Replaces the wall clock for change-time decisions made on this thread.
    pub(crate) static TEST_NOW: std::cell::Cell<Option<std::time::SystemTime>> =
        const { std::cell::Cell::new(None) };
    /// Number of times a baseline check slept on this thread.
    pub(crate) static SETTLE_WAITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Moves this thread's wall clock forward for change-time decisions.
    static TEST_CLOCK_AHEAD: std::cell::Cell<Duration> = const { std::cell::Cell::new(Duration::ZERO) };
}

/// Test guard: runs this thread's change-time clock one whole-second
/// granularity (plus margin) ahead, so files a test has just written count as
/// settled even when `TMPDIR` has whole-second timestamps. Only for tests that
/// are not about the pending window; never sleeps.
#[cfg(test)]
#[must_use]
pub(crate) fn settled_clock() -> SettledClock {
    if tmpdir_has_whole_second_ctimes() {
        TEST_CLOCK_AHEAD
            .with(|ahead| ahead.set(COARSE_SECONDS_GRANULARITY + Duration::from_millis(250)));
    }
    SettledClock(())
}

#[cfg(test)]
fn tmpdir_has_whole_second_ctimes() -> bool {
    use std::os::unix::fs::MetadataExt;
    static WHOLE_SECOND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *WHOLE_SECOND.get_or_init(|| {
        let probe =
            std::env::temp_dir().join(format!("catomic-ctime-probe-{}", std::process::id()));
        let whole = (0..3).all(|_| {
            std::fs::write(&probe, b"probe")
                .and_then(|()| std::fs::metadata(&probe))
                .is_ok_and(|metadata| metadata.ctime_nsec() == 0)
        });
        let _ = std::fs::remove_file(&probe);
        whole
    })
}

#[cfg(test)]
pub(crate) struct SettledClock(());

#[cfg(test)]
impl Drop for SettledClock {
    fn drop(&mut self) {
        TEST_CLOCK_AHEAD.with(|ahead| ahead.set(Duration::ZERO));
    }
}

fn now() -> std::time::SystemTime {
    #[cfg(test)]
    if let Some(now) = TEST_NOW.with(std::cell::Cell::get) {
        return now;
    }
    #[cfg(test)]
    return std::time::SystemTime::now() + TEST_CLOCK_AHEAD.with(std::cell::Cell::get);
    #[cfg(not(test))]
    std::time::SystemTime::now()
}

/// Whether a later change to the file is guaranteed a different ctime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChangeTime {
    Settled,
    /// The ctime tick may still be current for this much longer, as judged by
    /// the local clock. Filesystems stamped by a lagging server clock can stay
    /// reusable past this point; that case is not detectable here.
    Pending(Duration),
}

/// Page loading and file-original reads must validate the same source revision.
/// ctime catches in-place rewrites even when their length and mtime are restored,
/// provided the baseline is trusted through [`BaselineTrust`].
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

    pub(crate) fn change_time(&self) -> ChangeTime {
        let Ok(now) = now().duration_since(std::time::UNIX_EPOCH) else {
            return ChangeTime::Settled;
        };
        let Ok(changed_seconds) = u64::try_from(self.changed_seconds) else {
            return ChangeTime::Settled;
        };
        let changed = Duration::new(
            changed_seconds,
            u32::try_from(self.changed_nanoseconds).unwrap_or(0),
        );
        let granularity = if self.changed_nanoseconds == 0 {
            COARSE_SECONDS_GRANULARITY
        } else {
            CLOCK_TICK_GRANULARITY
        };
        match (changed + granularity).checked_sub(now) {
            Some(remaining) if !remaining.is_zero() => ChangeTime::Pending(remaining),
            _ => ChangeTime::Settled,
        }
    }
}

/// Records the exact bytes read against a baseline whose ctime is still
/// pending, so they can be checked again once the tick has passed.
pub(crate) struct RangeDigester {
    keys: std::collections::hash_map::RandomState,
    hasher: std::collections::hash_map::DefaultHasher,
    start: u64,
    end: u64,
}

impl RangeDigester {
    pub(crate) fn new(start: u64) -> Self {
        use std::hash::BuildHasher;
        let keys = std::collections::hash_map::RandomState::new();
        let hasher = keys.build_hasher();
        Self {
            keys,
            hasher,
            start,
            end: start,
        }
    }

    /// Accept one read. Reads must be contiguous or overlap what was seen.
    pub(crate) fn observe(&mut self, offset: usize, bytes: &[u8]) -> io::Result<()> {
        use std::hash::Hasher;
        let offset = offset as u64;
        let read_end = offset + bytes.len() as u64;
        if offset > self.end {
            return Err(io::Error::other("baseline digest read was not contiguous"));
        }
        if read_end > self.end {
            let fresh = usize::try_from(self.end - offset).expect("overlap fits the read");
            self.hasher.write(&bytes[fresh..]);
            self.end = read_end;
        }
        Ok(())
    }

    fn finish(self) -> RangeDigest {
        use std::hash::Hasher;
        RangeDigest {
            keys: self.keys,
            start: self.start,
            end: self.end,
            value: self.hasher.finish(),
        }
    }
}

struct RangeDigest {
    keys: std::collections::hash_map::RandomState,
    start: u64,
    end: u64,
    value: u64,
}

impl RangeDigest {
    fn matches(&self, file: &File) -> io::Result<bool> {
        use std::hash::{BuildHasher, Hasher};
        use std::os::unix::fs::FileExt;
        let mut hasher = self.keys.build_hasher();
        let mut chunk = vec![0u8; VERIFY_CHUNK_BYTES];
        let mut offset = self.start;
        while offset < self.end {
            let want = usize::try_from(self.end - offset)
                .unwrap_or(usize::MAX)
                .min(chunk.len());
            let read = file.read_at(&mut chunk[..want], offset)?;
            if read == 0 {
                return Ok(false);
            }
            hasher.write(&chunk[..read]);
            offset += read as u64;
        }
        Ok(hasher.finish() == self.value)
    }
}

/// How far a descriptor baseline can be trusted to expose later changes.
///
/// A baseline captured inside its ctime tick is provisional: a same-length
/// rewrite with a restored mtime in that tick keeps every compared field equal.
/// Instead of blocking until the tick passes, the bytes read against it are
/// digested. Once the tick has passed, those bytes are read again; a match
/// proves the file still holds that revision and every later change will get
/// a newer ctime, so the baseline is promoted. Reads of new bytes and whole-file
/// writes require a promoted baseline and fail closed otherwise.
pub(crate) struct BaselineTrust {
    provisional: Option<RangeDigest>,
    promoted: std::cell::Cell<bool>,
}

impl BaselineTrust {
    pub(crate) fn settled() -> Self {
        Self {
            provisional: None,
            promoted: std::cell::Cell::new(true),
        }
    }

    pub(crate) fn provisional(digest: RangeDigester) -> Self {
        Self {
            provisional: Some(digest.finish()),
            promoted: std::cell::Cell::new(false),
        }
    }

    /// Promote the baseline if its tick has passed. Never waits and never fails
    /// only because the tick is still pending, so rendering stays responsive.
    pub(crate) fn refresh(&self, snapshot: &DescriptorSnapshot, file: &File) -> io::Result<()> {
        if self.promoted.get() || snapshot.change_time() != ChangeTime::Settled {
            return Ok(());
        }
        self.verify(file)
    }

    /// Require a promoted baseline before reading bytes that were not digested
    /// or writing the whole document. Waits at most one short clock tick.
    pub(crate) fn require(&self, snapshot: &DescriptorSnapshot, file: &File) -> io::Result<()> {
        if self.promoted.get() {
            return Ok(());
        }
        match snapshot.change_time() {
            ChangeTime::Settled => {}
            ChangeTime::Pending(remaining) if remaining <= SHORT_SETTLE_LIMIT => {
                #[cfg(test)]
                SETTLE_WAITS.with(|waits| waits.set(waits.get() + 1));
                std::thread::sleep(remaining);
                if let ChangeTime::Pending(remaining) = snapshot.change_time() {
                    return Err(unsettled_error(remaining));
                }
            }
            ChangeTime::Pending(remaining) => return Err(unsettled_error(remaining)),
        }
        self.verify(file)
    }

    /// Check that `file` (the original, or a private copy of it) holds the
    /// digested bytes right now, without promoting the baseline. Lets Save run
    /// on a provisional baseline whose tick cannot pass soon (a future ctime):
    /// before promotion only the digested bytes have been served.
    pub(crate) fn require_digest_match(&self, file: &File) -> io::Result<()> {
        if self.promoted.get() {
            return Ok(());
        }
        let digest = self
            .provisional
            .as_ref()
            .expect("an unpromoted baseline has a digest");
        if digest.matches(file)? {
            Ok(())
        } else {
            Err(BackingFileChanged::error(
                "file changed within its timestamp granularity after it was opened",
            ))
        }
    }

    fn verify(&self, file: &File) -> io::Result<()> {
        let digest = self
            .provisional
            .as_ref()
            .expect("an unpromoted baseline has a digest");
        if digest.matches(file)? {
            self.promoted.set(true);
            Ok(())
        } else {
            Err(BackingFileChanged::error(
                "file changed within its timestamp granularity after it was opened",
            ))
        }
    }
}

fn unsettled_error(remaining: Duration) -> io::Error {
    let message = if remaining > COARSE_SECONDS_GRANULARITY {
        "File change time is in the future, so later external changes cannot be detected. \
         Other pages, search, and go to line stay unavailable until the clock passes it; \
         Save still works."
            .to_string()
    } else {
        format!(
            "File changed moments ago; try again in {:.1}s once its timestamp settles.",
            remaining.as_secs_f64().max(0.1)
        )
    };
    io::Error::new(io::ErrorKind::WouldBlock, BaselinePending(message))
}

/// A read was refused because the baseline's ctime tick has not passed yet.
/// Retrying later can succeed; nothing about the file is known to be wrong.
#[derive(Debug)]
pub(crate) struct BaselinePending(String);

impl BaselinePending {
    pub(crate) fn is(error: &io::Error) -> bool {
        error.get_ref().is_some_and(|cause| cause.is::<Self>())
    }
}

impl fmt::Display for BaselinePending {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for BaselinePending {}

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
mod tests;
