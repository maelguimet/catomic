//! Purpose: own private `.catnap` sidecar paths, bounded reads, and async writes.
//! Owns: race-safe candidate loading, identity checks, atomic 0600 writes, cleanup, and tasks.
//! Must not: decide editor timing, mutate buffers, render UI, or recover automatically.
//! Invariants: sidecars append `.catnap`; Unix reads are no-follow/nonblocking, retained, and capped.
//! Phase: 8 opt-in crash recovery.

use std::ffi::OsString;
use std::fs::{File, Metadata, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

#[derive(Debug)]
pub(crate) enum CatnapResult {
    Written { path: PathBuf, history: u64 },
    Error(String),
}

pub(crate) struct CatnapTask {
    receiver: Receiver<CatnapResult>,
}

pub(crate) struct RecoveryCandidate {
    file: File,
    text: String,
    identity: SidecarIdentity,
    max_bytes: usize,
}

impl RecoveryCandidate {
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn is_current(&mut self, original: &Path) -> io::Result<bool> {
        let path = catnap_path(original);
        let metadata = self.file.metadata()?;
        if SidecarIdentity::from(&metadata) != self.identity {
            return Ok(false);
        }
        self.file.seek(SeekFrom::Start(0))?;
        let (text, identity) = read_opened(&mut self.file, metadata, self.max_bytes)?;
        Ok(identity == self.identity
            && text == self.text
            && entry_matches_identity(&path, &identity)?)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct SidecarIdentity {
    len: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    changed_seconds: i64,
    #[cfg(unix)]
    changed_nanoseconds: i64,
}

impl SidecarIdentity {
    fn from(metadata: &Metadata) -> Self {
        Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            #[cfg(unix)]
            changed_seconds: metadata.ctime(),
            #[cfg(unix)]
            changed_nanoseconds: metadata.ctime_nsec(),
        }
    }
}

impl CatnapTask {
    pub(crate) fn start(original: &Path, content: String, history: u64) -> io::Result<Self> {
        let path = catnap_path(original);
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("catomic-catnap".to_string())
            .spawn(move || {
                let result = crate::file::io::atomic_write_private_string(&path, &content)
                    .map(|()| CatnapResult::Written { path, history })
                    .unwrap_or_else(|error| CatnapResult::Error(error.to_string()));
                let _ = sender.send(result);
            })?;
        Ok(Self { receiver })
    }

    pub(crate) fn try_result(&self) -> Option<CatnapResult> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(CatnapResult::Error(
                "catnap worker stopped without a result".to_string(),
            )),
        }
    }

    pub(crate) fn finish(self) -> CatnapResult {
        self.receiver.recv().unwrap_or_else(|_| {
            CatnapResult::Error("catnap worker stopped without a result".to_string())
        })
    }
}

pub(crate) fn catnap_path(original: &Path) -> PathBuf {
    let mut name = original
        .file_name()
        .map_or_else(|| OsString::from("untitled"), OsString::from);
    name.push(".catnap");
    original.with_file_name(name)
}

pub(crate) fn load_candidate(
    original: &Path,
    max_bytes: usize,
) -> io::Result<Option<RecoveryCandidate>> {
    let sidecar = catnap_path(original);
    let Some((mut file, metadata)) = open_regular_bounded(&sidecar, max_bytes)? else {
        return Ok(None);
    };
    if !candidate_is_new_enough(original, &metadata)? {
        return Ok(None);
    }
    let (text, identity) = read_opened(&mut file, metadata, max_bytes)?;
    if !entry_matches_identity(&sidecar, &identity)? {
        return Err(changed_during_read());
    }
    Ok(Some(RecoveryCandidate {
        file,
        text,
        identity,
        max_bytes,
    }))
}

fn candidate_is_new_enough(original: &Path, sidecar_meta: &Metadata) -> io::Result<bool> {
    let original_meta = match std::fs::metadata(original) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error),
    };
    Ok(match (sidecar_meta.modified(), original_meta.modified()) {
        (Ok(sidecar_time), Ok(original_time)) => sidecar_time >= original_time,
        _ => true,
    })
}

fn open_regular_bounded(path: &Path, max_bytes: usize) -> io::Result<Option<(File, Metadata)>> {
    let file = match open_without_following(path) {
        Ok(file) => file,
        Err(error)
            if error.kind() == io::ErrorKind::NotFound
                || error.raw_os_error() == Some(libc::ELOOP) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > max_bytes as u64 {
        return Ok(None);
    }
    Ok(Some((file, metadata)))
}

#[cfg(unix)]
fn open_without_following(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

#[cfg(not(unix))]
fn open_without_following(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).open(path)
}

fn read_opened(
    file: &mut File,
    initial_metadata: Metadata,
    max_bytes: usize,
) -> io::Result<(String, SidecarIdentity)> {
    let mut bytes = Vec::with_capacity(max_bytes.min(64 * 1024).saturating_add(1));
    file.by_ref()
        .take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Err(oversized());
    }
    let final_metadata = file.metadata()?;
    let initial_identity = SidecarIdentity::from(&initial_metadata);
    let final_identity = SidecarIdentity::from(&final_metadata);
    if !final_metadata.is_file()
        || final_metadata.len() > max_bytes as u64
        || initial_identity != final_identity
    {
        return Err(changed_during_read());
    }
    let text = String::from_utf8(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok((text, final_identity))
}

fn entry_matches_identity(path: &Path, identity: &SidecarIdentity) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.is_file() && SidecarIdentity::from(&metadata) == *identity),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn changed_during_read() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "catnap recovery changed while it was being read",
    )
}

fn oversized() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "catnap recovery exceeds the configured size limit",
    )
}

pub(crate) fn remove(original: &Path) -> io::Result<()> {
    match std::fs::remove_file(catnap_path(original)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
