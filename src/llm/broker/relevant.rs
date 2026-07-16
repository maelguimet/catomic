//! Purpose: this file must bind repo requests to exact bytes of every relevant file.
//! Owns: active/retrieved file pinning, bounded fingerprints, and symlink-safe paths.
//! Must not: expose file bytes to models, consume context budget, write, or network.
//! Invariants: pinned paths stay normalized/in-repo and every fingerprint is size-bounded.
//! Phase: 6 acceptance hardening.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;

use super::{
    ensure_normalized_relative, io_error, BrokerError, ContextBroker, MAX_RELEVANT_FILE_BYTES,
};

impl ContextBroker {
    pub fn pin_relevant_file(&mut self, path: &Path) -> Result<(), BrokerError> {
        ensure_normalized_relative(path)?;
        let joined = self.git.root.join(path);
        let canonical_root = self.git.root.canonicalize().map_err(io_error)?;
        let canonical = joined.canonicalize().map_err(io_error)?;
        if canonical.strip_prefix(&canonical_root).ok() != Some(path) {
            return Err(BrokerError::InvalidPath);
        }
        let (_, fingerprint) = self.open_relevant_file(path)?;
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

    pub(super) fn open_relevant_file(
        &self,
        relative: &Path,
    ) -> Result<(fs::File, u64), BrokerError> {
        let path = self.git.root.join(relative);
        let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(BrokerError::InvalidPath);
        }
        if metadata.len() > MAX_RELEVANT_FILE_BYTES {
            return Err(BrokerError::FileTooLarge {
                path: relative.to_path_buf(),
                bytes: metadata.len(),
            });
        }
        let fingerprint = fingerprint(&path)?;
        let file = fs::File::open(path).map_err(io_error)?;
        Ok((file, fingerprint))
    }
}

pub(super) fn fingerprint(path: &Path) -> Result<u64, BrokerError> {
    let bytes = fs::read(path).map_err(io_error)?;
    if bytes.len() as u64 > MAX_RELEVANT_FILE_BYTES {
        return Err(BrokerError::FileTooLarge {
            path: path.to_path_buf(),
            bytes: bytes.len() as u64,
        });
    }
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    Ok(hasher.finish())
}
