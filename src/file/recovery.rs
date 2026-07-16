//! Purpose: own private `.catnap` sidecar paths, bounded reads, and async writes.
//! Owns: candidate detection, UTF-8 loading, atomic 0600 writes, cleanup, and task polling.
//! Must not: decide editor timing, mutate buffers, render UI, or recover automatically.
//! Invariants: sidecars append `.catnap`; reads are capped before allocation; writes are atomic.
//! Phase: 8 opt-in crash recovery.

use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

#[derive(Debug)]
pub(crate) enum CatnapResult {
    Written { path: PathBuf, history: u64 },
    Error(String),
}

pub(crate) struct CatnapTask {
    receiver: Receiver<CatnapResult>,
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

pub(crate) fn has_candidate(original: &Path, max_bytes: usize) -> io::Result<bool> {
    let sidecar = catnap_path(original);
    let sidecar_meta = match std::fs::symlink_metadata(&sidecar) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if sidecar_meta.file_type().is_symlink()
        || !sidecar_meta.is_file()
        || sidecar_meta.len() > max_bytes as u64
    {
        return Ok(false);
    }
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

pub(crate) fn read_bounded(original: &Path, max_bytes: usize) -> io::Result<String> {
    let path = catnap_path(original);
    let path_metadata = std::fs::symlink_metadata(&path)?;
    if path_metadata.file_type().is_symlink() || !path_metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "catnap recovery must be a regular non-symlink file",
        ));
    }
    let mut file = std::fs::File::open(path)?;
    if file.metadata()?.len() > max_bytes as u64 {
        return Err(oversized());
    }
    let mut bytes = Vec::with_capacity(max_bytes.min(64 * 1024).saturating_add(1));
    file.by_ref()
        .take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Err(oversized());
    }
    String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
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
mod tests {
    use super::*;

    fn path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("catomic_recovery_{}_{}", std::process::id(), name))
    }

    #[test]
    fn sidecar_appends_catnap_without_losing_the_original_extension() {
        assert_eq!(
            catnap_path(Path::new("notes.txt")),
            PathBuf::from("notes.txt.catnap")
        );
    }

    #[test]
    fn candidate_and_read_are_bounded() {
        let original = path("bounded.txt");
        let sidecar = catnap_path(&original);
        let _ = std::fs::remove_file(&original);
        let _ = std::fs::remove_file(&sidecar);
        std::fs::write(&sidecar, "recovered").unwrap();

        assert!(has_candidate(&original, 9).unwrap());
        assert_eq!(read_bounded(&original, 9).unwrap(), "recovered");
        assert!(!has_candidate(&original, 8).unwrap());
        assert!(read_bounded(&original, 8).is_err());

        remove(&original).unwrap();
    }

    #[test]
    fn async_write_records_exact_content_and_history() {
        let original = path("task.txt");
        let sidecar = catnap_path(&original);
        let _ = std::fs::remove_file(&sidecar);
        let result = CatnapTask::start(&original, "nap\n".to_string(), 7)
            .unwrap()
            .finish();

        assert!(matches!(result, CatnapResult::Written { history: 7, .. }));
        assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), "nap\n");
        remove(&original).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn recovery_refuses_symlink_sidecars() {
        use std::os::unix::fs::symlink;

        let original = path("symlink.txt");
        let sidecar = catnap_path(&original);
        let target = path("symlink-target.txt");
        let _ = std::fs::remove_file(&sidecar);
        let _ = std::fs::remove_file(&target);
        std::fs::write(&target, "not a catnap").unwrap();
        symlink(&target, &sidecar).unwrap();

        assert!(!has_candidate(&original, 1024).unwrap());
        assert!(read_bounded(&original, 1024).is_err());

        let _ = std::fs::remove_file(sidecar);
        let _ = std::fs::remove_file(target);
    }
}
