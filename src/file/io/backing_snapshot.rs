//! Preserve original descriptor bytes before a hard-link save rewrites them.
//! Snapshots are owner-only, unlinked before copying, and retained only by open
//! descriptors. Copying is bounded in memory and occurs only on explicit save.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt};
use std::sync::atomic::{AtomicU64, Ordering};

/// Single-link originals survive atomic replacement. Multiply-linked originals
/// need a private copy; an already-detached snapshot has no links and is reused.
/// This runs inside the atomic writer, after its target identity is pinned. The
/// writer still validates the real destination independently before commit.
pub(crate) fn snapshot_hard_linked_file(source: &File) -> io::Result<Option<File>> {
    let before = source.metadata()?;
    if before.nlink() <= 1 {
        return Ok(None);
    }

    static NEXT_SNAPSHOT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "catomic-backing-{}-{}.tmp",
        std::process::id(),
        NEXT_SNAPSHOT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut snapshot = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    // No content is written while the snapshot has a directory entry. Closing
    // its last descriptor releases the disk space, including on every error.
    fs::remove_file(&path)?;

    let mut chunk = [0u8; 64 * 1024];
    let mut offset = 0;
    while offset < before.len() {
        let count = (before.len() - offset).min(chunk.len() as u64) as usize;
        source.read_exact_at(&mut chunk[..count], offset)?;
        snapshot.write_all(&chunk[..count])?;
        offset += count as u64;
    }
    let after = source.metadata()?;
    if before.len() != after.len()
        || before.modified()? != after.modified()?
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "original file changed while preserving backing storage",
        ));
    }
    Ok(Some(snapshot))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn hard_link_snapshot_is_private_byte_identical_and_reused() {
        let path =
            std::env::temp_dir().join(format!("catomic_snapshot_original_{}", std::process::id()));
        let alias = path.with_extension("alias");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&alias);
        let bytes = "λfirst\r\nsecond\r\n".repeat(10_000).into_bytes();
        fs::write(&path, &bytes).unwrap();
        let source = File::open(&path).unwrap();
        assert!(snapshot_hard_linked_file(&source).unwrap().is_none());
        fs::hard_link(&path, &alias).unwrap();

        let snapshot = snapshot_hard_linked_file(&source).unwrap().unwrap();
        let metadata = snapshot.metadata().unwrap();
        assert_eq!(metadata.nlink(), 0);
        assert_eq!(metadata.permissions().mode() & 0o077, 0);
        assert_ne!(metadata.ino(), source.metadata().unwrap().ino());
        fs::write(&alias, "rewritten").unwrap();
        let mut actual = vec![0; bytes.len()];
        snapshot.read_exact_at(&mut actual, 0).unwrap();
        assert_eq!(actual, bytes);
        assert!(snapshot_hard_linked_file(&snapshot).unwrap().is_none());

        fs::remove_file(path).unwrap();
        fs::remove_file(alias).unwrap();
    }
}
