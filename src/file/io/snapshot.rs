//! Purpose: capture and compare bounded on-disk identities for external-edit safety.
//! Owns: FileSnapshot, content identities, observations, and pure comparison.
//! Must not: own save/reload policy, watchers, buffers, UI, Project, or LLM state.
//! Invariants: ordinary editable files receive a full SHA-256; paged files hash
//!   fixed start/middle/end samples; capture drift fails closed; memory is fixed.
//! Phase: post-v0.1 external-change metadata-collision hardening.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

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

/// Initial-open capture that also refuses non-regular targets.
pub(crate) fn capture_regular_file_snapshot(path: impl AsRef<Path>) -> io::Result<FileSnapshot> {
    let path = path.as_ref();
    match fs::metadata(path) {
        Ok(meta) if meta.is_file() => snapshot_from_path_and_metadata(path, &meta),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("refusing to open non-regular file: {}", path.display()),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(FileSnapshot::Absent),
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
