# Decision 0005: pulldown-cmark for Explicit Preview

## Decision

Use `pulldown-cmark` without default features when the user explicitly enters
Markdown preview. An explicit render command treats the active in-memory buffer
or active large-file page as Markdown regardless of its path, then converts it
once into a read-only terminal text buffer. The same presentation entry point
is available to authored in-app Markdown surfaces. Ordinary editing and startup
do not construct a parser.

## Dependency justification

1. The standard library has no CommonMark parser, and maintaining another
   partial Markdown grammar would not make preview trustworthy.
2. Only explicitly opened Markdown surfaces use it, including `F6` preview.
3. Plain startup and typing paths do not parse or allocate preview content.
4. Event-to-terminal rendering has unit tests; app and PTY tests cover toggling,
   read-only behavior, and teardown.
5. Removal is isolated to `editor::markdown_preview`, the F6 view toggle, and
   one Cargo dependency; source editing and lexical highlighting remain intact.

## Bounds

Preview parses the current active Buffer. For a paged large file that means
only the active editable page, never the complete backing file. The generated
preview is then rendered with the existing visible-window query.

Input is capped at 10 MiB and rendered output at 32 MiB. The reading column is
capped at 100 cells, prose and code reflow at narrower widths, and long graphemes
degrade without splitting a cluster. Tables are accumulated in a short-lived
intermediate model with explicit row, column, and text caps. Per-cell output is
capped at 40 display cells and clipped grapheme-safely; a natural-width grid is
used only when it fits, otherwise rows become wrapped label/value entries. Raw
HTML remains inert preview text and passes through terminal-control sanitization.
