//! Basic file read / write.
//!
//! Phase 0: straightforward std::fs.
//! Later: atomic write (write to .tmp then rename), encoding handling,
//! large file streaming, crash safety.

use std::io;
use std::path::Path;

/// Read entire file as UTF-8 string.
/// Phase 0: lossy or panic is acceptable per TODO.
pub fn read_to_string<P: AsRef<Path>>(path: P) -> io::Result<String> {
    std::fs::read_to_string(path)
}

/// Write string to file.
pub fn write_string<P: AsRef<Path>>(path: P, contents: &str) -> io::Result<()> {
    std::fs::write(path, contents)
}

/// TODO (Phase 2+): atomic_write, backup on conflict, etc.
pub fn _placeholder_atomic_write() {}
