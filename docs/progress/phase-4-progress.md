# Phase 4 Progress

Phase 4 is in progress. Phase 3 acceptance is recorded in
[`../phase-3-acceptance.md`](../phase-3-acceptance.md).

## Completed

- **4-a viewport-only syntax foundation**: file extensions select a small
  built-in set for Markdown, Rust, Python, and JSON. Pure editor lexers return
  ordered half-open scalar spans for only the supplied visible line; terminal
  rendering maps those spans to ANSI while composing active search/selection
  reverse video.
- Markdown styling covers ATX headings, quote/list markers, fence delimiters,
  and inline code. Code styling covers language keywords, quoted strings,
  numbers, Rust/Python line comments, and JSON literals.
- The renderer still obtains content through the bounded visible-window Buffer
  query. No full-file parse, syntax cache, background worker, dependency, or
  Project service was added.
- **4-b optional view indicators**: `F7` toggles a fixed line-number gutter and
  `F8` toggles one-cell space/tab indicators. Both settings are retained per
  buffer. Cursor reveal, horizontal scrolling, and mouse mapping all account
  for the gutter without changing document coordinates.
- **4-c explicit Markdown preview**: `F6` uses `pulldown-cmark` to build a
  read-only terminal document for the active buffer or active large-file page.
  Preview navigation uses the normal bounded viewport renderer, does not mutate
  source history, and restores the source viewport on `F6` or Escape. Parsing
  is absent from startup, typing, and ordinary render paths.

## Remaining Phase 4 Work

- Phase 4 unit/golden/PTY/performance/manual acceptance evidence.
