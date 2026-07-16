# Catomic

Catomic should feel like Nano if Nano stopped being afraid of useful shortcuts.

It is not trying to be Vim.  
It is not trying to be Emacs.  
It is not trying to become Electron wearing terminal cosplay.

It should be:

- Fast
- Obvious
- Keyboard-friendly
- Hard to accidentally destroy work with
- Pleasant enough that opening it does not feel like entering a monastery

## Editing Model

- Normal text editor behavior by default
- Familiar shortcuts: `Ctrl+C` / `Ctrl+V` / `Ctrl+X` / `Ctrl+Z` / `Ctrl+Y` / `Ctrl+F` / `Ctrl+Q`
  (Note: in a raw terminal these are best-effort; see TODO.md "Terminal Realities" section for the feral raccoon details around paste and Ctrl keys.)
- Mouse support eventually, but not as a crutch
- Selection should behave like a GUI editor where possible
- Big-file mode should degrade gracefully instead of exploding

`Ctrl+F` opens incremental search. Matches move and highlight as you type.
Enter or Down moves forward, Up moves backward, and Escape closes search. The
same navigation wraps across oversized paged files without loading them whole.

`Ctrl+G` opens a cancellable 1-based goto-line prompt across ordinary and paged
files. `Ctrl+Shift+P` opens the first command prompt, with `goto N`,
`save`/`write`, and `quit`/`q`.

Hold Shift with the arrow keys to select text, or use `Ctrl+A` for the active
buffer/page. `Ctrl+C`, `Ctrl+X`, and `Ctrl+V` copy, cut, and paste through an
always-available internal clipboard; copy also sends OSC 52 to supporting
terminals. Bracketed terminal paste is inserted as one undoable edit. Terminal
emulators may intercept `Ctrl+Shift+C`/`Ctrl+Shift+V` before Catomic sees them.
Mouse clicks move the cursor, left-button drags select text, and a double click
selects the word or punctuation run under the pointer.

Visible lines are styled automatically for Markdown, Rust, Python, and JSON
based on the file extension. Highlighting is deliberately lexical and
viewport-only; opening a large file does not trigger a whole-document parse.
For Markdown files, `F6` builds a read-only rendered preview of the active
buffer (or active large-file page); press `F6` or Escape to return to editing.
Press `F7` to toggle line numbers and `F8` to show spaces and tabs. These view
settings are retained independently for each open buffer.

## Terminal Behavior

- Linux terminal only (for now)
- Use raw mode directly
- Draw with ANSI escape sequences
- Avoid curses if possible
- Detect terminal resize
- Minimize redraws
- Keep rendering dumb, predictable, and fast

## Buffer Strategy

Catomic needs a real text buffer, not "one giant string and prayers."

Possible approaches:

- **Gap buffer**: simple, good for normal editing
- **Piece table**: good undo story, good for files loaded from disk
- **Rope**: better for huge files, more complexity

**Default target**: piece table unless proven annoying.

## Big File Rules

For big files:

- Do not syntax-highlight everything
- Do not parse the whole file constantly
- Do not run linters automatically without limits
- Do not ask an LLM to rewrite the entire file by default
- Keep line indexing lazy or incremental
- Offer "large file mode" when needed

Every regular UTF-8 file remains editable. Small and medium files use one
in-memory PieceTable; oversized files use editable, file-backed line pages
instead of being rejected or opened read-only. The page size is configurable in
`~/.config/catomic/config.toml` (or `$XDG_CONFIG_HOME/catomic/config.toml`):

```toml
[big_files]
page_lines = 20000

[files]
auto_reload = true
```

Lower values trade more page transitions for less line metadata in memory.
Use `Ctrl+PageDown` and `Ctrl+PageUp` to move between file pages. The status
line shows the active page number and source byte range. Edits on visited pages
are retained, undo/redo follows edit order across pages, and `Ctrl+S` atomically
streams the complete logical document without materializing the whole file.
Page boundaries stay anchored to the opened file during a session and rebalance
after reload or reopen. `Ctrl+F` searches all pages, including unsaved page
edits and matches across edited boundaries; press Enter to run or Escape to
cancel.

Clean buffers reload automatically when another process changes or deletes the
file. Set `[files] auto_reload = false` to require manual confirmation instead.
Dirty buffers are never discarded automatically; Ctrl+R remains the explicit
check/reload fallback.

## Multiple Buffers

Pass multiple files on the command line to open them in one editor session:

```sh
catomic notes.txt todo.txt server.log
```

Use `Alt+PageDown` and `Alt+PageUp` to move to the next and previous buffer.
Each buffer keeps its own cursor, viewport, dirty state, file watcher, and paged
file position. The status line shows `buffer N/M` when more than one is open.
`Ctrl+S` saves the active buffer; `Ctrl+Q` warns if any open buffer is dirty.

## File Watching

Catomic should notice when a clanker or another process edits the current file.

Behavior:

- If file changed externally and local buffer is clean: reload
- If file changed externally and local buffer has edits: warn
- If possible, show a small diff or offer:
  - reload from disk
  - keep local version
  - save as conflict copy

## Markdown

Native `.md` support should mean:

- Readable headings
- Lists
- Code blocks
- Blockquotes
- Maybe inline emphasis
- No cursed browser engine
- No "full Markdown spec compliance" illness

Possible modes:

- Edit mode
- Rendered preview mode
- Split mode later, if terminal width allows it

## Linter Support

Linter support should be command-based.

Example config idea:

```ini
[linters]
python = "ruff check {file}"
javascript = "eslint {file}"
markdown = "markdownlint {file}"
```

Rules:

- Run manually first
- Maybe run on save later
- Parse common output formats
- Jump to error line
- Never block typing

## Autocomplete

Autocomplete should start simple:

- Words from current buffer
- File paths
- Language keywords
- Snippets

Later:

- LSP support
- Local model completion
- Remote API completion

Tab accepts. Escape dismisses. No aggressive ghost-text demon unless explicitly enabled.

## LLM Support

LLM support should be powerful but caged.

Possible shortcut:

- `Ctrl+Shift+P` opens command palette
- User chooses an LLM action
- Catomic sends selected text, current file, or explicit instruction blocks

Instruction block format:

```
>>> catomic
Refactor this function.
Keep behavior identical.
Do not edit outside this block unless necessary.
<<<
```

LLM safety rules:

- Prefer diff/patch output over full-file replacement
- Always preview changes before applying
- Every LLM edit must be undoable
- Never send hidden files or unrelated project files by default
- Local and remote OpenAI-compatible APIs should both work
- Remote API use should be obvious, not sneaky

## Plugin System

Plugin support should come after the core editor is stable.

First version:

- External commands
- Hooks
- Simple config files

Later:

- Scripting API
- Editor commands
- Custom keybindings
- Custom render overlays

**Do not build a cathedral before the text cursor works.**

## Cat Features

Mandatory cat nonsense:

- Optional cat status messages
- Cat-themed panic messages
- Maybe `:meow`
- Maybe autosave creates `.catnap` recovery files
- Absolutely no productivity-hostile gimmicks enabled by default

---

See [TODO.md](./TODO.md) for build phases, stack decisions, and research on existing editors.
