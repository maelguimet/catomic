//! Read explicitly requested standard input into a bounded UTF-8 document.
//! No temporary files, terminal access, or implicit reads belong here.

use std::io::{self, Read};
use std::os::fd::{AsFd, AsRawFd};

use super::size::{classify_file_size, open_size_warning_message, LARGE_FILE_LIMIT_BYTES};
use super::text_format::{self, DecodedText};

pub(crate) struct ImportedText {
    pub(crate) decoded: DecodedText,
    pub(crate) warning: Option<String>,
}

pub(crate) fn read_stdin(cancelled: impl FnMut() -> bool) -> io::Result<ImportedText> {
    // A direct descriptor read honors the byte cap even at the final one-byte
    // overflow probe. StdinLock may prefetch extra pipe bytes into its buffer.
    // Duplication preserves stdin itself for Crossterm's controlling-TTY choice.
    let descriptor = io::stdin()
        .as_fd()
        .try_clone_to_owned()
        .map_err(|error| io::Error::new(error.kind(), format!("open standard input: {error}")))?;
    read(StdinReader(std::fs::File::from(descriptor)), cancelled)
}

struct StdinReader(std::fs::File);

impl Read for StdinReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let mut descriptor = libc::pollfd {
            fd: self.0.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: the descriptor remains owned and valid throughout this call,
        // and poll writes only this initialized pollfd. A short wait closes the
        // signal/check race when a producer holds its pipe open without output.
        match unsafe { libc::poll(&mut descriptor, 1, 100) } {
            -1 => Err(io::Error::last_os_error()),
            0 => Err(io::ErrorKind::Interrupted.into()),
            _ => self.0.read(bytes),
        }
    }
}

pub(crate) fn read(reader: impl Read, cancelled: impl FnMut() -> bool) -> io::Result<ImportedText> {
    read_bounded(reader, LARGE_FILE_LIMIT_BYTES as usize, cancelled)
}

fn read_bounded(
    mut reader: impl Read,
    limit: usize,
    mut cancelled: impl FnMut() -> bool,
) -> io::Result<ImportedText> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        if cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "standard input read cancelled",
            ));
        }
        let remaining = (limit + 1 - bytes.len()).min(chunk.len());
        let count = match reader.read(&mut chunk[..remaining]) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(io::Error::new(
                    error.kind(),
                    format!("read standard input: {error}"),
                ));
            }
        };
        if bytes.len() + count > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "standard input exceeds 100 MiB; write it to a file and open that path for paged editing",
            ));
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    if cancelled() {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "standard input read cancelled",
        ));
    }
    let length = bytes.len() as u64;
    let decoded = text_format::decode(bytes).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("standard input is not valid UTF-8: {error}"),
        )
    })?;
    Ok(ImportedText {
        decoded,
        warning: open_size_warning_message(length, classify_file_size(length)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn bounded_input_accepts_the_limit_and_reads_only_one_overflow_byte() {
        let exact = read_bounded(Cursor::new(b"four"), 4, || false).unwrap();
        assert_eq!(exact.decoded.text, "four");
        let mut oversized = Cursor::new(b"four plus unread bytes");
        let error = read_bounded(&mut oversized, 4, || false).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("exceeds 100 MiB"));
        assert_eq!(oversized.position(), 5);
    }

    #[test]
    fn input_preserves_supported_format_and_rejects_invalid_utf8() {
        let input = read(Cursor::new(b"\xef\xbb\xbfhello\r\nworld\r\n"), || false).unwrap();
        assert_eq!(input.decoded.text, "hello\nworld\n");
        assert!(input.decoded.format.utf8_bom);
        assert_eq!(
            input.decoded.format.line_ending,
            text_format::LineEnding::Crlf
        );
        assert!(input.warning.is_none());
        assert_eq!(read(io::empty(), || false).unwrap().decoded.text, "");
        let error = read(Cursor::new(b"valid\n\xff"), || false).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error
            .to_string()
            .contains("standard input is not valid UTF-8"));
    }

    #[test]
    fn input_read_errors_and_cancellation_fail_without_a_partial_document() {
        struct FailsAfterText(bool);
        impl Read for FailsAfterText {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                if self.0 {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "fixture read failure",
                    ));
                }
                self.0 = true;
                bytes[0] = b'x';
                Ok(1)
            }
        }
        let error = read(FailsAfterText(false), || false).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert!(error
            .to_string()
            .contains("read standard input: fixture read failure"));
        let error = read(io::repeat(b'x'), || true).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    }
}
