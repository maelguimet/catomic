//! Purpose: capture and compare bounded on-disk identities for external-edit safety.
//! Owns: FileSnapshot, content identities, observations, and pure comparison.
//! Must not: own save/reload policy, watchers, buffers, UI, Project, or LLM state.
//! Invariants: ordinary editable files receive a full SHA-256; paged files hash
//!   fixed start/middle/end samples; capture drift fails closed; snapshot capture
//!   uses fixed memory; pinned full reads are limited to the full-read tier.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use ring::digest::{Context, SHA256};

use crate::file::size::LARGE_FILE_LIMIT_BYTES;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileSnapshot {
    Present {
        len: u64,
        mtime: Option<std::time::SystemTime>,
        change_id: Option<FileChangeId>,
        content_identity: Option<FileContentIdentity>,
    },
    Absent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileContentIdentity {
    FullSha256([u8; 32]),
    SampledSha256([u8; 32]),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChangeId {
    device: u64,
    inode: u64,
    ctime_seconds: i64,
    ctime_nanoseconds: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternalFileStatus {
    NoPath,
    Unchanged,
    Modified,
    Deleted,
    Unknown(io::ErrorKind),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalFileObservation {
    pub status: ExternalFileStatus,
    pub live_snapshot: Option<FileSnapshot>,
}

/// One regular-file descriptor pinned to the revision captured in `snapshot`.
/// Open/reload code must derive bytes and format from this descriptor, then
/// verify both the descriptor and pathname before accepting the result.
#[derive(Debug)]
pub(crate) struct PinnedFile {
    file: File,
    snapshot: FileSnapshot,
}

impl PinnedFile {
    pub(crate) fn open(path: impl AsRef<Path>) -> io::Result<Option<Self>> {
        let path = path.as_ref();
        let Some(mut file) = open_nonblocking(path)? else {
            return Ok(None);
        };
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("refusing to open non-regular file: {}", path.display()),
            ));
        }
        let snapshot = snapshot_from_open_file(&mut file, path)?;
        Ok(Some(Self { file, snapshot }))
    }

    pub(crate) fn snapshot(&self) -> &FileSnapshot {
        &self.snapshot
    }

    pub(crate) fn file_mut(&mut self) -> &mut File {
        &mut self.file
    }

    pub(crate) fn read_all_verified(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        let len = match &self.snapshot {
            FileSnapshot::Present { len, .. } => *len,
            FileSnapshot::Absent => unreachable!("PinnedFile always captures a present file"),
        };
        if len > LARGE_FILE_LIMIT_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "paged file cannot use the full-read descriptor path",
            ));
        }
        self.file.seek(SeekFrom::Start(0))?;
        let capacity = usize::try_from(len).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "file size exceeds this platform's addressable range",
            )
        })?;
        let mut bytes = Vec::with_capacity(capacity);
        self.file.read_to_end(&mut bytes)?;

        let mut loaded = snapshot_from_metadata(&self.file.metadata()?);
        if let FileSnapshot::Present {
            content_identity: slot,
            ..
        } = &mut loaded
        {
            *slot = Some(FileContentIdentity::FullSha256(digest_bytes(&bytes)));
        }
        self.file.seek(SeekFrom::Start(0))?;
        if loaded == self.snapshot {
            Ok(bytes)
        } else {
            Err(snapshot_drift_error(path))
        }
    }

    pub(crate) fn into_file(self) -> File {
        self.file
    }

    pub(crate) fn ensure_descriptor_unchanged(&mut self, path: &Path) -> io::Result<()> {
        let current = snapshot_from_open_file(&mut self.file, path)?;
        if current == self.snapshot {
            Ok(())
        } else {
            Err(snapshot_drift_error(path))
        }
    }

    pub(crate) fn ensure_path_unchanged(&self, path: &Path) -> io::Result<()> {
        ensure_path_matches_snapshot(path, &self.snapshot)
    }
}

/// Verify a pathname still names the exact captured content identity. This is
/// validation only: callers keep `expected` as the accepted clean baseline.
pub(crate) fn ensure_path_matches_snapshot(
    path: impl AsRef<Path>,
    expected: &FileSnapshot,
) -> io::Result<()> {
    let path = path.as_ref();
    if capture_file_snapshot(path)? == *expected {
        Ok(())
    } else {
        Err(snapshot_drift_error(path))
    }
}

/// Capture current on-disk state. Files through 100 MiB are fully hashed with
/// fixed memory; larger paged files hash three fixed 64 KiB samples.
pub fn capture_file_snapshot(path: impl AsRef<Path>) -> io::Result<FileSnapshot> {
    let path = path.as_ref();
    match fs::metadata(path) {
        Ok(meta) => snapshot_from_path_and_metadata(path, &meta),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(FileSnapshot::Absent),
        Err(error) => Err(error),
    }
}

fn open_nonblocking(path: &Path) -> io::Result<Option<File>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NONBLOCK);
    match options.open(path) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn snapshot_from_metadata(meta: &fs::Metadata) -> FileSnapshot {
    FileSnapshot::Present {
        len: meta.len(),
        mtime: meta.modified().ok(),
        change_id: file_change_id(meta),
        content_identity: None,
    }
}

fn snapshot_from_path_and_metadata(path: &Path, meta: &fs::Metadata) -> io::Result<FileSnapshot> {
    let mut snapshot = snapshot_from_metadata(meta);
    if !meta.is_file() {
        return Ok(snapshot);
    }

    let mut file = File::open(path)?;
    if snapshot_from_metadata(&file.metadata()?) != snapshot {
        return Err(snapshot_drift_error(path));
    }
    let content_identity = capture_content_identity(&mut file, meta.len())?;
    if snapshot_from_metadata(&file.metadata()?) != snapshot
        || snapshot_from_metadata(&fs::metadata(path)?) != snapshot
    {
        return Err(snapshot_drift_error(path));
    }
    if let FileSnapshot::Present {
        content_identity: slot,
        ..
    } = &mut snapshot
    {
        *slot = Some(content_identity);
    }
    Ok(snapshot)
}

