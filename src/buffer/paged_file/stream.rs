//! Purpose: stream the complete logical paged document without materializing it.
//! Owns: ordered edited-page overlays and bounded untouched descriptor copies.
//! Must not: replace paths, mutate page/history state, render, or scan line metadata.
//! Invariants: each original byte range is emitted once; descriptor drift fails closed.
//! Phase: 2-by editable paged-file storage.

use std::io::{self, Write};
use std::os::unix::fs::FileExt;

use super::{EditablePage, PagedFileBuffer};
use crate::buffer::Buffer;

const COPY_CHUNK_BYTES: usize = 64 * 1024;

impl PagedFileBuffer {
    pub(super) fn stream_document(&self, out: &mut dyn Write) -> io::Result<()> {
        self.ensure_unchanged()?;
        let mut starts: Vec<usize> = self.retained.keys().copied().collect();
        starts.push(self.active().start_byte);
        starts.sort_unstable();
        starts.dedup();

        let mut original_offset = 0usize;
        for start in starts {
            let page = self.page_at(start).expect("known paged-file page");
            if page.buffer.edit_history_position() == 0 {
                continue;
            }
            self.write_original_range(original_offset, page.start_byte, out)?;
            page.buffer.write_to(out)?;
            original_offset = page.end_byte;
        }
        self.write_original_range(original_offset, self.total_bytes, out)?;
        self.ensure_unchanged()
    }

    fn page_at(&self, start: usize) -> Option<&EditablePage> {
        if self.active().start_byte == start {
            Some(self.active())
        } else {
            self.retained.get(&start)
        }
    }

    fn write_original_range(
        &self,
        mut start: usize,
        end: usize,
        out: &mut dyn Write,
    ) -> io::Result<()> {
        let mut chunk = vec![0u8; COPY_CHUNK_BYTES];
        while start < end {
            let len = (end - start).min(chunk.len());
            read_exact_at(&self.file, &mut chunk[..len], start)?;
            out.write_all(&chunk[..len])?;
            start += len;
        }
        Ok(())
    }
}

fn read_exact_at(file: &std::fs::File, mut out: &mut [u8], mut offset: usize) -> io::Result<()> {
    while !out.is_empty() {
        let read = file.read_at(out, offset as u64)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "short read while streaming paged file",
            ));
        }
        offset += read;
        out = &mut out[read..];
    }
    Ok(())
}
