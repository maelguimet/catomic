# Catomic user guide

Catomic is a Linux-first, modeless terminal text editor. It does not scan
repositories, start linters, or contact a network service during startup or
ordinary editing. Linter and model-assisted actions run only after you invoke
them explicitly.

Catomic is currently open-beta software. Keep backups of important files, and
read [File formats and save safety](#file-formats-and-save-safety) before using
it on files with unusual links, ACLs, or extended attributes.

## Contents

- [Requirements and installation](#requirements-and-installation)
- [Updating, backup, and rollback](#updating-backup-and-rollback)
- [Starting Catomic](#starting-catomic)
- [The editor screen](#the-editor-screen)
- [Editing and navigation](#editing-and-navigation)
- [Finding, replacing, and going to a line](#finding-replacing-and-going-to-a-line)
- [Working with multiple buffers](#working-with-multiple-buffers)
- [Saving and external changes](#saving-and-external-changes)
- [Views, highlighting, and Markdown](#views-highlighting-and-markdown)
- [Large files](#large-files)
- [Completion](#completion)
- [Linting](#linting)
- [External commands and hooks](#external-commands-and-hooks)
- [Model-assisted commands](#model-assisted-commands)
- [Crash recovery](#crash-recovery)
- [Configuration reference](#configuration-reference)
- [Shortcut reference](#shortcut-reference)
- [Command reference](#command-reference)
- [File formats and save safety](#file-formats-and-save-safety)
- [Troubleshooting](#troubleshooting)

## Requirements and installation

Catomic currently targets Linux terminals and stable Rust. The package declares
Rust 1.87 as its minimum supported version.

Clone the repository and build an optimized binary:

```sh
git clone https://github.com/maelguimet/catomic.git
cd catomic
cargo build --release --locked
./target/release/catomic
```

To install it into Cargo's binary directory, normally `~/.cargo/bin`, and
provision the private commented user configuration:

```sh
./scripts/install.sh
```

The installer creates `$XDG_CONFIG_HOME/catomic/config.toml` when the XDG root
is absolute, otherwise `~/.config/catomic/config.toml`. It uses directory mode
`0700` and file mode `0600`, publishes the template without overwriting a racing
path, and leaves any existing configuration byte-for-byte untouched.

Make sure Cargo's binary directory is in `PATH`, then verify the installation:

```sh
catomic --version
catomic --help
```

## Updating, backup, and rollback

Catomic has an explicit updater. Checking is read-only:

```sh
catomic update --check
```

It reports the installed version and method, the available version or source
revision, whether the update can be applied, and the trusted source it queried.
The command contacts the official GitHub repository because checking a remote
version requires a network request, but it does not fetch into the checkout,
write a cache, create a backup, or change the binary.

The default update prints its source and asks before any network or install
action. Use `--yes` for deterministic non-interactive operation:

```sh
catomic update
catomic update --yes
```

Use `--backup` to make a private, timestamped copy of user-owned state before
the update is downloaded or built:

```sh
catomic update --backup
```

Backups live below
`$XDG_STATE_HOME/catomic/update-backups` (normally
`~/.local/state/catomic/update-backups`). They include Catomic's XDG config,
data, and state trees, excluding older updater backups. Backup directories use
mode `0700` and regular files use `0600`; symlinks are copied without following
them. Caches are not user state and are not included. The updater prints the
exact backup path.

### Supported install methods

- An official managed x86_64 Linux release downloads the exact architecture
  asset and its SHA-256 file from the GitHub release. HTTPS origins and
  redirects are allowlisted, requests have bounded timeouts, responses and
  declared asset sizes are capped, and the candidate's checksum and version are
  verified before it can run or replace the installed binary.
- A binary built in an official checkout whose current branch can fast-forward
  to `master`, including one installed by `./scripts/install.sh`, retains that
  checkout as its update source.
  Catomic preserves local changes, checks the official remote revision, refuses
  non-fast-forward history, fetches without running hooks, and builds in an
  isolated temporary worktree. The new revision must build successfully and
  validate the existing configuration before the executable is replaced. Only
  then is the source checkout fast-forwarded and the local changes reapplied.
- If that source checkout no longer exists, Catomic runs the official Cargo git
  install command itself. `--check` remains unsupported for a missing checkout
  and exits without writing.
- Cargo registry installs, detached Git installs, forks, diverged branches, and
  architectures without a managed release are reported as unsupported.

Dirty official source checkouts are stashed with untracked files before an
update and popped afterward with their staged state restored. Git reports any
conflicts normally.

### Atomic install and recovery

Managed releases and retained-checkout updates stage the new executable beside
the installed one, sync it, and atomically rename over it. Before that rename,
Catomic creates a sibling rollback binary containing the old bytes. A failed
download, checksum, build,
configuration validation, or staging step leaves the installed executable
untouched. If final source fast-forwarding fails after replacement, Catomic
automatically restores the old binary.

The missing-checkout fallback delegates the replacement directly to `cargo
install` and therefore does not create Catomic's sibling rollback binary.

On managed-release and retained-checkout success Catomic prints the old and new
versions, backup status, rollback path, and an exact recovery command. Roll back
manually with the printed command, which has this shape:

```sh
cp -- /path/to/.catomic.rollback-VERSION-TIMESTAMP /path/to/catomic
```

The rollback copy is intentionally retained after success. Remove it manually
after the new version has behaved correctly. User configuration is never
regenerated, normalized, migrated, or deleted by the updater. A future schema
migration must be an explicit, separately confirmed operation; an incompatible
configuration makes the update fail closed.

`cargo uninstall catomic` removes a Cargo-managed executable but intentionally
does not remove XDG configuration, data, state, updater backups, or retained
rollback binaries. For an official standalone binary, remove the installed
binary and any sibling `.catomic.rollback-*` files explicitly; user-owned XDG
state remains untouched unless you separately choose to delete it.

Updater exit codes are stable for automation: `0` means current, successfully
updated, or user-cancelled; `2` is command-line usage; `3` unsupported install;
`4` remote/checksum policy failure; `5` unsafe source state or prompt I/O; `6`
backup failure; `7` candidate configuration failure; `8` build failure;
and `9` install or rollback failure.

If the updater is unavailable for a source install, the manual equivalent is:

```sh
git pull --ff-only
./scripts/install.sh --force
```

To remove a Cargo-installed copy:

```sh
cargo uninstall catomic
```

## Starting Catomic

Open a file:

```sh
catomic notes.md
```

Open a filename containing spaces without shell quoting:

```sh
catomic hello world.md
```

Start with an untitled empty buffer:

```sh
catomic
```

If a named path does not exist, Catomic opens an empty buffer for that path. The
file is not created until you save it.

Every non-command word forms one path, joined with spaces. The example above
therefore opens exactly `hello world.md`, not separate startup buffers. Shell
quoting remains supported when you prefer it:

```sh
catomic "hello world.md"
```

The first argument `config` or `update` selects that command instead of a file.
Those commands reject unsupported arguments. A working-directory file with a
reserved or option-like name remains an ordinary file when written as an
explicit path:

```sh
catomic ./config
catomic ./update
catomic ./-draft.md
```

The file path and file contents must be valid UTF-8. The editor also requires
a UTF-8 locale selected by the first non-empty value among `LC_ALL`,
`LC_CTYPE`, and `LANG`. Help and version output remain available when the locale
is invalid because they do not enter terminal raw mode.

## The editor screen

The main area contains the active buffer. The bottom line shows a transient
message when an operation needs attention; otherwise it shows only the active
path (or `[untitled]`) beside the small cat decoration. The parent path is
muted and the filename is red. Exceptional navigation context is appended as
`file N/M` or `page N`. The terminal window title tracks the active filename and Catomic
restores the previous title when the editor exits.

The small cat decoration is enabled by default. It changes presentation only
and can be disabled with `[cat] status_messages = false`.

The persistent row has no full-width background: it clears the row, then draws
the compact identity in subdued colors. Transient informational messages,
warnings, errors, and interactive prompts still use separate full-width
semantic styles. `NO_COLOR`, `TERM=dumb`, and monochrome terminals use dim and
bold distinctions for the persistent identity and inverse-video boundaries for
transient messages. Long status and prompt text is clipped at the terminal edge
without splitting a Unicode grapheme or allowing control characters to reach
the terminal.

Press `Ctrl+H` or `F1` at any time to open the built-in task reference. It is
rendered from Markdown and read-only: use the configured Find shortcut to
search it, the arrow and page keys to navigate, and `Escape` to return.

### Prompts and read-only views

`Ctrl+Shift+P` or `F2` opens the command prompt. Commands are entered without a
leading colon. For example, type `save`, not `:save`. This guide uses command
names without a colon for that reason.

Most prompts and read-only result views follow the same small interaction model:

- `Enter` accepts, opens, applies, or advances the current operation;
- `Escape` cancels or closes it;
- arrow keys and page keys navigate read-only content; and
- `Ctrl+Q` still reaches the normal quit guard; and
- `Ctrl+Shift+C` immediately interrupts through the SIGINT teardown path.

## Editing and navigation

Catomic uses familiar modeless editing. Type to insert text. `Backspace` and
`Delete` remove text, and `Enter` inserts a newline.

Press `Insert` in the normal editing surface to toggle the session-wide typing
state. Insert mode adds ordinary typed characters before the cursor. Overwrite
mode replaces exactly one Unicode grapheme under the cursor and uses a steady
block cursor; at an empty line, end of line, or end of file it inserts instead,
so ordinary typing never overwrites a newline. Each overwritten character is
one ordinary undoable typing transaction.

Overwrite mode affects direct character typing only. Selection replacement
keeps its usual range semantics, while paste, completion, indentation, command
output, and confirmed model edits keep their existing transactional behavior.
Prompts and read-only views use their own input handling and the terminal's
default cursor shape; closing them resumes the session's insert/overwrite state.

Cursor movement and deletion operate on Unicode grapheme clusters rather than
raw bytes. Combining marks, wide characters, emoji sequences, and tabs remain
one visual unit where appropriate.

### Movement

| Action | Key |
| --- | --- |
| Move one grapheme or line | Arrow keys |
| Move to start or end of line | `Home` / `End` |
| Move to start or end of document | `Ctrl+Home` / `Ctrl+End` |
| Move by one visible viewport | `PageUp` / `PageDown` |
| Move by word | `Ctrl+Left` / `Ctrl+Right` |
| Move by paragraph | `Ctrl+Up` / `Ctrl+Down` |
| Delete previous or next word | `Ctrl+Backspace` / `Ctrl+Delete` |

Add `Shift` to the grapheme, line, word, page, and document-edge movement forms
to extend the selection. `Ctrl+A` selects the active ordinary buffer or the
current page of a paged file. Paragraph movement follows the exception below.

Some terminal emulators reserve `Ctrl+Shift+Left` and `Ctrl+Shift+Right` for
terminal-tab navigation and never send those events to Catomic. Use
`Alt+Shift+Left` and `Alt+Shift+Right` as the built-in word-selection fallbacks;
the standard chords remain available when the terminal forwards them.

A paragraph is a maximal run of non-blank logical lines; one or more blank
lines separate paragraphs. `Ctrl+Down` skips the remainder of the current
paragraph and all separating blank lines, then lands on the first non-blank
line of the next paragraph. `Ctrl+Up` first lands on the first line of the
current paragraph; another press moves to the first line of the previous
paragraph. From a blank line, either key skips the blank run in its direction.
Movement clamps at the document edge, preserves the current terminal-cell
column when the target line permits it, and snaps to a grapheme boundary.
Plain `Ctrl+Up`/`Ctrl+Down` clears an active selection like other non-extending
navigation; it does not infer a selection-extending paragraph action from
`Shift`.

`Insert` toggles the session-wide overwrite mode for printable typing, and
switching buffers preserves that shared state. The steady block cursor indicates
overwrite mode. A typed character in overwrite mode replaces one complete
Unicode grapheme. Newlines, paste, prompts, command/model results, and other edit
paths keep their normal insert/replace semantics.

Catomic requests the Kitty enhanced-keyboard protocol and xterm modified-key
mode 2 while its alternate screen is active. A terminal path that honors a
request and preserves the physical Backspace key reports plain `Backspace`
without modifiers and `Ctrl+Backspace` with `Control`, so the former deletes
one grapheme and the latter deletes one word. Catomic restores the terminal's
previous keyboard modes when the session ends.

Legacy terminal paths may emit the same byte for both physical keys. No
application can distinguish the chord after that information has been lost;
Catomic treats the event as plain `Backspace` and deletes one grapheme. For a
portable fallback, map an unused, distinguishable chord to the existing
word-delete action, for example:

```toml
[keybindings]
delete-word-backward = ["ctrl+u"]
```

### Mouse selection

- A left click moves the cursor.
- A left-button drag selects text.
- A double click selects the word or punctuation run under the pointer.

Mouse coordinates follow visible terminal cells, including tabs, wide
characters, line numbers, and soft-wrapped lines.

A click in the line-number gutter moves to the start of that displayed row; for
a soft-wrapped continuation, that is the continuation's first document column.
A click past the rendered end of a line moves to its end. The bottom status row
is not document content: dragging across its persistent path selects that text,
and `Ctrl+C` copies it through the same clipboard path as a document selection.
Prompts and read-only views ignore document clicks; close the active surface
before positioning the editable source cursor.

Completing any Catomic selection, including keyboard selection, Select All,
dragging, double-clicking a word, or dragging across the persistent path,
immediately updates the process-local clipboard and emits a bounded OSC 52
clipboard write. An empty selection changes neither path. Explicit `Ctrl+C`
additionally invokes the desktop system-clipboard helper described below.

Terminal-native selection remains available through the emulator's mouse
bypass modifier, commonly `Shift`. Use it when terminal scrollback or text
outside Catomic's document mapping is the intended source. Terminal
configuration can change or disable that bypass; Catomic does not override it.

The mouse wheel scrolls three visible rows per normalized wheel event without
moving the document cursor or selection. With soft wrap enabled those are
wrapped visual rows; otherwise they are logical lines. Horizontal scroll is
preserved. If the logical cursor leaves the viewport, Catomic hides the terminal
caret until the next keyboard navigation or editing action reveals it. Wheel
events over the status row or an active prompt do not scroll the underlying
document. Help, Markdown preview, and proposal/result views use the same
viewport-only scrolling without changing their source buffer.

In mobile mode, tap **Menu** for touch-accessible cursor, selection, scroll,
page, file, view, and model-assisted actions. Termux finger drags do not emit
editor drag events; choose **Select: mark start, then tap end** and tap the other
boundary. Finger scrolling moves only the viewport and preserves the current
cursor and selection. The status and action rows never map into document text.

### Clipboard and paste

`Ctrl+C` and the cut actions keep Catomic's process-local clipboard, invoke a
real system-clipboard helper, and emit bounded, ST-terminated OSC 52. The first
successful applicable helper is used: `wl-copy` under Wayland; `xclip` or
`xsel` under X11; `clip.exe` under WSL; and `termux-clipboard-set` under Termux.
Install `wl-clipboard`, `xclip`, or `xsel` when the desktop environment does not
already provide one. Clipboard text is passed through the helper's standard
input, never interpolated into a shell command.

The helper path is terminal-emulator independent and is the normal local Linux
copy path. OSC 52 remains useful when Catomic runs over SSH and the desired
clipboard belongs to the local terminal. Catomic emits both transports because
the external helper addresses the machine running Catomic while OSC 52
addresses the terminal endpoint. `Ctrl+C` never interrupts Catomic;
`Ctrl+Shift+C` remains the interrupt action.

`Ctrl+V` pastes Catomic's process-local clipboard, which is shared by every
buffer in the current session. Completed selections update that internal value
and the OSC 52 path immediately; invoking the desktop helper is reserved for
explicit copy and cut actions.

`Ctrl+K` cuts the current logical line, including its terminating line break
when present. Consecutive presses append complete lines to one clipboard payload
in document order, so one `Ctrl+V` restores them together. Each press is a
separate undo transaction. Moving, editing, switching buffers, or using another
action ends the append chain; an active selection keeps ordinary selection-cut
behavior.

Bracketed terminal paste is inserted as one undoable edit. Terminal emulators,
multiplexers, desktop shortcuts, and SSH clients may intercept clipboard chords
before Catomic receives them; see [Troubleshooting](#troubleshooting).

### Undo and redo

- `Ctrl+Z` — Undo.
- `Ctrl+Y` / `Ctrl+Shift+Z` — Redo.

Catomic keeps `Ctrl+Shift+Z` as a default GUI-style redo alias. If a terminal
reports that chord without the Shift modifier, it is indistinguishable from
`Ctrl+Z`, so Catomic performs undo. The `undo` and `redo` actions remain
remappable through `[keybindings]`.

Grouped actions such as a bracketed paste, selected-line indentation, Replace
All, a confirmed command result, or a confirmed model edit are each one undoable
transaction.

### Indentation

`Tab` inserts spaces to the next configured tab stop. If lines are selected,
`Tab` indents them without replacing the selection; `Shift+Tab` unindents the
current or selected lines as one edit.

`Enter` preserves the current line's indentation and adds one configured tab
level after common block openers. The global tab width defaults to four spaces,
and language-specific settings can override it by file extension.

## Finding, replacing, and going to a line

### Find

Press `Ctrl+F` to open incremental search. Matches update as you type.

- `Enter` or `Down` moves to the next match.
- `Up` moves to the previous match.
- Navigation wraps at the beginning or end.
- `Escape` closes search and clears its highlight.

In paged large files, an explicit search examines the whole logical file,
including unsaved edits on visited pages and matches that cross edited page
boundaries. The search runs on a cancellable worker rather than the typing path.

### Replace

`Ctrl+Shift+F` opens a two-stage Replace Next prompt. Enter the search text,
press `Enter`, enter the replacement, and press `Enter` again. The next match is
replaced as one undoable edit.

To replace every non-overlapping match, open the command prompt and run:

```text
replace-all
```

Replace All is available for fully loaded ordinary buffers and is one undoable
transaction. It refuses paged files instead of silently replacing only the
visible page.

The command `replace` opens the same Replace Next prompt as
`Ctrl+Shift+F`.

### Go to line

Press `Ctrl+G`, type a 1-based line number, and press `Enter`. You can also run
`goto LINE` or `line LINE` from the command prompt. A line past the end moves to
the last line and reports what happened. Locating a line in a paged file is
cancellable with `Escape`.

## Working with multiple buffers

Startup opens exactly one file buffer. Create or open additional buffers during
the editor session:

| Action | Shortcut | Command |
| --- | --- | --- |
| Open a path | `Ctrl+O` | `open PATH`, `edit PATH`, `e PATH` |
| New untitled buffer | `Ctrl+N` | `new` |
| Close active buffer | `Ctrl+W` | `close` |
| Discard and close a dirty buffer | — | `close!` |
| Next buffer | `Alt+PageDown` | — |
| Previous buffer | `Alt+PageUp` | — |

Opening an already-open file switches to or reports the existing buffer rather
than creating a duplicate. On Linux, Catomic follows symlinks and identifies an
existing regular file by device and inode, so relative/absolute spellings,
`.`/`..` aliases, symlinks, and hard links select the first buffer that opened
the file. That buffer keeps its original path spelling. A hard-linked file still
cannot be saved because the atomic-save safety policy refuses targets with more
than one link.

For a missing path, Catomic resolves the deepest existing parent directory and
then compares the remaining normalized path. It does not assume that different
nonexistent or dangling path components will later become the same file. If two
open missing paths later converge on one file, Save is blocked instead of
letting independent dirty buffers overwrite one another through watcher timing.

Each buffer retains its cursor, viewport, selection, dirty state, file watcher,
latest external-reload markers, whitespace/wrapping toggles, and large-file page
position. External-reload marker visibility and line numbers are session-global
preferences shared by every buffer.

`Ctrl+S` saves only the active buffer. `Ctrl+Q` checks every open buffer; if any
are dirty, the first press warns and the second press quits without saving.
`Ctrl+Shift+C` is deliberately different: it is the configurable `interrupt`
action, exits immediately with SIGINT status, and does not ask about unsaved
changes. Catomic still restores its terminal modes before exiting.

A dirty buffer refuses an ordinary close. Save it first, or use the explicit
`close!` command when discarding the changes is intentional.

## Saving and external changes

### Save and Save As

- `Ctrl+S` saves the active named buffer.
- `Ctrl+Shift+S` opens Save As.
- `save`, `write`, and `w` save from the command prompt.
- `save as PATH`, `save-as PATH`, and `saveas PATH` save to a new path.

Saving an untitled buffer automatically opens Save As. Paths may be relative,
absolute, or home-relative, such as `~/notes/today.md`.

If a Save As destination already exists, Catomic refuses the first submission
and asks you to submit the same path again. If the target changes between those
submissions, the confirmation is invalidated.

Save As never overwrites a path represented by another open buffer, including a
symlink or hard-link alias. Switch to that buffer or close it first; repeated
confirmation does not bypass this in-process collision guard.

### Atomic saves

Catomic writes a temporary sibling, verifies the resulting file state, and
atomically replaces the destination. On Linux it preserves the mode, owner, and
group of an existing regular file. Replacing the destination changes its inode;
software that relies on inode identity should account for that.

An atomic save removes a matching `.catnap` recovery sidecar after the source
save succeeds.

### External file changes

File watching is always enabled for watchable named files. Watch notifications are
hints; Catomic captures a fresh bounded disk identity before acting. Files up to
100 MiB use a streaming SHA-256 of the complete content in addition to size,
timestamps, and Unix device/inode/change time. This detects rapid same-length
rewrites even when every available metadata field collides.

- A clean buffer reloads automatically when the file changes or is deleted,
  unless `[files] auto_reload = false`.
- A dirty buffer is never discarded automatically.
- `Ctrl+R` explicitly checks for an external change or confirms a reload.

Every accepted reload compares the exact prior buffer revision with the exact
verified disk revision that is installed. Added graphemes use the semantic
`external_added` role and a `+` gutter marker; replacements use
`external_changed` and `~`; deleted text uses a non-document `-` marker on the
nearest surviving line. The markers never enter buffer bytes, selection,
search, clipboard, save output, dirty tracking, or undo history.

`F5` toggles this presentation and remembers the choice. Turning it off dismisses
the current markers in every buffer. A later reload replaces that buffer's
previous set. Any local content edit clears the active buffer's complete set;
Catomic deliberately does not guess how snapshot coordinates map through an
unrelated edit. Buffer switching alone preserves each set.

External diffing is bounded to old and new buffers of at most 10 MiB and 200,000
lines each. Larger, paged, or length-opaque buffers still reload safely but show
an explicit status message that highlighting was skipped. A line over 1 MiB is
marked as one changed line instead of building a grapheme index for it.

When a dirty buffer differs from disk, the first `Ctrl+R` arms a reload and the
second performs it only if the observed disk state is still the same. Editing
the buffer or another disk change invalidates the confirmation.

The save guard works similarly. If the file changed or disappeared after it was
opened or last saved, the first `Ctrl+S` refuses to overwrite it. A second
`Ctrl+S` forces the save only if the external state still matches the state you
were warned about. This makes the dangerous action explicit without trusting a
stale yes/no prompt.

Set automatic clean reload off when you want every external change to require
manual confirmation:

```toml
[files]
auto_reload = false
```

## Views, highlighting, and Markdown

Catomic applies lightweight, viewport-bounded syntax styling to Markdown,
Rust, Python, and JSON based on the file extension. It is deliberately lexical:
there is no tree-sitter parse or whole-document highlighting pass.
Markdown source styling keeps every delimiter visible and every document
coordinate unchanged while distinguishing headings, emphasis, links, inline
code, fences, quotes, list/task markers, and table delimiters. Unsupported or
malformed constructs remain ordinary readable text.

| View | Key | Behavior |
| --- | --- | --- |
| External changes | `F5` | Toggle latest external-reload marks for all buffers and remember the choice |
| Markdown preview | `F6` | Render the current buffer or active large-file page as Markdown |
| Line numbers | `F7` | Toggle line numbers for all buffers and remember the choice |
| Visible whitespace | `F8` | Show spaces and tabs |
| Soft wrapping | `F9` | Wrap at terminal width without inserting newlines |

`F6` is an explicit content command: it attempts Markdown rendering for
`README`, `.txt`, extensionless, untitled, and Markdown-named buffers alike.
The filename still chooses source highlighting defaults, but it never vetoes
preview. Press `F6` again or `Escape` to leave Markdown preview. Soft-wrapped
continuations preserve document coordinates and mouse mapping. Whitespace and
soft wrapping remain per-buffer settings. F5 and F7 update the session-global
preference used by every current buffer and buffers opened later.

The shared Markdown presentation layer reflows paragraphs, headings, nested
quotes and lists, tasks, links, footnotes, rules, and indented code blocks to
the terminal width. At 40 cells and above it reserves two side margins; on
wider terminals it centers an 88-cell maximum reading column. It
keeps parsed inline semantics through terminal presentation: strong, emphasis,
strikethrough, inline code, and links use terminal attributes without restoring
their source delimiters. H1 uses a title band, H2 uses a bold section style,
and H3–H6 use progressively quieter styles and deeper indentation, all with
defined block spacing and no generated rulers. Quoted paragraphs use quotation
marks and quote nesting uses depth indentation; thematic breaks use a compact
centered mark, inline code is
distinct from indented fenced blocks, and links use OSC 8 destinations without
dumping URLs into prose. These attributes and structural markers remain
readable when color is unavailable. Tables retain parsed alignment and
terminal-cell measurement (including wide characters, combining marks, and
emoji). They use aligned columns with internal separators when they fit and a
wrapped label/value layout when they do not.
An active preview is laid out again when the terminal width or line-number
gutter changes; this never reparses during ordinary editing.

Preview construction accepts at most 10 MiB of active source and 32 MiB of
rendered output. Individual table cells are capped at 40 terminal cells;
pathological table row, column, and text counts are refused with a render error.
No source bytes, path, dirty state, selection, history, cursor, viewport, or
line-ending format are changed. Raw HTML is displayed as inert text; terminal
control characters become visible safe glyphs rather than executing.

## Large files

Catomic classifies files by their on-disk byte size:

| Tier | Size | Open behavior |
| --- | --- | --- |
| Small | Up to 10 MiB | Fully loaded ordinary buffer |
| Large | Over 10 MiB through 100 MiB | Fully loaded, with a warning |
| Huge | Over 100 MiB through 1 GiB | Editable paged mode |
| Extreme | Over 1 GiB | Editable paged mode |

Paged mode avoids materializing the entire file as one in-memory string. The
default page contains 20,000 lines and can be changed with
`[big_files] page_lines`.

- `Ctrl+PageDown` opens the next file page.
- `Ctrl+PageUp` opens the previous file page.
- The status line shows the page number.
- Edits to visited pages remain available when you move away.
- Undo and redo follow edit order across pages.
- `Ctrl+S` streams the complete logical document to an atomic save, combining
  edited pages with untouched source ranges.
- `Ctrl+F` searches the complete logical document after explicit invocation.

Page boundaries stay anchored to the opened source during a session and are
rebuilt after reload or reopen. If the underlying descriptor drifts while a
paged operation is using it, Catomic fails closed rather than mixing revisions.

For Huge and Extreme paged files, external-change checks hash fixed 64 KiB
samples at the start, middle, and end (192 KiB total) in addition to filesystem
metadata. This keeps checks independent of total file size and catches common
whole-file and boundary rewrites. A same-inode, same-size rewrite confined
outside all three samples while also preserving every available timestamp is a
known best-effort case; Catomic cannot prove that revision changed without an
unbounded scan or an immutable filesystem snapshot.

Paged files support LF and CRLF endings. UTF-8-BOM and CR-only files must remain
at or below the 100 MiB paging threshold. Replace All, full-buffer external
command input, recovery sidecars, and whole-file model edits may refuse paged
files because those actions require a fully loaded buffer.

To trade more page transitions for less line metadata, reduce the page size:

```toml
[big_files]
page_lines = 10_000
```

The value must be a positive integer.

## Completion

Press `Ctrl+Space` or `Tab` to request completion.

- `Tab` and `Shift+Tab` cycle candidates.
- `Enter` accepts the active candidate.
- `Escape` dismisses the list.

Completion derives candidates only from a bounded window of the current buffer.
It does not scan directories, retain a project cache, or perform filesystem
work. If there is no completion candidate, `Tab` performs normal indentation.

Accepting a completion is one undoable replacement. Catomic does not enable
continuous ghost text or a background completion service.

Typing a colon-prefixed query of at least two characters at a token boundary
opens the inline emoji picker. For example, `:hun` ranks `💯 hundred points`
first. The table updates as the query changes; use `Up`/`Down`, `Tab`, or
`Shift+Tab` to select a row, `Enter` to replace only the active query, and
`Escape` to dismiss it without changing the text. Colons inside words, URLs,
times, and `::` sequences do not trigger the picker.

Emoji names and GitHub-style aliases come from a bundled deterministic table.
Matching and insertion are entirely local: typing never starts a process,
filesystem scan, or network request.

## Linting

Linting is an explicit editor action. Configure a command for the file
extension, save the active buffer, then press `F4` (the remappable `lint`
action).

Language-specific configuration is preferred:

```toml
[languages.rs]
tab_size = 4

[languages.py]
tab_size = 4
linter = "ruff check {file}"

[languages.js]
tab_size = 2
linter = "eslint {file}"
```

Every linter command must contain `{file}`. Catomic shell-quotes and substitutes
the absolute active path, then runs the command from that file's parent
directory without blocking typing. `Escape` cancels a running linter.
The command must accept that single file path. Project-wide tools such as Cargo
need an explicit wrapper that accepts `{file}` and emits findings for that file
as either absolute paths or paths relative to the file's parent directory.

The parser accepts common `file:line:column: message`-shaped output for the
active file. Findings appear directly as lightweight underlined marks. Move the
cursor to a marked line to see the raw linter message in the status row. A rerun
replaces all prior marks. Editing, reloading, renaming, or switching buffers
immediately cancels and invalidates stale results; a late result from an older
revision is discarded. Catomic does not infer severity from message wording or
open a separate findings picker. If a command fails without parseable findings,
Catomic reports the exit state instead of inventing a clean result. Output that
exceeds the bounded capture limit is rejected without installing partial
findings.

The older extension table remains supported, but a `[languages.EXT]` linter
wins when both are present:

```toml
[linters]
py = "ruff check {file}"
```

## External commands and hooks

Named commands are trusted local configuration. Catomic limits their input,
output, and runtime, but the command itself runs through `/bin/sh -c` and can
have arbitrary effects outside the editor.

### Named commands

Configure commands by name:

```toml
[commands.upper]
command = "tr '[:lower:]' '[:upper:]'"
input = "selection"
output = "replace-input"
timeout_secs = 10

[commands.date]
command = "date +%F"
input = "none"
output = "insert"
```

Run one from the command prompt:

```text
run upper
```

`input` accepts:

- `none` (default): send no stdin;
- `selection`: send the active selection; or
- `buffer`: send the complete ordinary buffer.

`output` accepts:

- `preview` (default): show the result without editing;
- `insert`: insert confirmed stdout at the captured cursor; or
- `replace-input`: replace the selected or full-buffer input after confirmation.

`replace-input` requires `selection` or `buffer` input. Buffer input and command
edits require a fully loaded ordinary buffer. A command that contains `{file}`
requires a saved active path; the placeholder becomes the shell-quoted absolute
path, and the command runs from that file's directory.

Commands run asynchronously. Input is capped at 16 MiB, stdout and stderr at
1 MiB each, and configured timeouts must be 1–300 seconds. `Escape` cancels a
running command. Completed output opens read-only; only successful, complete,
non-stale output can be applied with `Enter`, and it becomes one undoable edit.
Failed or truncated output cannot apply.

### Lifecycle hooks

The same named commands can run in order on lifecycle events:

```toml
[commands.inspect]
command = "file {file}"

[commands.check]
command = "printf 'saved: %s\n' {file}"

[commands.redact-check]
command = "./scripts/check-secrets {file}"

[hooks]
on_open = ["inspect"]
on_save = ["check"]
before_llm = ["redact-check"]
```

Hook arrays may reference only configured names and may not contain duplicates.
Commands run sequentially. A failure, timeout, cancellation, stale target, or
rejected preview stops the remainder of the chain.

`on_save` begins only after a successful atomic save. If you confirm a hook edit,
the buffer becomes dirty again. `before_llm` completes before Catomic prepares
the endpoint/context confirmation, so there is no model client or request while
the hook is running.

## Model-assisted commands

Model support is explicit, transient, and preview-first. A named preset can use
an OpenAI-compatible Chat Completions endpoint or a headless command adapter.
Catomic does not construct an HTTP client, read a credential value, or start a
command until you invoke a model action and confirm its destination and context.
Credentials are resolved only when a confirmed request is about to start.

### Presets and HTTP configuration

The no-config default remains the original local endpoint. Existing single
`[llm]` configuration continues to work and becomes one implicit `local`
preset. New configurations can name several presets:

```toml
[llm]
default = "local"

[[llm.backends]]
name = "local"
type = "openai-compatible"
base_url = "http://127.0.0.1:8080/v1"
model = "local-model"
models = ["local-model-small", "local-model-large"]

[[llm.backends]]
name = "openrouter"
type = "openai-compatible"
base_url = "https://openrouter.ai/api/v1"
model = "provider/model-id"
api_key_env = "OPENROUTER_API_KEY"
headers = { "HTTP-Referer" = "https://example.invalid/catomic" }
header_envs = { "X-Provider-Key" = "PROVIDER_SECONDARY_KEY" }
timeout_secs = 120
```

The base URL must be plain HTTP or HTTPS without embedded credentials,
whitespace, query, or fragment. Timeouts must be 1–600 seconds.

Loopback HTTP may use credentials. Unauthenticated LAN HTTP is also allowed for
local models. Catomic refuses to send an API key or credential header to a
non-loopback plaintext HTTP endpoint; use HTTPS for an authenticated remote
endpoint.

The client refuses redirects and ignores ambient proxy variables so context
cannot silently leave through a destination other than the one you confirmed.
Static non-secret metadata `headers` and credential `header_envs` are explicit
per preset. Credential-looking static headers are rejected. Picker and status
text never show header values. An explicit `api_key_env` or
`header_envs` variable must be present when that preset is invoked. The implicit
legacy preset preserves the prior optional-key behavior for local servers.
Static and environment-sourced HTTP header values are capped at 8192 bytes and
must be valid HTTP header values.

The `models` array adds static model choices for the same HTTP destination. Set
`discovery = true` to permit model-list discovery, but this does not make it
automatic: select that preset in the picker, press `Ctrl+D`, inspect the shown
`BASE_URL/models` destination, then press `Enter`. Discovery sends no document
content, is cancellable, refuses redirects/proxies, accepts at most 256 KiB and
128 validated model identifiers, uses at most a ten-second request timeout, and
keeps them in memory for five minutes.
Discovered choices never become executable configuration and are never written
to disk.

### Headless command presets

A command preset is an argv adapter, not a shell string:

```toml
[[llm.backends]]
name = "local-headless"
type = "command"
program = "/usr/local/bin/my-headless-model"
args = ["--structured-output"]
model = "friendly-model-id"
input = "stdin-text-v1"
output = "claude-json-v1"
timeout_secs = 120
```

`program` must be an absolute path or a bare name resolved through absolute
`PATH` entries. Arguments are passed exactly, including spaces and Unicode;
Catomic adds no implicit `/bin/sh -c`. `stdin-text-v1` writes a transcript beginning
with `Catomic model request v1`, followed by `[system]`, `[user]`, and (for repo
broker rounds) `[assistant]` sections.

Two output contracts are supported:

- `claude-json-v1`: one successful JSON result object with `type = "result"`,
  `is_error = false`, and a non-empty string `result`;
- `codex-jsonl-v1`: JSONL lifecycle events ending in `turn.completed`, with one
  or more `item.completed` agent-message items. Reasoning items are ignored;
  tool, command, file-change, or unknown items fail closed.

Vendor CLI flags and event schemas change. Catomic deliberately does not append
illustrative Claude, Codex, Grok, or other flags. Configure and test the
non-interactive text/proposal mode for the exact installed version. The child
runs with a private temporary working directory, bounded stdin/stdout/stderr and
runtime, and a dedicated process group that is killed while its direct child is
reaped on cancellation.
Stderr is suppressed from errors. A configured executable is still trusted user
code with your OS permissions and inherited authentication environment after
confirmation; Catomic is not an OS sandbox. Never configure an agent/tool mode
that can mutate the workspace. Repository-local command presets are not loaded.
For command requests, the prompt names only the active file's basename (or the
confirmed repository-relative path), never the workspace's absolute path.

### Selecting the active model

Press `F10`, run `model`/`models`, or bind any normal-mode chord to the canonical
`select-model` action. Type to filter by preset, model, adapter, destination, or
availability. Use arrows/PageUp/PageDown and `Enter`; `Escape` cancels. Each row
starts with `A` for the effective active choice, `S` for an explicit session
override, and `D` for the configured default. The row also shows local/remote,
the canonical URL or resolved executable, and availability.

Selection changes subsequent `meow`, `bigmeow`, `gitmeow`, and `megameow`
requests for every buffer in this Catomic process. It never invokes the backend,
reads credential values, or rewrites configuration. To persist another default,
edit `llm.default` as a separate explicit configuration action. Reopening or
filtering the picker and switching buffers never persists anything.

### One-key inline clanker

Press `F3`, or run `run-clanker` / `inline-meow`, to use an instruction written
in the active document. The default one-line form is:

```text
>> Refactor this code without changing its behavior.
```

The context and editable target precedence is:

```text
selection → catblocks → bounded full file
```

An active selection is sent alone and only its exact captured bytes may be
replaced. With no selection, one or more blocks expose only their interiors:

```text
>> Rename the concept consistently.

<catblock>
first relevant section
</catblock>

private text that is not sent

<catblock>
second relevant section
</catblock>
```

Delimiter lines remain byte-identical. Nested, overlapping, mismatched, empty,
or unclosed blocks fail closed with line-numbered errors. Combined mode sends
numbered, independently bounded blocks in one request and previews/applies every
replacement atomically. Queued mode sends exactly one block at a time, waits for
that block's review, applies accepted blocks as separate undo steps, and
revalidates the next captured block before sending it. `Escape` cancels the
active request and clears the remaining queue. Errors stop the queue by default.

With no selection or catblock, F3 uses the fully retained current file. Above
`warn_lines` (500 by default), the F2 prompt asks for a typed one-time `yes` or
`no` before the normal endpoint/context confirmation. Neither answer can bypass
the absolute 2,000-line / 64-KiB hard context limit. Paged files are refused.

The instruction chosen is the unique nearest marker at or before the cursor (or
selection start), and its line and exact text appear in confirmation. A marker
matches when the trimmed line starts with `instruction_prefix` followed by
whitespace; a configured suffix must then close the line. The older
`>>> catomic ... <<<` instruction block remains supported. Customize syntax
globally or by normalized file extension:

```toml
[llm.inline]
instruction_prefix = ">>"
instruction_suffix = ""
context_open = "<catblock>"
context_close = "</catblock>"
warn_lines = 500
block_mode = "combined" # or "queued"
queue_limit = 16
stop_on_error = true
remove_instruction_after_apply = true

[languages.rs.llm.inline]
instruction_prefix = "// >>"
context_open = "// <catblock>"
context_close = "// </catblock>"

[languages.html.llm.inline]
instruction_prefix = "<!-- >>"
instruction_suffix = "-->"
```

Required marker strings are non-empty, no more than 64 bytes, free of control
characters and outer whitespace, and may not overlap one another. The optional
instruction suffix has the same bounds. `warn_lines` is 1–2,000;
`queue_limit` is 1–64.

Successful apply removes only the confirmed instruction by default. Cleanup is
shown in the proposal and belongs to the accepted edit transaction: one undo
restores both content and instruction in combined mode. Cleanup removes the
instruction's terminating newline when present; for a final unterminated marker
it removes the preceding newline, preserving the document's line-ending style
and final-newline state. Failed, cancelled, rejected, stale, or partial queued
runs keep the instruction byte-identical.

After apply, inserted/replaced graphemes use the semantic `llm_changed` role
(red and underlined by default); changed/deleted lines have a red gutter mark.
These marks mean locally applied model output, not selection or an error, and
never become file content. They survive scrolling, wrapping, resize, undo/redo,
tabs, and Unicode layout. `Shift+F3` or `clear-clanker-changes` dismisses them
without editing the document. The next clanker apply, buffer close, or an
ordinary edit that invalidates a marked range also clears them. With `NO_COLOR`
set, reverse/underline and gutter markers provide the non-color fallback.

### Current-file commands

These commands are available directly:

- `meow INSTRUCTION` sends the active selection.
- `bigmeow INSTRUCTION` sends the current file.

If `meow` has no selection, place the cursor inside an instruction block. Its
text supplies both the instruction and bounded context:

```text
>>> catomic
Refactor this function.
Keep behavior identical.
Do not edit outside this block unless necessary.
<<<
```

With no `bigmeow` argument, an instruction block under the cursor supplies the
instruction. Every instruction follows the same proposed-edit flow; words such
as `explain`, `ask`, or `edit` have no special command meaning.

Before sending, Catomic shows the active preset, adapter, canonical endpoint or
resolved executable, model, exact context extent, and warnings for a dotfile
path or obvious secret-like lines. `Enter` confirms the request; `Escape`
cancels without constructing the client or starting the command.

Context is capped at 64 KiB and 2,000 lines. Oversized context fails closed
rather than being silently truncated. A proposed edit must be either:

- a validated single-file unified patch whose headers name the confirmed active
  path; or
- for a selected region, a strict `catomic_replacement` JSON envelope.

The result opens in a read-only preview. A second `Enter` applies it as one
undoable buffer change. The model command itself never saves the file. If the
active path or source revision changes during the request or before apply, the
proposal is discarded or refused.

### Repository-aware commands

`gitmeow INSTRUCTION` asks about a focused task with at most 64 KiB of
repository-broker context. `megameow INSTRUCTION` asks with a broader, still
bounded 128 KiB repository budget. The active-file context remains separately
capped at 64 KiB for either command. Both commands require a saved active file
inside a Git repository and a stable repository and active-file snapshot
throughout the request. Catomic detects and validates the Git root for each
invocation; it does not retain a workspace or repository session between
requests.

After invocation, Catomic captures bounded Git state and a bounded file map on a
cancellable worker, then presents a separate send confirmation. The model can
make at most eight read-only broker requests to list files, read a bounded range,
grep, or show a file diff. Initial and subsequently retrieved repository context
share the selected command's 64 or 128 KiB total budget.

The broker omits dot paths from its map, refuses path escapes, symlinks, unknown
or oversized files, and obvious secret-like direct reads. Grep skips sensitive
files and reports how many were omitted. Git capture disables pagers, fsmonitor,
external diff, and textconv helpers and strips inherited `GIT_*` variables that
could redirect repository identity.

HEAD, branch, status, tracked diff, the active file, and retrieved files are
checked for drift after the response and again before preview apply. Any drift
fails closed. Repository commands still accept only a single-file edit to the
confirmed active file. Multi-file apply and `feralmeow` are not implemented.

For the complete security contract, read [LLM rules](llm-rules.md).

## Crash recovery

Recovery is disabled by default. Enable it explicitly:

```toml
[recovery]
enabled = true
interval_secs = 30
max_bytes = 1_048_576
```

When enabled, a dirty named ordinary buffer no larger than `max_bytes` gets an
atomic, owner-only sibling sidecar such as `notes.txt.catnap` after the configured
interval. Untitled, oversized, and paged buffers are skipped. The interval must
be 5–3,600 seconds and the size cap 1–16 MiB.

On a later open, a newer valid sidecar produces a notice. Run `recover` to open
it read-only. Press `Enter` to apply the recovered text as one undoable buffer
edit, or `Escape` to leave the source untouched. Source drift invalidates the
preview, and recovery never replaces the source file automatically.

A successful normal save removes the sidecar. Recovery is a crash aid, not a
replacement for explicit saves, backups, or version control.

## Configuration reference

Catomic reads one TOML file from:

1. `$XDG_CONFIG_HOME/catomic/config.toml` when `XDG_CONFIG_HOME` is an absolute
   path; otherwise
2. `~/.config/catomic/config.toml` when `HOME` is an absolute path.

The source installer creates this file from Catomic's documented template and
never replaces an existing path. Unknown keys are ignored for forward
compatibility, but a malformed recognized value is a startup error for settings
loaded at startup. Linter and model settings are loaded lazily when invoked.

Run `catomic config` from the shell, or `config` from the in-editor command
prompt, to open that exact path as an ordinary editable buffer inside Catomic.
`catomic config edit` is an alias for the same Catomic-native action and does not
use `$VISUAL`, `$EDITOR`, `/bin/sh`, or another editor. If the file is later
missing, Catomic asks inside the live editor before atomically recreating an
owner-only file from the same documented, commented template. A `catomic`
directory created for that file uses mode `0700`. An existing user-owned,
non-symlink directory is accepted without changing its mode when group and
others cannot write to it; writable or differently owned directories are
refused with a permission error. Catomic never overwrites a file that appears
during confirmation. Configuration is validated and applied as one document at
normal startup, so restart Catomic after saving; the running session does not
silently apply a partial reload. The dedicated config action uses built-in
defaults, so an invalid config can still be opened and repaired. When the
in-editor command opens config from another buffer, `Ctrl+Q` closes that
temporary config buffer and returns to the invoking buffer. Unsaved config
changes are never discarded on the first press; a second `Ctrl+Q` explicitly
discards only the config detour. A config opened directly from the shell keeps
the editor's normal session-quit behavior.

Shell workflows can discover, validate, or edit the same path without guessing
the XDG resolution:

```sh
catomic config
catomic config path
catomic config check
catomic config edit
catomic config refresh-keybindings
```

`config check` is read-only. `config` and `config edit` are equivalent; both
enter Catomic and ask before recreating a missing file. If terminal setup fails,
no missing configuration is created.

`config refresh-keybindings` asks before adding or replacing Catomic's clearly
marked, commented action/default inventory. Active mappings (including `[]`
unbindings), unrelated settings, and comments outside the generated block are
left alone. Repeating the command when the inventory is current changes no
bytes. A missing config receives the same private template used by installation
and in-editor creation.

The configured persistent presentation defaults are:

```toml
[view]
external_diff = true
line_numbers = false
```

After an explicit F5 or F7 press, Catomic atomically writes both current values to
`$XDG_STATE_HOME/catomic/preferences.toml`, or
`~/.local/state/catomic/preferences.toml` when `HOME` is absolute. It never
rewrites `config.toml`, and merely starting Catomic does not create the state
file. Each key resolves independently: its saved value, then its `[view]` value,
then the built-in default (`true` for `external_diff`, `false` for
`line_numbers`). Remove `preferences.toml` to return control to `config.toml`.

Running instances do not live-reload each other's view state. Each keeps its
current session choice; atomic replacement prevents partial TOML, and the last
completed rename determines the default read by the next launch.

### Complete example

```toml
[editor]
tab_size = 4

[big_files]
page_lines = 20_000

[files]
auto_reload = true

[view]
external_diff = true
line_numbers = false

[cat]
status_messages = true

[recovery]
enabled = false
interval_secs = 30
max_bytes = 1_048_576

[theme]
name = "default"

[theme.colors]
text = "default"
background = "default"
cursor = "default"
selection = { fg = "black", bg = "cyan" }
line_number = "bright-black"
status = "default"
status_filename = { fg = "default", bold = true, underline = true }
message = { fg = "black", bg = "white" }
status_warning = { fg = "black", bg = "yellow" }
error = { fg = "bright-white", bg = "red" }
markdown_heading = "bright-blue"
markdown_emphasis = "magenta"
markdown_code = "green"
markdown_marker = "bright-cyan"
syntax_keyword = "magenta"
syntax_string = "green"
syntax_comment = "bright-black"
syntax_number = "yellow"
search_match = { fg = "black", bg = "yellow" }
diff_added = "green"
diff_removed = "red"
external_added = { fg = "green", underline = true }
external_changed = { fg = "cyan", underline = true }
external_deleted = { fg = "red", bold = true }
lint = { fg = "red", underline = true }
llm_changed = { fg = "red", underline = true }
preview = "default"

[languages.rs]
tab_size = 4

[languages.py]
tab_size = 4
linter = "ruff check {file}"

[keybindings]
save-as = ["alt+s"]
search = ["alt+f"]

[commands.upper]
command = "tr '[:lower:]' '[:upper:]'"
input = "selection"
output = "replace-input"
timeout_secs = 10

[hooks]
on_open = []
on_save = []
before_llm = []

[llm]
default = "local"

[[llm.backends]]
name = "local"
type = "openai-compatible"
base_url = "http://127.0.0.1:8080/v1"
model = "local-model"
models = ["local-model-small"]
timeout_secs = 120

[[llm.backends]]
name = "headless"
type = "command"
program = "/usr/local/bin/my-headless-model"
args = ["--structured-output"]
model = "headless-model"
input = "stdin-text-v1"
output = "claude-json-v1"
timeout_secs = 120

[llm.inline]
instruction_prefix = ">>"
instruction_suffix = ""
context_open = "<catblock>"
context_close = "</catblock>"
warn_lines = 500
block_mode = "combined"
queue_limit = 16
stop_on_error = true
remove_instruction_after_apply = true
```

### Setting limits and defaults

| Setting | Default | Valid values |
| --- | --- | --- |
| `editor.tab_size` | `4` | Integer `1`–`16` |
| `languages.EXT.tab_size` | global value | Integer `1`–`16` |
| `languages.EXT.linter` | none | String containing `{file}` |
| `big_files.page_lines` | `20000` | Positive integer |
| `files.auto_reload` | `true` | Boolean |
| `view.external_diff` | `true` | Boolean; overridden by the saved F5 choice |
| `view.line_numbers` | `false` | Boolean; overridden by the saved F7 choice |
| `cat.status_messages` | `true` | Boolean |
| `mobile.action_bar` | `auto` | `auto`, `always`, or `never` |
| `recovery.enabled` | `false` | Boolean |
| `recovery.interval_secs` | `30` | Integer `5`–`3600` |
| `recovery.max_bytes` | `1048576` | Integer `1`–`16777216` |
| `theme.name` | `default` | `default`, `high-contrast`, or `mono` |
| `commands.NAME.input` | `none` | `none`, `selection`, `buffer` |
| `commands.NAME.output` | `preview` | `preview`, `insert`, `replace-input` |
| `commands.NAME.timeout_secs` | `10` | Integer `1`–`300` |
| `llm.default` | first backend / implicit `local` | Existing backend name |
| `llm.backends[].name` | required | Unique printable name, 1–64 characters |
| `llm.backends[].type` | required | `openai-compatible` or `command` |
| HTTP `base_url`, `model` | required | Canonical HTTP(S) URL; printable model ID |
| HTTP `models` | empty | At most 128 printable static model IDs |
| HTTP `api_key_env`, `header_envs` | none | Valid environment-variable names |
| HTTP `headers` | empty | Explicit non-secret metadata; 32 total headers max |
| HTTP `discovery` | `false` | Boolean; still requires picker confirmation |
| Command `program`, `args` | required / empty | Absolute or bare executable; at most 64 bounded args |
| Command `input` | `stdin-text-v1` | Versioned stdin transcript contract |
| Command `output` | required | `claude-json-v1` or `codex-jsonl-v1` |
| Backend `enabled` | `true` | Boolean; disabled presets remain visible but cannot select |
| Backend `timeout_secs` | `120` | Integer `1`–`600` |

Legacy `llm.base_url`, `llm.model`, `llm.api_key_env`, and `llm.timeout_secs`
remain valid only when `llm.backends` is absent. Mixing the two shapes is an
error rather than an ambiguous partial migration.
| `llm.inline.instruction_prefix` | `>>` | Non-empty unambiguous marker, at most 64 bytes |
| `llm.inline.instruction_suffix` | empty | Empty or bounded suffix, at most 64 bytes |
| `llm.inline.context_open` | `<catblock>` | Non-empty unambiguous marker, at most 64 bytes |
| `llm.inline.context_close` | `</catblock>` | Non-empty unambiguous marker, at most 64 bytes |
| `llm.inline.warn_lines` | `500` | Integer `1`–`2000` |
| `llm.inline.block_mode` | `combined` | `combined`, `queued` |
| `llm.inline.queue_limit` | `16` | Integer `1`–`64` |
| `llm.inline.stop_on_error` | `true` | Boolean |
| `llm.inline.remove_instruction_after_apply` | `true` | Boolean |

Language extension names are case-normalized and may be written with or without
a leading dot. Command names may contain ASCII letters, digits, `-`, and `_`.

### Color schemes

Themes use semantic roles, so rendering does not hard-code colors by syntax or
surface. The complete role inventory is `text`, `background`, `cursor`,
`selection`, `line_number`, `status`, `status_filename`, `message`,
`status_warning`, `status_prompt`, `error`,
`markdown_heading`, `markdown_emphasis`, `markdown_code`, `markdown_marker`,
`markdown_link`, `syntax_keyword`, `syntax_string`, `syntax_comment`, `syntax_number`,
`search_match`, `diff_added`, `diff_removed`, `external_added`,
`external_changed`, `external_deleted`, `lint`, `llm_changed`, and `preview`.
External-reload and model-change roles remain independent. The syntax roles
apply consistently to the built-in Rust, Python, and JSON highlighters.

A role may be `"default"`, one of the standard 16 names (`black` through
`white` and `bright-black` through `bright-white`), an integer from 0 to 255,
`"index:N"`, `"#RRGGBB"`, or `"rgb(R,G,B)"`. Roles also accept a table such as
`{ fg = "black", bg = "cyan", bold = true, dim = false, underline = false,
reverse = false }`. RGB is emitted as
truecolor only when the terminal advertises `COLORTERM=truecolor` or `24bit`;
otherwise Catomic selects a stable xterm-256 fallback.

`NO_COLOR`, missing or monochrome terminal types, and `TERM=dumb` suppress color
while retaining bold, underline, and inverse-video distinctions for selections
and search matches.

The built-in `default` scheme preserves terminal-default text/background while
keeping selection, search, warnings, and errors distinguishable. Use
`high-contrast` for explicit black/bright-white base colors or `mono` to remove
syntax hues. Inline roles override the named scheme. `background` has explicit
precedence over `text.bg`. Invalid recognized colors fail the whole startup or
`config check`; no subset is applied. Every styled segment is reset, and exit
restores SGR attributes and the terminal's cursor color.

The persistent footer uses the terminal's own default foreground/background
pair, so its contrast follows the selected terminal theme instead of assuming a
dark background. The basename is bold and underlined rather than assigned a
fixed hue. At four or more rows, a cleared row separates document text from the
footer; smaller terminals drop the separator first, then the footer, so the
editing area never becomes zero-height. Transient messages keep their semantic
full-row styles.

### Custom keybindings and action registry

The recommended action-oriented form replaces an action's complete default
chord list. Use an empty array to unbind every default for that action:

```toml
[keybindings]
save = ["ctrl+s", "alt+s"]
help = []
prompt-cancel = ["alt+x"]
mouse-select-word = ["mouse-left-double"]
select-model = ["alt+m"]
```

The Phase 7 chord-oriented form such as `"alt+s" = "save"` remains accepted as
an explicit compatibility override and may replace a built-in chord even across
global/local scope precedence. Action-oriented entries are preferred
because replacement and unbinding are explicit.

Chord modifiers are `ctrl`/`control`, `alt`, and `shift`. Keys may be one
character, `space`, `tab`, `enter`, `esc`, `backspace`, `delete`, `insert`, an
arrow key, `pageup`, `pagedown`, `home`, `end`, or `f1` through `f12`. Mouse
gestures are `mouse-left`, `mouse-left-drag`, `mouse-left-up`,
`mouse-left-double`, `mouse-wheel-up`, and `mouse-wheel-down`. Button actions
cannot be assigned wheel gestures (or vice versa). Catomic rejects configurable
unmodified or Shift-only printable keys so a remap cannot silently capture ordinary typing.

Global actions have first precedence, followed by the active local surface,
then editor typing. A chord may therefore have a different local meaning in a
prompt, search, completion, preview, picker, or help surface. Two actions in the
same effective scope cannot share a chord; global bindings overlap every local
scope and therefore cannot shadow a local action. `config check` reports both action
names, both input chords, the scope, and the normalized collision. `Ctrl+Space`
and terminals that report it as Ctrl+Null normalize to the same chord, as do
`Shift+Tab` and BackTab, modifier aliases, case, and `esc`/`escape`.

This guide preserves the complete default registry for configuration lookup.
The built-in help uses the same registry and the loaded keybinding map, but
shows only a curated set of high-value workflows. This inventory is checked
against the registry in tests:

<!-- action-registry-start -->

```text
help | global | ctrl+h, f1
quit | global | ctrl+q
interrupt | global | ctrl+shift+c
save | editor | ctrl+s
save-as | editor | ctrl+shift+s
open | editor | ctrl+o
new | editor | ctrl+n
close | editor | ctrl+w
reload | editor | ctrl+r
lint | editor | f4
search | editor,help | ctrl+f
replace | editor | ctrl+shift+f
goto-line | editor | ctrl+g
command-prompt | editor | ctrl+shift+p, f2
complete | editor | ctrl+space
undo | editor | ctrl+z
redo | editor | ctrl+y, ctrl+shift+z
move-left | editor,preview,picker,help | left
move-right | editor,preview,picker,help | right
move-up | editor,preview,picker,help | up
move-down | editor,preview,picker,help | down
select-left | editor | shift+left
select-right | editor | shift+right
select-up | editor | shift+up
select-down | editor | shift+down
line-start | editor,preview,picker,help | home
line-end | editor,preview,picker,help | end
select-line-start | editor | shift+home
select-line-end | editor | shift+end
document-start | editor | ctrl+home
document-end | editor | ctrl+end
select-document-start | editor | ctrl+shift+home
select-document-end | editor | ctrl+shift+end
viewport-up | editor,preview,picker,help | pageup
viewport-down | editor,preview,picker,help | pagedown
select-viewport-up | editor | shift+pageup
select-viewport-down | editor | shift+pagedown
word-left | editor | ctrl+left
word-right | editor | ctrl+right
select-word-left | editor | ctrl+shift+left, alt+shift+left
select-word-right | editor | ctrl+shift+right, alt+shift+right
paragraph-previous | editor | ctrl+up
paragraph-next | editor | ctrl+down
delete-backward | editor | backspace
delete-forward | editor | delete
delete-word-backward | editor | ctrl+backspace
delete-word-forward | editor | ctrl+delete
insert-newline | editor | enter
indent | editor | tab
unindent | editor | shift+tab
toggle-overwrite | editor | insert
select-all | editor | ctrl+a
copy | editor | ctrl+c
cut | editor | ctrl+x
cut-line | editor | ctrl+k
paste | editor | ctrl+v
previous-buffer | editor | alt+pageup
next-buffer | editor | alt+pagedown
previous-page | editor | ctrl+pageup
next-page | editor | ctrl+pagedown
toggle-external-diff | editor,preview | f5
markdown-preview | editor,preview | f6
line-numbers | editor,preview | f7
whitespace | editor,preview | f8
soft-wrap | editor,preview | f9
run-clanker | editor | f3
clear-clanker-changes | editor | shift+f3
select-model | editor | f10
prompt-submit | prompt | enter
prompt-cancel | prompt | esc
prompt-delete-backward | prompt,search | backspace
search-next | search | enter, down
search-previous | search | up
search-cancel | search | esc
completion-next | completion | tab, ctrl+space
completion-previous | completion | shift+tab
completion-accept | completion | enter
completion-cancel | completion | esc
preview-accept | preview | enter
preview-cancel | preview | esc
picker-accept | picker | enter
picker-cancel | picker | esc
help-close | help | esc
mouse-place-cursor | editor | mouse-left
mouse-extend-selection | editor | mouse-left-drag
mouse-finish-selection | editor | mouse-left-up
mouse-select-word | editor | mouse-left-double
mouse-scroll-up | editor,preview,picker,help | mouse-wheel-up
mouse-scroll-down | editor,preview,picker,help | mouse-wheel-down
```

<!-- action-registry-end -->

## Shortcut reference

| Category | Action | Shortcut |
| --- | --- | --- |
| App | Help | `Ctrl+H` or `F1` |
| App | Quit; press twice to discard dirty buffers | `Ctrl+Q` |
| App | Interrupt immediately through SIGINT teardown | `Ctrl+Shift+C` |
| Files | Save | `Ctrl+S` |
| Files | Save As | `Ctrl+Shift+S` |
| Files | Open | `Ctrl+O` |
| Files | New buffer | `Ctrl+N` |
| Files | Close clean buffer | `Ctrl+W` |
| Files | Check/reload external change | `Ctrl+R` |
| Buffers | Previous / next buffer | `Alt+PageUp` / `Alt+PageDown` |
| Editing | Select/copy/cut/paste | `Ctrl+A` / `Ctrl+C` / `Ctrl+X` / `Ctrl+V` |
| Editing | Cut current line; repeated cuts append | `Ctrl+K` |
| Editing | Undo | `Ctrl+Z` |
| Editing | Redo | `Ctrl+Y` / `Ctrl+Shift+Z` |
| Editing | Indent / unindent | `Tab` / `Shift+Tab` |
| Editing | Delete previous / next word | `Ctrl+Backspace` / `Ctrl+Delete` |
| Navigation | Move by word | `Ctrl+Left` / `Ctrl+Right` |
| Navigation | Previous / next paragraph | `Ctrl+Up` / `Ctrl+Down` |
| Navigation | Start / end of document | `Ctrl+Home` / `Ctrl+End` |
| Editing | Toggle insert/overwrite mode | `Insert` |
| Search | Find | `Ctrl+F` |
| Search | Replace next | `Ctrl+Shift+F` |
| Search | Go to line | `Ctrl+G` |
| Tools | Command prompt | `Ctrl+Shift+P` or `F2` |
| Tools | Completion | `Ctrl+Space` |
| Tools | Inline clanker | `F3` |
| Tools | Clear clanker change marks | `Shift+F3` |
| Tools | Lint saved active file | `F4` |
| View | External-reload change marks | `F5` |
| View | Markdown preview | `F6` |
| View | Line numbers | `F7` |
| View | Visible whitespace | `F8` |
| View | Soft wrapping | `F9` |
| Tools | Select model/backend for this session | `F10` |
| Large files | Previous / next page | `Ctrl+PageUp` / `Ctrl+PageDown` |

This table shows built-in defaults. The in-editor `Ctrl+H`/`F1` reference is a
shorter task guide whose displayed chords reflect the effective configured
bindings; unbound actions are described without advertising a dead shortcut.
On Android/Termux, tap **Menu** in the action row instead; its scrollable palette
exposes essential actions normally reached through modifiers, function keys, or
page keys, including the inline clanker and model/provider selector.

## Command reference

Open the prompt with `Ctrl+Shift+P` or `F2`. Do not add a leading colon.

| Command | Aliases | Purpose / requirement |
| --- | --- | --- |
| `help` | `shortcuts` | Open built-in help |
| `config` | — | Open the resolved user config; confirm before first creation |
| `save` | `write`, `w` | Save active buffer |
| `save as PATH` | `save-as PATH`, `saveas PATH` | Save to a new path |
| `open PATH` | `edit PATH`, `e PATH` | Open path in a buffer |
| `new` | — | Create an untitled buffer |
| `close` | — | Close active clean buffer |
| `close!` | — | Discard and close active dirty buffer |
| `goto LINE` | `line LINE` | Go to a 1-based line |
| `replace` | — | Replace next match |
| `replace-all` | `replaceall` | Replace all in an ordinary buffer |
| `run NAME` | — | Run a configured trusted command |
| `recover` | — | Preview a newer `.catnap` sidecar |
| `model` | `models`, `select-model` | Search/select a process-local model preset |
| `run-clanker` | `inline-meow` | Run document instruction with automatic bounded scope |
| `clear-clanker-changes` | — | Dismiss applied-model marks without editing text |
| `meow TEXT` | — | Send selection/instruction block to configured model |
| `bigmeow TEXT` | — | Send current ordinary file to configured model |
| `gitmeow TEXT` | — | Detect Git and use focused bounded repository context |
| `megameow TEXT` | — | Detect Git and use broader bounded repository context |
| `quit` | `q` | Use the normal guarded quit path |

## File formats and save safety

### Text encoding and line endings

Catomic accepts valid UTF-8 text. Ordinary buffers preserve:

- an optional UTF-8 byte-order mark;
- LF line endings;
- CRLF line endings; or
- CR-only line endings.

UTF-16, arbitrary binary data, and non-UTF-8 filenames are refused rather than
decoded or rewritten heuristically.

Paged files support LF and CRLF only. A BOM-prefixed or CR-only file above the
paging threshold is refused.

### Target restrictions

Opening and saving are deliberately conservative:

- ordinary file targets are supported;
- directories, FIFOs, sockets, devices, and other non-regular targets are
  refused;
- a symlink to a regular file can be edited, and save replaces its referent
  while leaving the final symlink intact;
- a dangling final symlink is refused;
- Save As refuses symlinks that resolve to unsupported target types;
- files with more than one hard link are staged, then updated in place so every
  alias retains the shared inode and its mode, ownership, attributes, and ACLs;
  and
- extended attributes and POSIX ACLs are copied to single-link replacements and
  reapplied to multiply-linked shared inodes, then verified before success.

On Linux, saving must preserve mode, owner, group, extended attributes, and
POSIX ACLs. If the filesystem, mount, container, or network share cannot provide
the required metadata behavior, saving fails instead of downgrading silently.

Single-link saves retain atomic replacement. A hard-linked file cannot be both
atomically replaced and kept on its shared inode: Catomic fully writes and syncs
a sibling staging file, rechecks that the target still names the inspected
inode, then truncates, writes, and syncs that inode. If the in-place write or
sync fails, aliases may contain partial new content; Catomic reports that risk
and keeps the complete staged file at the path named in the error for recovery.
Failures before the in-place update leave every alias unchanged and remove the
staging file.

Do not remove ACLs, attributes, or links merely to appease the editor unless you
understand why they exist.

## Troubleshooting

### `UTF-8 locale required`

Inspect the active locale:

```sh
locale
printf 'LC_ALL=%s\nLC_CTYPE=%s\nLANG=%s\n' "$LC_ALL" "$LC_CTYPE" "$LANG"
```

Select a UTF-8 locale available on the system, for example:

```sh
export LANG=C.UTF-8
```

`LC_ALL` overrides the other variables when non-empty, so an old `LC_ALL=C`
will still cause refusal even if `LANG` is UTF-8.

### A shortcut does nothing or inserts the wrong key

The terminal emulator or multiplexer probably intercepted or rewrote it. Try
the fallback keys first: `F1` for help and `F2` for the command prompt. Check the
terminal, tmux, screen, desktop, and SSH-client key mappings. You can also map a
different chord in `[keybindings]`.

If `Ctrl+Shift+Z` undoes instead of redoing, the terminal omitted the Shift
modifier and Catomic received an event indistinguishable from `Ctrl+Z`. Catomic
must treat that event as undo; use `Ctrl+Y` or configure another redo chord.

### `Ctrl+Backspace` deletes one grapheme instead of one word

The terminal path did not preserve the modifier. Catomic requests enhanced
keyboard reporting, but a legacy terminal, SSH client, or multiplexer may
ignore or rewrite it. Use the `delete-word-backward` keybinding fallback shown
under [Editing and navigation](#editing-and-navigation), or configure the
terminal to send the explicit CSI-u bytes `ESC [ 127 ; 5 u` for
`Ctrl+Backspace`.

Source builds include a one-key compatibility probe. Run each command once for
plain `Backspace` and once for `Ctrl+Backspace`:

```sh
cargo run --quiet --example keyboard_probe -- legacy-bytes
cargo run --quiet --example keyboard_probe -- enhanced-bytes
cargo run --quiet --example keyboard_probe -- legacy-event
cargo run --quiet --example keyboard_probe -- enhanced-event
```

In an enhanced path, plain `Backspace` is normally `1b 5b 31 32 37 75`
(`CSI 127u`) and decodes as Backspace without modifiers. `Ctrl+Backspace` is
`1b 5b 31 32 37 3b 35 75` (`CSI 127;5u`) and decodes as Backspace with
`CONTROL`; its raw burst may first include a separate protocol record for the
physical Control-key press. The event probe skips only that standalone modifier
record and reports the Backspace event. A legacy `Ctrl+Backspace` may instead
produce `08`, which Crossterm 0.28 decodes as `Ctrl+H`, or the same `7f` and
unmodified Backspace event as the plain key. Neither legacy result identifies
`Ctrl+Backspace`, so that path needs the fallback.

Repeat the probe directly and inside tmux when diagnosing a difference. Record
the terminal name/version, `TERM`, `tmux -V`, whether SSH is involved, all eight
probe results, and the result of this live Catomic check: type `one two`, press
plain `Backspace` and verify only `o` is removed; undo, then press
`Ctrl+Backspace` and verify `two` is removed as one undoable edit. Recent tmux
versions require extended-key support on the outer terminal; an incorrect
`TERM` or disabled `extkeys` terminal feature can retain the legacy behavior.
For tmux 3.5 with Kitty as the outer terminal, this configuration requests the
minimal disambiguation flag from Kitty and emits CSI-u into the pane:

```tmux
set -s extended-keys on
set -s extended-keys-format csi-u
set -as terminal-features ',xterm-kitty:extkeys'
set -as terminal-overrides ',xterm-kitty:Eneks=\E[>1u:Dseks=\E[<1u'
```

Restart the tmux server after changing these server and terminal options. Do
not use Kitty's `REPORT_ALL_KEYS` flag in the outer override: tmux 3.5 may split
an unmodified `CSI 127u`; flag 1 keeps plain Backspace as `7f` while preserving
`CSI 127;5u` for the modified chord.

### F7 says the preference was not saved

The in-memory toggle still applies to every buffer in the current session.
Check that `XDG_STATE_HOME`, or the `HOME` fallback, is an absolute writable
path and that its `catomic` directory can be created. Catomic leaves an existing
complete preference file in place when atomic replacement fails.

### System clipboard copy or paste does not work

Catomic's internal `Ctrl+C`/`Ctrl+X`/`Ctrl+V` clipboard should still work within
the session. For local Linux copy, check that `WAYLAND_DISPLAY` or `DISPLAY` is
present and install `wl-clipboard`, `xclip`, or `xsel` as appropriate. WSL uses
`clip.exe`; Termux uses `termux-clipboard-set`. Over SSH, enable OSC 52 writing
in the local terminal and any intervening multiplexer. External paste still
depends on the terminal delivering bracketed paste.

`Ctrl+Shift+C` is Catomic's default configurable `interrupt` action, not its
copy key. If a terminal reserves `Ctrl+C`, Catomic cannot receive the copy
action; change the terminal binding or use a remapped Catomic copy action.

### Mouse clicks do not reach Catomic

Catomic requests xterm-compatible button, drag, and SGR mouse reporting while
the editor is active, then disables every requested mouse mode during terminal
teardown. If clicks, drags, double clicks, and wheel events all do nothing, the
terminal, multiplexer, or SSH client is probably not forwarding mouse reports.
Keyboard navigation remains available when forwarding is unavailable.

First compare the same Catomic command inside and outside the multiplexer, and
record the terminal version plus `TERM`, `TMUX`, and `STY`. Inside tmux, inspect
`tmux show-options -gv mouse` and any custom `Mouse...Pane` bindings. A custom
binding that consumes an event must use `send-keys -M` when it intends to pass
that event to the program in the pane; see the official
[tmux mouse-support manual](https://man.openbsd.org/tmux.1#MOUSE_SUPPORT),
[tmux mouse guide](https://github.com/tmux/tmux/wiki/Getting-Started#using-the-mouse)
and [tmux FAQ](https://github.com/tmux/tmux/wiki/FAQ#how-do-i-use-the-mouse) for
the current forwarding and terminal-bypass behavior.

Many terminals reserve a modifier such as `Shift` for selecting terminal
scrollback even when an application requested mouse input. In Ghostty this is
the documented native-selection coexistence path: unmodified mouse events reach
Catomic, while `Shift` selection stays in Ghostty and uses its copy-on-select
setting. Do not hold that bypass modifier when testing Catomic cursor mapping;
do hold it when testing Ghostty-native selection. Check terminal mouse-reporting
settings, then include whether click, drag, double-click, wheel, and bypassed
selection work in a bug report along with the Catomic version and terminal
dimensions; Catomic cannot recover coordinates that never reach its PTY.

### Save is refused after another program edited the file

Read the status message. Use `Ctrl+R` twice to accept the unchanged observed disk
revision, or Save As to preserve your local version elsewhere. Use `Ctrl+S`
twice only when you intentionally want to overwrite the exact external revision
you were warned about. Any further drift invalidates the second press.

### Save is refused because of filesystem semantics

Check the target type, link count, extended attributes, and ACLs:

```sh
stat path/to/file
getfattr -d path/to/file 2>/dev/null
getfacl path/to/file 2>/dev/null
```

Commands vary by distribution. Network, FUSE, overlay, and container filesystems
may expose weaker or different replacement and in-place durability behavior.
For a failed hard-link update, preserve the staged path printed in the error
until you have compared or restored its contents.

### `lint` reports no configured linter

Confirm that you:

1. saved the active buffer;
2. configured its normalized extension under `[languages.EXT]` or `[linters]`;
3. included `{file}` in the command; and
4. installed the linter in the environment where Catomic runs.

### A model command cannot connect or refuses the endpoint

Open `models` and inspect the preset state and exact destination. For HTTP,
check the base URL and whether the service implements OpenAI-compatible Chat
Completions. Make sure `api_key_env`/`header_envs` name present environment
variables rather than containing keys. Authenticated remote endpoints require
HTTPS; redirects, embedded URL credentials, ambient proxies, and non-loopback
plaintext keys are intentionally refused.

For command presets, install the displayed resolved executable and verify that
the configured CLI version emits exactly the declared structured format. A
missing binary, timeout, oversized output, non-UTF-8/malformed/partial JSON,
non-zero exit, or tool event fails closed. Raw stderr and provider response
bodies are intentionally not printed; use the CLI's own safe diagnostic command
outside Catomic when authentication or version setup needs more detail.

An edit response must be a single-file unified patch for the confirmed active
path, or the strict selected-region replacement envelope. Prose or full-file
replacement output is rejected and cannot bypass the edit parser.

### The terminal looks broken after a crash

Catomic installs teardown guards and signal/panic restoration, but a hard kill or
terminal failure can bypass user-space cleanup. Run:

```sh
reset
```

Then report the crash with the terminal name/version, `$TERM`, locale, Linux
details, filesystem type, Catomic version, exact keystrokes, and a non-sensitive
fixture.

## Current limitations and reporting bugs

- Linux terminals are the first-class platform. Windows and macOS are not yet
  supported targets.
- There is no tree-sitter parser, full LSP client, split view, embedded scripting
  API, or plugin ABI.
- Highlighting is lexical and viewport-only.
- Repository model edits remain single-file and preview-first.
- Terminal clipboard and modified-key behavior vary by emulator and multiplexer.

For a reproducible non-sensitive bug, use the
[bug report form](https://github.com/maelguimet/catomic/issues/new?template=bug_report.yml).
Remove private file contents and credentials from reports. Security-sensitive
findings belong in the private process described by [SECURITY.md](../SECURITY.md).
