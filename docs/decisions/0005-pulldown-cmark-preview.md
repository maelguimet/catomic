# Decision 0005: pulldown-cmark for Explicit Preview

## Decision

Use `pulldown-cmark` without default features when the user explicitly enters
Markdown preview. Convert the active in-memory buffer or active large-file page
once into a read-only terminal text buffer; ordinary editing and startup do not
construct a parser.

## Dependency justification

1. The standard library has no CommonMark parser, and maintaining another
   partial Markdown grammar would not make preview trustworthy.
2. Only Plain/Text Markdown preview uses it, after an explicit `F6` invocation.
3. Plain startup and typing paths do not parse or allocate preview content.
4. Event-to-terminal rendering has unit tests; app and PTY tests cover toggling,
   read-only behavior, and teardown.
5. Removal is isolated to `editor::markdown_preview`, the F6 view toggle, and
   one Cargo dependency; source editing and lexical highlighting remain intact.

## Bounds

Preview parses the current active Buffer. For a paged large file that means
only the active editable page, never the complete backing file. The generated
preview is then rendered with the existing visible-window query.

Tables are accumulated in a short-lived intermediate model so parser-provided
alignment metadata survives until every row has been measured with Catomic's
terminal-cell layout helpers. Per-cell output is capped at 40 display cells and
clipped grapheme-safely with an ellipsis; the complete table retains its natural
measured width and the existing read-only cursor provides horizontal navigation.
Raw HTML remains inert preview text and passes through the renderer's terminal
control sanitization.
