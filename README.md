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

## Build and Install

Catomic targets Linux and stable Rust. Build the optimized binary or install it
from this checkout:

```sh
cargo build --release
# binary: target/release/catomic

cargo install --path .
```

Run `catomic`, optionally followed by one or more file paths. The completed
v0.1 roadmap and verification record is in
[`docs/v0.1-acceptance.md`](docs/v0.1-acceptance.md).

## Editing Model

- Normal text editor behavior by default
- Familiar shortcuts: `Ctrl+C` / `Ctrl+V` / `Ctrl+X` / `Ctrl+Z` / `Ctrl+Y` / `Ctrl+F` / `Ctrl+Q`
  (Note: in a raw terminal these are best-effort; see TODO.md "Terminal Realities" section for the feral raccoon details around paste and Ctrl keys.)
- Mouse support without making it a crutch
- Selection should behave like a GUI editor where possible
- Big-file mode should degrade gracefully instead of exploding

`Ctrl+F` opens incremental search. Matches move and highlight as you type.
Enter or Down moves forward, Up moves backward, and Escape closes search. The
same navigation wraps across oversized paged files without loading them whole.
`Ctrl+Shift+F` opens a two-stage Find/Replace prompt. `replace-all` in the
command prompt replaces every non-overlapping match in an ordinary buffer;
paged-file replacement refuses explicitly instead of silently changing one page.

`Ctrl+G` opens a cancellable 1-based goto-line prompt across ordinary and paged
files. `Ctrl+Shift+S` opens Save As; enter a filename such as `hello.txt`, a
relative or absolute path, or a home-relative path such as `~/notes/hello.txt`.
An existing destination requires submitting the same path again before Catomic
overwrites it. `Ctrl+S` opens this prompt automatically for an untitled buffer.
`Ctrl+Shift+P` opens the command prompt, with `goto N`,
`save`/`write`, `save as PATH`, and `quit`/`q`.

`Ctrl+O` opens a path in a buffer, `Ctrl+N` creates an untitled buffer, and
`Ctrl+W` closes the active clean buffer. A dirty buffer refuses to close until
saved; use the explicit `close!` command to discard it. The command prompt also
accepts `open PATH`, `new`, and `close`.

`Ctrl+H` or `F1` opens the built-in shortcut reference. The help view is
read-only; use the arrow keys or Page Up/Down to navigate and Escape to return.
The command prompt aliases `help` and `shortcuts` open the same view.

Hold Shift with the arrow keys to select text, or use `Ctrl+A` for the active
buffer/page. `Ctrl+C`, `Ctrl+X`, and `Ctrl+V` copy, cut, and paste through an
always-available internal clipboard; copy also sends OSC 52 to supporting
terminals. Bracketed terminal paste is inserted as one undoable edit. Terminal
emulators may intercept `Ctrl+Shift+C`/`Ctrl+Shift+V` before Catomic sees them.
Mouse clicks move the cursor, left-button drags select text, and a double click
selects the word or punctuation run under the pointer.

`Home`/`End` move to line edges, `Ctrl+Home`/`Ctrl+End` move to document edges,
and Page Up/Down move by one visible page. `Ctrl+Left`/`Ctrl+Right` move by word;
`Ctrl+Backspace`/`Ctrl+Delete` remove a word as one undoable edit. Add Shift to
the movement forms to extend the selection.

Enter preserves the current line's indentation and adds one configured tab
level after common block openers. Tab indents selected lines without replacing
them; Shift+Tab unindents the current or selected lines as one undoable edit.

Visible lines are styled automatically for Markdown, Rust, Python, and JSON
based on the file extension. Highlighting is deliberately lexical and
viewport-only; opening a large file does not trigger a whole-document parse.
For Markdown files, `F6` builds a read-only rendered preview of the active
buffer (or active large-file page); press `F6` or Escape to return to editing.
Press `F7` to toggle line numbers and `F8` to show spaces and tabs. These view
settings are retained independently for each open buffer. `F9` toggles bounded
soft wrapping at the terminal width; wrapped continuations preserve document
coordinates and mouse mapping instead of inserting newlines.

Cursor movement, deletion, clipping, and terminal placement respect extended
grapheme clusters and terminal-cell width, including combining marks, wide
characters, emoji sequences, and tabs.

Catomic accepts valid UTF-8 and preserves an optional UTF-8 BOM plus the
detected LF, CRLF, or CR line-ending style across Save, Save As, and reload. The
active format appears in the status line. Oversized paged files currently
support LF and CRLF; BOM-prefixed or CR-only files must remain below the paged
threshold. UTF-16 and other non-UTF-8 encodings are rejected rather than
silently corrupted.

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

