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

## Remaining Phase 4 Work

- Markdown preview toggle.
- Toggleable line numbers and whitespace indicators.
- Phase 4 unit/golden/PTY/performance/manual acceptance evidence.
