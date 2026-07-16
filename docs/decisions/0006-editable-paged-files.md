# 0006 - Editable Paged Files

Date: 2026-07-16

Status: accepted

## Decision

Every regular UTF-8 file supported by Catomic remains editable regardless of
byte size. Small/Large files keep the full-read PieceTable path. Huge/Extreme
files use `PagedFileBuffer`, which composes editable, file-backed PieceTables
over configurable logical-line pages.

- `[big_files] page_lines` defaults to 20,000 and controls source-page size.
- The active page and pages with edit history are retained; untouched inactive
  pages are reloaded from the stable descriptor when visited.
- Page byte ranges are stable session anchors. Inserts and newline changes do
  not reflow every later page while typing; reload/reopen uses a fresh descriptor
  and rebalances pages from the resulting file.
- Undo/redo uses one global transaction order and activates the affected page.
- Ctrl+S streams untouched descriptor ranges and edited page content into the
  existing atomic-save path without building a whole-file String.
- Ctrl+F searches the stable descriptor plus unsaved edited-page overlays. It
  preserves matches across read chunks and edited page boundaries.
- Same-descriptor metadata drift fails page loads, rendering, search, and save
  closed. Clean path changes auto-reload by default; dirty buffers never reload
  automatically and retain the Ctrl+R/save-conflict confirmation paths.
- Byte size alone never selects a read-only or refusal mode.

No new runtime dependency, background index, network service, mmap, or unsafe
code is introduced.

## Rationale

Stable source anchors keep typing and page navigation local: an early newline
edit does not require rescanning or renumbering the rest of a multi-gigabyte
file. File-backed PieceTable originals reuse the bounded query and streaming
write seams already present. Retaining only edited pages makes memory use scale
with the configured active page plus the user's edits, while the overlay stream
preserves exact whole-document save semantics.

## Consequences

- A page is a storage/navigation unit, not a permanent document partition.
- A single line longer than the configured page target still requires scanning
  that logical line.
- Until reload/reopen, text joined across a source boundary may render on separate
  pages even though save and Ctrl+F treat it as one logical stream.
- The legacy `LargeFileBuffer` remains test-only infrastructure for its scanner
  and historical coverage; App open/reload policy uses `PagedFileBuffer`.
