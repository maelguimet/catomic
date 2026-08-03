//! Purpose: decode and encode supported UTF-8 text formats without changing document newlines.
//! Owns: UTF-8 BOM detection, line-ending policy, normalized reads, and streaming writes.
//! Must not: choose save paths, mutate buffers, perform atomic replacement, or know App/UI.
//! Invariants: in-memory text uses LF; writes restore the recorded BOM and newline sequence.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
#[cfg(test)]
use std::path::Path;

use memchr::{memchr, memchr2};

use crate::buffer::Buffer;

const UTF8_BOM: &[u8; 3] = b"\xEF\xBB\xBF";
const FORMAT_SCAN_CHUNK_BYTES: usize = 64 * 1024;
const FORMAT_WRITE_BUFFER_BYTES: usize = 64 * 1024;

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

pub struct DecodedText {
    pub text: String,
    pub format: TextFormat,
}

#[cfg(test)]
pub fn detect_file_format(path: impl AsRef<Path>) -> io::Result<TextFormat> {
    let mut file = File::open(path)?;
    detect_file_format_from(&mut file)
}

pub(crate) fn detect_file_format_from(file: &mut File) -> io::Result<TextFormat> {
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = vec![0u8; FORMAT_SCAN_CHUNK_BYTES];
    let mut detection = FormatDetection::default();
    loop {
        let read = file.read(&mut bytes)?;
        if read == 0 {
            return Ok(detection.finish());
        }
        if let Some(format) = detection.push(&bytes[..read]) {
            return Ok(format);
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

pub(crate) fn decode(bytes: Vec<u8>) -> io::Result<DecodedText> {
    let utf8_bom = bytes.starts_with(UTF8_BOM);
    let content_start = usize::from(utf8_bom) * UTF8_BOM.len();
    let content = &bytes[content_start..];
    let first_ending = first_line_ending(content);
    let line_ending = first_ending.line_ending();
    let first_cr = match first_ending {
        FirstLineEnding::Found {
            index, byte: b'\r', ..
        }
        | FirstLineEnding::TrailingCr { index } => Some(index),
        FirstLineEnding::Found { index, .. } => {
            memchr(b'\r', &content[index + 1..]).map(|offset| index + 1 + offset)
        }
        FirstLineEnding::None => None,
    };
    let format = TextFormat {
        utf8_bom,
        line_ending,
    };

    let Some(first_cr) = first_cr else {
        let mut text = String::from_utf8(bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if utf8_bom {
            text.drain(..UTF8_BOM.len());
        }
        return Ok(DecodedText { text, format });
    };

    let mut normalized = Vec::with_capacity(content.len());
    let mut plain_start = 0usize;
    let mut cr_index = first_cr;
    loop {
        normalized.extend_from_slice(&content[plain_start..cr_index]);
        normalized.push(b'\n');
        plain_start = cr_index + 1;
        if content.get(plain_start) == Some(&b'\n') {
            plain_start += 1;
        }
        let Some(offset) = memchr(b'\r', &content[plain_start..]) else {
            break;
        };
        cr_index = plain_start + offset;
    }
    normalized.extend_from_slice(&content[plain_start..]);
    let text = String::from_utf8(normalized)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(DecodedText { text, format })
}

#[cfg(test)]
fn detect(bytes: &[u8]) -> TextFormat {
    let mut detection = FormatDetection::default();
    detection.push(bytes).unwrap_or_else(|| detection.finish())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FirstLineEnding {
    None,
    TrailingCr {
        index: usize,
    },
    Found {
        index: usize,
        byte: u8,
        line_ending: LineEnding,
    },
}

impl FirstLineEnding {
    fn line_ending(self) -> LineEnding {
        match self {
            Self::Found { line_ending, .. } => line_ending,
            Self::TrailingCr { .. } => LineEnding::Cr,
            Self::None => LineEnding::default(),
        }
    }
}

fn first_line_ending(bytes: &[u8]) -> FirstLineEnding {
    let Some(index) = memchr2(b'\r', b'\n', bytes) else {
        return FirstLineEnding::None;
    };
    match bytes[index] {
        b'\n' => FirstLineEnding::Found {
            index,
            byte: b'\n',
            line_ending: LineEnding::Lf,
        },
        b'\r' if bytes.get(index + 1) == Some(&b'\n') => FirstLineEnding::Found {
            index,
            byte: b'\r',
            line_ending: LineEnding::Crlf,
        },
        b'\r' if index + 1 == bytes.len() => FirstLineEnding::TrailingCr { index },
        b'\r' => FirstLineEnding::Found {
            index,
            byte: b'\r',
            line_ending: LineEnding::Cr,
        },
        _ => unreachable!("memchr2 returns only CR or LF offsets"),
    }
}

#[derive(Default)]
struct FormatDetection {
    utf8_bom: Option<bool>,
    bom_match_len: usize,
    line_ending: Option<LineEnding>,
    pending_cr: bool,
}

impl FormatDetection {
    fn push(&mut self, bytes: &[u8]) -> Option<TextFormat> {
        self.detect_bom(bytes);
        if self.line_ending.is_none() && !bytes.is_empty() {
            if self.pending_cr {
                self.line_ending = Some(if bytes[0] == b'\n' {
                    LineEnding::Crlf
                } else {
                    LineEnding::Cr
                });
                self.pending_cr = false;
            } else {
                match first_line_ending(bytes) {
                    FirstLineEnding::None => {}
                    FirstLineEnding::TrailingCr { .. } => self.pending_cr = true,
                    FirstLineEnding::Found { line_ending, .. } => {
                        self.line_ending = Some(line_ending)
                    }
                }
            }
        }
        self.completed_format()
    }

    fn finish(self) -> TextFormat {
        TextFormat {
            utf8_bom: self.utf8_bom.unwrap_or(false),
            line_ending: self.line_ending.unwrap_or(if self.pending_cr {
                LineEnding::Cr
            } else {
                LineEnding::default()
            }),
        }
    }

    fn detect_bom(&mut self, bytes: &[u8]) {
        if self.utf8_bom.is_some() {
            return;
        }
        for &byte in bytes.iter().take(UTF8_BOM.len() - self.bom_match_len) {
            if byte != UTF8_BOM[self.bom_match_len] {
                self.utf8_bom = Some(false);
                return;
            }
            self.bom_match_len += 1;
            if self.bom_match_len == UTF8_BOM.len() {
                self.utf8_bom = Some(true);
                return;
            }
        }
    }

    fn completed_format(&self) -> Option<TextFormat> {
        Some(TextFormat {
            utf8_bom: self.utf8_bom?,
            line_ending: self.line_ending?,
        })
    }
}

#[cfg(test)]
pub(crate) fn detect_bytes_for_perf(bytes: &[u8]) -> TextFormat {
    detect(bytes)
}

#[cfg(test)]
pub(crate) fn write_chunks_for_perf(
    chunks: &[&[u8]],
    out: &mut dyn Write,
    format: TextFormat,
) -> io::Result<()> {
    if format.utf8_bom {
        out.write_all(UTF8_BOM)?;
    }
    let mut writer = FormatWriter::new(out, format);
    for chunk in chunks {
        writer.write_all(chunk)?;
    }
    writer.finish()
}

struct FormatWriter<'a> {
    out: &'a mut dyn Write,
    format: TextFormat,
    pending_cr: bool,
    prefix: [u8; UTF8_BOM.len()],
    prefix_len: usize,
    prefix_checked: bool,
    converted: Vec<u8>,
}

impl<'a> FormatWriter<'a> {
    fn new(out: &'a mut dyn Write, format: TextFormat) -> Self {
        Self {
            out,
            format,
            pending_cr: false,
            prefix: [0; UTF8_BOM.len()],
            prefix_len: 0,
            prefix_checked: !format.utf8_bom,
            converted: Vec::new(),
        }
    }

    fn finish(mut self) -> io::Result<()> {
        self.finish_prefix()?;
        if self.pending_cr {
            self.write_newline()?;
        }
        self.flush_converted()?;
        self.out.flush()
    }

    fn consume(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let mut bytes = bytes;
        if self.pending_cr {
            self.write_newline()?;
            self.pending_cr = false;
            if bytes.first() == Some(&b'\n') {
                bytes = &bytes[1..];
            }
        }

        if self.format.line_ending == LineEnding::Lf && memchr(b'\r', bytes).is_none() {
            self.flush_converted()?;
            return self.out.write_all(bytes);
        }

        let mut plain_start = 0usize;
        let mut search_start = 0usize;
        while let Some(offset) = memchr2(b'\r', b'\n', &bytes[search_start..]) {
            let mut index = search_start + offset;
            self.write_converted(&bytes[plain_start..index])?;
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
            search_start = index;
        }
        self.write_converted(&bytes[plain_start..])
    }

    fn finish_prefix(&mut self) -> io::Result<()> {
        if self.prefix_checked {
            return Ok(());
        }
        self.prefix_checked = true;
        if self.prefix[..self.prefix_len] != UTF8_BOM[..self.prefix_len]
            || self.prefix_len != UTF8_BOM.len()
        {
            let prefix = self.prefix;
            self.consume(&prefix[..self.prefix_len])?;
        }
        Ok(())
    }

    fn write_newline(&mut self) -> io::Result<()> {
        self.write_converted(match self.format.line_ending {
            LineEnding::Lf => b"\n",
            LineEnding::Crlf => b"\r\n",
            LineEnding::Cr => b"\r",
        })
    }

    fn write_converted(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            if self.converted.is_empty() && bytes.len() >= FORMAT_WRITE_BUFFER_BYTES {
                self.out.write_all(bytes)?;
                return Ok(());
            }
            if self.converted.capacity() == 0 {
                self.converted.reserve_exact(FORMAT_WRITE_BUFFER_BYTES);
            }
            let available = FORMAT_WRITE_BUFFER_BYTES - self.converted.len();
            let take = available.min(bytes.len());
            self.converted.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.converted.len() == FORMAT_WRITE_BUFFER_BYTES {
                self.flush_converted()?;
            }
        }
        Ok(())
    }

    fn flush_converted(&mut self) -> io::Result<()> {
        let mut written = 0usize;
        while written < self.converted.len() {
            match self.out.write(&self.converted[written..]) {
                Ok(0) => {
                    if written > 0 {
                        self.converted.drain(..written);
                    }
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "failed to write converted text format bytes",
                    ));
                }
                Ok(count) => written += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    if written > 0 {
                        self.converted.drain(..written);
                    }
                    return Err(error);
                }
            }
        }
        self.converted.clear();
        Ok(())
    }
}

impl Write for FormatWriter<'_> {
    fn write(&mut self, mut bytes: &[u8]) -> io::Result<usize> {
        let original_len = bytes.len();
        if !self.prefix_checked {
            let needed = UTF8_BOM.len().saturating_sub(self.prefix_len);
            let take = needed.min(bytes.len());
            self.prefix[self.prefix_len..self.prefix_len + take].copy_from_slice(&bytes[..take]);
            self.prefix_len += take;
            bytes = &bytes[take..];
            if self.prefix_len == UTF8_BOM.len() {
                self.finish_prefix()?;
            }
        }
        self.consume(bytes)?;
        Ok(original_len)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flush_converted()?;
        self.out.flush()
    }
}

#[cfg(test)]
#[path = "text_format_tests.rs"]
mod tests;
