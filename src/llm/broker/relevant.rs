//! Purpose: this file must bind repo requests to exact bytes of every relevant file.
//! Owns: active/retrieved file pinning, bounded fingerprints, and symlink-safe paths.
//! Must not: expose file bytes to models, consume context budget, write, or network.
//! Invariants: pinned paths stay normalized/in-repo and every fingerprint is size-bounded.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::Path;

use super::{
    ensure_normalized_relative, io_error, BrokerError, ContextBroker, MAX_RELEVANT_FILE_BYTES,
};

impl ContextBroker {
    pub fn pin_relevant_file(&mut self, path: &Path) -> Result<(), BrokerError> {
        ensure_normalized_relative(path)?;
        let (_, fingerprint) = self.snapshot_relevant_file(path)?;
        self.record_relevant_file(path, fingerprint)
    }

    pub(super) fn record_relevant_file(
        &mut self,
        path: &Path,
        fingerprint: u64,
    ) -> Result<(), BrokerError> {
        match self.relevant_files.get(path) {
            Some(expected) if *expected != fingerprint => {
                Err(BrokerError::FileChanged(path.to_path_buf()))
            }
            Some(_) => Ok(()),
            None => {
                self.relevant_files.insert(path.to_path_buf(), fingerprint);
                Ok(())
            }
        }
    }

    pub(super) fn snapshot_relevant_file(
        &self,
        relative: &Path,
    ) -> Result<(Vec<u8>, u64), BrokerError> {
        ensure_normalized_relative(relative)?;
        let path = self.canonical_root.join(relative);
        if path.canonicalize().map_err(io_error)? != path {
            return Err(BrokerError::InvalidPath);
        }
        let before_path = checked_metadata(&path, relative)?;
        let mut file = fs::File::open(&path).map_err(io_error)?;
        let before_open = file.metadata().map_err(io_error)?;
        if !same_revision(&before_path, &before_open) {
            return Err(BrokerError::FileChanged(relative.to_path_buf()));
        }
        let mut bytes = Vec::with_capacity(before_open.len() as usize);
        (&mut file)
            .take(MAX_RELEVANT_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(io_error)?;
        if bytes.len() as u64 > MAX_RELEVANT_FILE_BYTES {
            return Err(too_large(relative, bytes.len() as u64));
        }
        let after_open = file.metadata().map_err(io_error)?;
        let after_path = checked_metadata(&path, relative)?;
        if !same_revision(&before_open, &after_open) || !same_revision(&after_open, &after_path) {
            return Err(BrokerError::FileChanged(relative.to_path_buf()));
        }
        if path.canonicalize().map_err(io_error)? != path {
            return Err(BrokerError::InvalidPath);
        }
        let fingerprint = fingerprint_bytes(&bytes);
        Ok((bytes, fingerprint))
    }
}

fn checked_metadata(path: &Path, relative: &Path) -> Result<fs::Metadata, BrokerError> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(BrokerError::InvalidPath);
    }
    if metadata.len() > MAX_RELEVANT_FILE_BYTES {
        return Err(too_large(relative, metadata.len()));
    }
    Ok(metadata)
}

fn too_large(path: &Path, bytes: u64) -> BrokerError {
    BrokerError::FileTooLarge {
        path: path.to_path_buf(),
        bytes,
    }
}

fn fingerprint_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

#[cfg(unix)]
fn same_revision(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

#[cfg(not(unix))]
fn same_revision(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.len() == right.len() && left.modified().ok() == right.modified().ok()
}