Every supported UTF-8 file remains editable. Small and medium files use one
in-memory PieceTable; oversized files use editable, file-backed line pages
instead of being rejected or opened read-only. The page size is configurable in
`~/.config/catomic/config.toml` (or `$XDG_CONFIG_HOME/catomic/config.toml`):

```toml
[editor]
tab_size = 4

[big_files]
page_lines = 20000

[files]
auto_reload = true

[cat]
status_messages = true

[recovery]
enabled = false
interval_secs = 30
max_bytes = 1048576

[languages.rs]
tab_size = 4
linter = "cargo check --message-format short {file}"

[languages.py]
tab_size = 2
linter = "ruff check {file}"
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

The small ASCII cat badge in the status line is enabled by default. Set
`[cat] status_messages = false` if you want the completely plain status format;
the setting changes presentation only.

If Catomic panics, its panic hook first restores the terminal and prints a short
cat-themed notice that promises only the safety of the last explicit save. The
ordinary Rust panic details follow for debugging.

Crash recovery is opt-in. With `[recovery] enabled = true`, dirty named files
up to `max_bytes` get an atomic, owner-only sibling such as
`notes.txt.catnap` after the configured interval. Untitled, oversized, and
paged files are skipped; normal startup and typing never create a sidecar when
recovery is disabled. If a newer sidecar exists on the next open, Catomic
offers `:recover`. Recovery opens read-only, Enter applies it as one undoable
buffer edit, and Escape leaves the source untouched. An ordinary successful
save removes the sidecar. Catomic never replaces the source automatically.

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

Linter support is command-based and Project-only. Enter Project mode with
`:project` (or `:code`), then use `:lint` on a saved file. Configuration is
loaded lazily from `$XDG_CONFIG_HOME/catomic/config.toml` or
`~/.config/catomic/config.toml`:

Language-specific settings are preferred because tab width and linting stay
together:

```toml
[editor]
tab_size = 4

[languages.py]
tab_size = 4
linter = "ruff check {file}"

[languages.js]
tab_size = 2
linter = "eslint {file}"
```

The older `[linters]` extension-to-command table remains supported. Every
linter command must contain `{file}`. Language-specific commands take
precedence over legacy mappings. Tab inserts spaces to the next configured stop
when no completion candidate exists; the insertion is one undoable edit.
Commands run asynchronously with bounded output and can be cancelled with Escape.
`:diagnostics` (or `:dlist`) opens the result list; `:dnext` and `:dprev` jump
between diagnostics, opening already-discovered files when needed.

Project file discovery is also explicit: `:files` performs one bounded,
cancellable scan rooted at the active file's directory and opens a read-only
picker. Nothing scans while Catomic is in Plain mode.

Rules:

- Run manually first
- Use an explicit `on_save` named-command hook when automatic checks are desired
- Parse common output formats
- Jump to error line
- Never block typing

## Autocomplete

Press `Ctrl+Space` or Tab to request completion, Tab/Shift+Tab to cycle, Enter
to accept, and Escape to dismiss. Plain mode derives candidates only from a
bounded window of the current buffer. Project mode can additionally complete
path-like prefixes from the most recent explicit `:files` result; it never
starts a scan merely because completion was requested.

Current completion sources:

- Words from current buffer
- Cached Project file paths

Later:

- LSP support
- Local model completion
- Remote API completion

Acceptance is one undoable replacement. No aggressive ghost-text demon is
enabled.

## Keybindings

Normal-mode chords can override existing editor actions without replacing their
save, quit, undo, completion, or view logic:

```toml
[keybindings]
"ctrl+w" = "save"
"alt+s" = "save-as"
"alt+f" = "search"
"ctrl+shift+g" = "command-prompt"
```

Supported actions are `help`, `save`, `save-as`, `open`, `new`, `close`,
`replace`, `quit`, `reload`, `search`, `goto-line`, `command-prompt`, `undo`,
`redo`, `complete`, `next-buffer`, `previous-buffer`, `next-page`,
`previous-page`, `markdown-preview`, `line-numbers`, `whitespace`, and
`soft-wrap`. Chords use `ctrl`, `alt`, and `shift` plus a character, navigation
key, or `f1` through `f12`. Prompt and picker keys remain local while those
interfaces are active.

## External commands

Named shell commands are opt-in and run only after `:run <name>`:

```toml
[commands.upper]
command = "tr '[:lower:]' '[:upper:]'"
input = "selection"
output = "replace-input"
timeout_secs = 10