fn snapshot_from_open_file(file: &mut File, path: &Path) -> io::Result<FileSnapshot> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("refusing to open non-regular file: {}", path.display()),
        ));
    }
    let mut snapshot = snapshot_from_metadata(&metadata);
    let content_identity = capture_content_identity(file, metadata.len())?;
    if snapshot_from_metadata(&file.metadata()?) != snapshot {
        return Err(snapshot_drift_error(path));
    }
    if let FileSnapshot::Present {
        content_identity: slot,
        ..
    } = &mut snapshot
    {
        *slot = Some(content_identity);
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(snapshot)
}

fn capture_content_identity(file: &mut File, len: u64) -> io::Result<FileContentIdentity> {
    const CHUNK_BYTES: usize = 64 * 1024;

    if len <= LARGE_FILE_LIMIT_BYTES {
        file.seek(SeekFrom::Start(0))?;
        let mut context = Context::new(&SHA256);
        let mut chunk = [0u8; CHUNK_BYTES];
        loop {
            let read = file.read(&mut chunk)?;
            if read == 0 {
                break;
            }
            context.update(&chunk[..read]);
        }
        return Ok(FileContentIdentity::FullSha256(finish_digest(context)));
    }

    let sample_len = CHUNK_BYTES as u64;
    let starts = [
        0,
        (len / 2).saturating_sub(sample_len / 2),
        len.saturating_sub(sample_len),
    ];
    let mut context = Context::new(&SHA256);
    context.update(b"catomic-sampled-file-v1");
    context.update(&len.to_le_bytes());
    let mut chunk = [0u8; CHUNK_BYTES];
    for (index, start) in starts.into_iter().enumerate() {
        if starts[..index].contains(&start) {
            continue;
        }
        context.update(&start.to_le_bytes());
        file.seek(SeekFrom::Start(start))?;
        let wanted = (len - start).min(sample_len) as usize;
        file.read_exact(&mut chunk[..wanted])?;
        context.update(&chunk[..wanted]);
    }
    Ok(FileContentIdentity::SampledSha256(finish_digest(context)))
}

fn finish_digest(context: Context) -> [u8; 32] {
    let digest = context.finish();
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(digest.as_ref());
    bytes
}

fn digest_bytes(bytes: &[u8]) -> [u8; 32] {
    let digest = ring::digest::digest(&SHA256, bytes);
    let mut output = [0u8; 32];
    output.copy_from_slice(digest.as_ref());
    output
}

fn snapshot_drift_error(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::Interrupted,
        format!("file changed while capturing snapshot: {}", path.display()),
    )
}

#[cfg(test)]
pub(crate) fn compare_to_snapshot(
    path: impl AsRef<Path>,
    baseline: &FileSnapshot,
) -> io::Result<ExternalFileStatus> {
    let current = match capture_file_snapshot(path.as_ref()) {
        Ok(snapshot) => snapshot,
        Err(error) => return Ok(ExternalFileStatus::Unknown(error.kind())),
    };
    Ok(compare_live_snapshot_to_baseline(&current, baseline))
}

pub(crate) fn compare_live_snapshot_to_baseline(
    live: &FileSnapshot,
    baseline: &FileSnapshot,
) -> ExternalFileStatus {
    match (baseline, live) {
        (FileSnapshot::Absent, FileSnapshot::Absent) => ExternalFileStatus::Unchanged,
        (FileSnapshot::Absent, FileSnapshot::Present { .. }) => ExternalFileStatus::Modified,
        (FileSnapshot::Present { .. }, FileSnapshot::Absent) => ExternalFileStatus::Deleted,
        (FileSnapshot::Present { .. }, FileSnapshot::Present { .. }) if live == baseline => {
            ExternalFileStatus::Unchanged
        }
        (FileSnapshot::Present { .. }, FileSnapshot::Present { .. }) => {
            ExternalFileStatus::Modified
        }
    }
}

pub fn observe_external_file(
    path: Option<&Path>,
    baseline: Option<&FileSnapshot>,
) -> ExternalFileObservation {
    let Some(path) = path else {
        return ExternalFileObservation {
            status: ExternalFileStatus::NoPath,
            live_snapshot: None,
        };
    };
    let live_result = capture_file_snapshot(path);
    let live_snapshot = live_result.as_ref().ok().cloned();
    let status = match (&live_result, baseline) {
        (Ok(FileSnapshot::Present { .. }), None) => ExternalFileStatus::Unchanged,
        (Ok(FileSnapshot::Absent), None) => ExternalFileStatus::Deleted,
        (Err(error), None) => ExternalFileStatus::Unknown(error.kind()),
        (Ok(live), Some(base)) => compare_live_snapshot_to_baseline(live, base),
        (Err(error), Some(_)) => ExternalFileStatus::Unknown(error.kind()),
    };
    ExternalFileObservation {
        status,
        live_snapshot,
    }
}

#[cfg(unix)]
fn file_change_id(meta: &fs::Metadata) -> Option<FileChangeId> {
    use std::os::unix::fs::MetadataExt;

    Some(FileChangeId {
        device: meta.dev(),
        inode: meta.ino(),
        ctime_seconds: meta.ctime(),
        ctime_nanoseconds: meta.ctime_nsec(),
    })
}

#[cfg(not(unix))]
fn file_change_id(_meta: &fs::Metadata) -> Option<FileChangeId> {
    None
}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;
