//! Purpose: decode and encode supported UTF-8 text formats without changing document newlines.
//! Owns: UTF-8 BOM detection, line-ending policy, normalized reads, and streaming writes.
//! Must not: choose save paths, mutate buffers, perform atomic replacement, or know App/UI.
//! Invariants: in-memory text uses LF; writes restore the recorded BOM and newline sequence.
//! Phase: post-v0.1 core usability.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

use crate::buffer::Buffer;

const UTF8_BOM: &[u8; 3] = b"\xEF\xBB\xBF";
const FORMAT_SCAN_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineEnding {
    #[default]
    Lf,
    Crlf,
    Cr,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextFormat {
    pub utf8_bom: bool,
    pub line_ending: LineEnding,
}

impl TextFormat {
    pub fn label(self) -> &'static str {
        match (self.utf8_bom, self.line_ending) {
            (false, LineEnding::Lf) => "utf-8 lf",
            (false, LineEnding::Crlf) => "utf-8 crlf",
            (false, LineEnding::Cr) => "utf-8 cr",
            (true, LineEnding::Lf) => "utf-8-bom lf",
            (true, LineEnding::Crlf) => "utf-8-bom crlf",
            (true, LineEnding::Cr) => "utf-8-bom cr",
        }
    }
}

pub struct DecodedText {
    pub text: String,
    pub format: TextFormat,
}

pub fn read_text_file(path: impl AsRef<Path>) -> io::Result<DecodedText> {
    let bytes = std::fs::read(path)?;
    decode(bytes)
}

pub fn detect_file_format(path: impl AsRef<Path>) -> io::Result<TextFormat> {
    let mut file = File::open(path)?;
    let mut bytes = vec![0u8; FORMAT_SCAN_CHUNK_BYTES];
    let mut first_chunk = true;
    let mut utf8_bom = false;
    let mut pending_cr = false;
    loop {
        let read = file.read(&mut bytes)?;
        if read == 0 {
            return Ok(TextFormat {
                utf8_bom,
                line_ending: if pending_cr {
                    LineEnding::Cr
                } else {
                    LineEnding::default()
                },
            });
        }
        let chunk = &bytes[..read];
        if first_chunk {
            utf8_bom = chunk.starts_with(UTF8_BOM);
            first_chunk = false;
        }
        if pending_cr {
            return Ok(TextFormat {
                utf8_bom,
                line_ending: if chunk.first() == Some(&b'\n') {
                    LineEnding::Crlf
                } else {
                    LineEnding::Cr
                },
            });
        }
        if let Some(index) = chunk.iter().position(|byte| matches!(byte, b'\r' | b'\n')) {
            let line_ending = match chunk[index] {
                b'\n' => LineEnding::Lf,
                b'\r' if chunk.get(index + 1) == Some(&b'\n') => LineEnding::Crlf,
                b'\r' if index + 1 == chunk.len() => {
                    pending_cr = true;
                    continue;
                }
                _ => LineEnding::Cr,
            };
            return Ok(TextFormat {
                utf8_bom,
                line_ending,
            });
        }
    }
}

pub fn write_buffer(
    buffer: &dyn Buffer,
    out: &mut dyn Write,
    format: TextFormat,
) -> io::Result<()> {
    if format.utf8_bom {
        out.write_all(UTF8_BOM)?;
    }
    let mut writer = FormatWriter::new(out, format);
    buffer.write_to(&mut writer)?;
    writer.finish()
}

fn decode(bytes: Vec<u8>) -> io::Result<DecodedText> {
    let format = detect(&bytes);
    let mut text = String::from_utf8(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if format.utf8_bom {
        text.drain(..UTF8_BOM.len());
    }
    let text = if text.as_bytes().contains(&b'\r') {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text
    };
    Ok(DecodedText { text, format })
}

fn detect(bytes: &[u8]) -> TextFormat {
    let utf8_bom = bytes.starts_with(UTF8_BOM);
    let bytes = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    let line_ending = bytes
        .iter()
        .position(|byte| matches!(byte, b'\r' | b'\n'))
        .map(|index| match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => LineEnding::Crlf,
            b'\r' => LineEnding::Cr,
            _ => LineEnding::Lf,
        })
        .unwrap_or_default();
    TextFormat {
        utf8_bom,
        line_ending,
    }
}

struct FormatWriter<'a> {
    out: &'a mut dyn Write,
    format: TextFormat,
    pending_cr: bool,
    prefix: Vec<u8>,
    prefix_checked: bool,
}

impl<'a> FormatWriter<'a> {
    fn new(out: &'a mut dyn Write, format: TextFormat) -> Self {
        Self {
            out,
            format,
            pending_cr: false,
            prefix: Vec::with_capacity(UTF8_BOM.len()),
            prefix_checked: !format.utf8_bom,
        }
    }

    fn finish(mut self) -> io::Result<()> {
        self.finish_prefix()?;
        if self.pending_cr {
            self.write_newline()?;
        }
        self.out.flush()
    }

    fn consume(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut bytes = bytes;
        if self.pending_cr {
            self.write_newline()?;
            self.pending_cr = false;
            if bytes.first() == Some(&b'\n') {
                bytes = &bytes[1..];
            }
        }

        let mut plain_start = 0usize;
        let mut index = 0usize;
        while index < bytes.len() {
            if !matches!(bytes[index], b'\r' | b'\n') {
                index += 1;
                continue;
            }
            self.out.write_all(&bytes[plain_start..index])?;
            if bytes[index] == b'\r' && index + 1 == bytes.len() {
                self.pending_cr = true;
                return Ok(());
            }
            self.write_newline()?;
            index += if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
                2
            } else {
                1
            };
            plain_start = index;
        }
        self.out.write_all(&bytes[plain_start..])
    }

    fn finish_prefix(&mut self) -> io::Result<()> {
        if self.prefix_checked {
            return Ok(());
        }
        self.prefix_checked = true;
        let prefix = std::mem::take(&mut self.prefix);
        if prefix.as_slice() != UTF8_BOM {
            self.consume(&prefix)?;
        }
        Ok(())
    }

    fn write_newline(&mut self) -> io::Result<()> {
        self.out.write_all(match self.format.line_ending {
            LineEnding::Lf => b"\n",
            LineEnding::Crlf => b"\r\n",
            LineEnding::Cr => b"\r",
        })
    }
}

impl Write for FormatWriter<'_> {
    fn write(&mut self, mut bytes: &[u8]) -> io::Result<usize> {
        let original_len = bytes.len();
        if !self.prefix_checked {
            let needed = UTF8_BOM.len().saturating_sub(self.prefix.len());
            let take = needed.min(bytes.len());
            self.prefix.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.prefix.len() == UTF8_BOM.len() {
                self.finish_prefix()?;
            }
        }
        self.consume(bytes)?;
        Ok(original_len)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

#[cfg(test)]
#[path = "text_format_tests.rs"]
mod tests;