[commands.date]
command = "date +%F"
output = "insert"
```

`input` is `none` (the default), `selection`, or `buffer`. `output` is
`preview` (the default), `insert`, or `replace-input`; replacing input requires
selection or buffer input. `{file}` expands to the shell-quoted absolute active
path, and the command runs from that file's directory.

Commands never block typing. Input is capped at 16 MiB, stdout and stderr at
1 MiB each, and timeouts are limited to 1–300 seconds. Escape cancels a running
command. Completed output opens read-only; successful, complete output requires
Enter before insertion or replacement and applies as one undoable edit. Failed
or truncated output cannot apply. Catomic invokes `/bin/sh -c`, so configured
commands are trusted user code and may have side effects outside the editor.

The same named commands can be attached to lifecycle hooks:

```toml
[hooks]
on_open = ["inspect"]
on_save = ["check"]
before_llm = ["redact-check", "policy-check"]
```

Hooks run sequentially in listed order and use the same bounded execution and
read-only result preview as `:run`. A failed, timed-out, stale, or cancelled
command stops the remaining chain. `on_save` starts only after a successful
atomic save; confirmed hook edits make the buffer dirty again. `before_llm`
finishes before Catomic prepares the ordinary endpoint/context confirmation, so
no model client or network request exists while hooks are running.

## LLM Support

LLM support is explicit, transient, and caged. Open the command prompt with
`Ctrl+Shift+P`; Catomic shows the exact context extent, model, and endpoint
before anything can be sent. Enter confirms the network request and Escape
cancels it without constructing a client.

Configure any OpenAI-compatible local or remote endpoint lazily in
`$XDG_CONFIG_HOME/catomic/config.toml` or `~/.config/catomic/config.toml`:

```toml
[llm]
base_url = "http://127.0.0.1:8080/v1"
model = "local-model"
api_key_env = "OPENAI_API_KEY"
timeout_secs = 120
```

The API key is read from the named environment variable only after Enter.
Endpoint URLs are canonicalized before confirmation and cannot contain
credentials, whitespace, a query, or a fragment. With no configuration, the
local endpoint and model shown above are used.

Current-buffer commands work from Plain mode:

- `:meow <instruction>` sends the active selection. With no selection, place
  the cursor inside an instruction block; its text becomes both instruction
  and bounded context.
- `:bigmeow <instruction>` sends the current file. With no command argument,
  the instruction comes from the block under the cursor.
- An instruction beginning with `explain` opens a read-only answer instead of
  an edit preview.

Instruction block format:

```
>>> catomic
Refactor this function.
Keep behavior identical.
Do not edit outside this block unless necessary.
<<<
```

LLM safety rules:

- Context is capped at 64 KiB and 2,000 lines and fails closed rather than
  truncating silently.
- HTTP redirects are refused and ambient proxy settings are ignored, so context
  is sent directly to the confirmed endpoint only.
- Active-context dotfile paths and obvious secret-like lines are called out in
  the Enter confirmation.
- Edits must be a validated single-file unified patch whose headers name the
  confirmed active path. A selected region may instead use the strict
  `catomic_replacement` JSON envelope.
- Every edit opens a read-only preview, requires a second Enter to apply, and
  becomes one undoable buffer transaction. No command writes a file.

Repo-aware commands require explicit Project mode (`:project` or `:code`) and
a saved active file inside a Git repository:

- `:gitmeow <instruction>` and `:megameow <instruction>` capture bounded Git
  state and a bounded file map on a cancellable worker, then show a separate
  send confirmation.
- The model can make at most eight read-only broker requests: list files, read
  a bounded file range, grep, or show a file diff. The total broker response
  budget is 128 KiB; symlinks, unknown paths, oversized files, and path escapes
  are refused.
- Dot paths are omitted from the broker file map. Direct reads and diffs refuse
  obvious secret-like content; grep skips sensitive files and reports how many
  were omitted.
- HEAD, branch, status, tracked diff, and every retrieved file are rechecked
  after the response and again before preview apply. Drift discards/refuses the
  proposal.
- Git context disables pagers, fsmonitor, external diff, and textconv helpers;
  repository configuration cannot launch helper programs during capture, and
  inherited `GIT_*` variables cannot redirect repository identity.

`:feralmeow` is not implemented. Wide multi-file proposals are deliberately
outside the Phase 6 single-file safety contract.

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

- Toggleable cat status badge
- Helpful cat-themed panic notice after terminal restoration
- Useful, explicitly confirmed `:meow` LLM command
- Opt-in, bounded `.catnap` autosave and preview-first recovery
- Absolutely no productivity-hostile gimmicks enabled by default

---

See [TODO.md](./TODO.md) for build phases, stack decisions, and research on existing editors.
