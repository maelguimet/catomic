# Phase 5 Progress

Phase 5 is complete. Its acceptance record is
[`../phase-5-acceptance.md`](../phase-5-acceptance.md).

## Completed

- **5-a local completion**: Plain mode collects unique sorted word extensions
  from a bounded current-buffer window only. `Ctrl+Space` or Tab opens the
  terminal UI, Tab/Shift+Tab cycle, Enter applies one undoable range edit, and
  Escape dismisses it.
- **5-b capability lifecycle**: `:project`/`:code` explicitly constructs a
  Project session rooted at the active file; `:plain`/`:text` destroys it.
  Plain startup owns no linter, discovery, index, LSP, or network service.
- **5-c on-demand linting**: a lazy extension-to-command config requires a
  `{file}` placeholder. `:lint` runs only for a saved Project file in a bounded,
  cancellable worker and parses common `file:line:column: message` output.
  `:diagnostics`, `:dnext`, and `:dprev` provide read-only listing and jumps,
  including already-discovered cross-file targets.
- **5-d explicit discovery**: `:files` starts a bounded, cancellable std-only
  walk that skips symlinks and common generated directories. The transient
  read-only picker opens or reuses selected files without blocking typing.
- **5-e cached Project paths**: path-like completion uses only the last explicit
  discovery result. It cannot trigger disk work, and Plain completion remains
  current-buffer-only.

## Acceptance

Exact completion/undo golden coverage, the real PTY Project discovery and save
flow, the complete default suite, and the 4,096-file release measurement pass.
See the acceptance record for the evidence matrix and current sample values.
