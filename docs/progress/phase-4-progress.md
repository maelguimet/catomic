# Phase 4 Progress

Phase 4 is complete. Its acceptance record is
[`../phase-4-acceptance.md`](../phase-4-acceptance.md).

## Completed

- **4-a viewport-only syntax foundation**: file extensions select a small
  built-in set for Markdown, Rust, Python, and JSON. Pure editor lexers return
  ordered half-open scalar spans for only the supplied visible line; terminal
  rendering maps those spans to ANSI while composing active search/selection
  reverse video.
- Markdown styling covers ATX headings, quote/list markers, fence delimiters,
  and inline code. Code styling covers language keywords, quoted strings,
  numbers, Rust/Python line comments, and JSON literals.
- The ordinary renderer still obtains content through the bounded
  visible-window Buffer query. The viewport styling foundation added no
  full-file parse, syntax cache, background worker, dependency, or Project
  service.
- **4-b optional view indicators**: `F7` toggles a fixed line-number gutter and
  `F8` toggles one-cell space/tab indicators. Both settings are retained per
  buffer. Cursor reveal, horizontal scrolling, and mouse mapping all account
  for the gutter without changing document coordinates.
- **4-c explicit Markdown preview**: `F6` uses `pulldown-cmark` to build a
  read-only terminal document for the active buffer or active large-file page.
  Preview navigation uses the normal bounded viewport renderer, does not mutate
  source history, and restores the source viewport on `F6` or Escape. Parsing
  is absent from startup, typing, and ordinary render paths.

## Acceptance

The exact preview golden, live F6/F7/F8 PTY smoke, 10 MiB preview/render
measurement, and manual terminal checklist pass. See the acceptance record for
the evidence matrix and current sample values.
