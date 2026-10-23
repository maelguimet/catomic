# Decision 0016: One focused surface owns input

Date: 2026-10-18

Status: accepted

## Context

Prompts, pickers, read-only views, and mobile overlays each kept their own
`Option` on `App`. Five separate precedence chains then picked a winner: the
shortcut scope, raw keys, paste, the mobile action bar, and the display buffer.
The chains disagreed whenever two surfaces were open at once. With a recovery
preview and a Replace prompt both open, typed characters reached the preview
while paste and Enter reached the prompt, and the mobile bar showed the
preview's buttons while its Accept button submitted the prompt.

## Decision

`App::focus` is a single `Focus` value: the editor, a prompt (search, Replace
or its review, command), a picker (word or emoji), a read-only view (help,
recovery preview, command preview, Markdown preview), or a mobile overlay. The
focused surface owns its state, so two surfaces cannot be open together.
Opening a surface closes the focused one through that surface's own close path,
which restores the viewport and any unresolved recovery candidate. A mobile
overlay is the only surface that keeps the focus beneath it, and closing the
overlay restores it.

`input/surfaces.rs` keeps the canonical key order as one table. Background
workers (lint, external command, goto) stay in their modules and only intercept
their own cancel key at their place in that order. Paste, the shortcut scope,
the mobile bar, and the display buffer all read the same focus.

## Consequences

- Paste and mobile actions always reach the surface that receives keys.
- A worker sets the shortcut scope only while the editor is focused.
- New temporary surfaces add a `Focus` variant and a close path; the compiler
  then lists every router that must handle them.
