# Catomic user guide

Catomic is a Linux-first, modeless terminal text editor. Its default Plain mode
behaves like a conventional editor and does not scan repositories, start
linters, or contact a network service. Project tools and model-assisted commands
exist, but they run only after you invoke them explicitly.

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
- [Plain mode and Project mode](#plain-mode-and-project-mode)
- [Completion](#completion)
- [Linters and diagnostics](#linters-and-diagnostics)
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

To install it into Cargo's binary directory, normally `~/.cargo/bin`:

```sh
cargo install --path . --locked
```

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
- A binary built in a clean official `master` checkout, including
  `cargo install --path . --locked`, retains that checkout as its update source.
  Catomic checks the official remote revision, refuses non-fast-forward history,
  fetches without running hooks, and builds in an isolated temporary worktree.
  The new revision must pass all tests and validate the existing configuration
  before the executable is replaced. Only then is the source checkout
  fast-forwarded.
- Cargo registry installs, detached Git installs, forks, missing source
  checkouts, non-`master` branches, and architectures without a managed release
  are reported as unsupported. The command exits without changing anything and
  prints a manual Cargo command where applicable.

Dirty source checkouts are never stashed, reset, cleaned, or overwritten.
Commit, stash, or back up both tracked and untracked work yourself, then rerun
the updater. This deliberately leaves stash policy under your control.

### Atomic install and recovery

The new executable is staged beside the installed one, synced, and atomically
renamed over it. Before that rename, Catomic creates a sibling rollback binary
containing the old bytes. A failed download, checksum, test, build,
configuration validation, or staging step leaves the installed executable
untouched. If final source fast-forwarding fails after replacement, Catomic
automatically restores the old binary.

On success Catomic prints the old and new versions, backup status, rollback
path, and an exact recovery command. Roll back manually with the printed
command, which has this shape:

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
backup failure; `7` candidate configuration failure; `8` test/build failure;
and `9` install or rollback failure.

If the updater is unavailable for a source install, the manual equivalent is:

```sh
git pull --ff-only
cargo install --path . --locked --force
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

Open several files in one session:

```sh
catomic notes.txt todo.txt server.log
```

Start with an untitled empty buffer:

```sh
catomic
```

If a named path does not exist, Catomic opens an empty buffer for that path. The
file is not created until you save it.

When two or more file arguments include any missing path, Catomic stops before
entering terminal mode and prints every parsed argument as `existing`,
`missing`, or status unavailable. This guards against an unquoted filename with
spaces accidentally becoming several buffers. For example, if you intended one
file, quote it in the shell:

```sh
catomic "henlo world.md"
```

To deliberately open or create several buffers when any path is missing, use
the explicit opt-in:

```sh
catomic --allow-missing draft-one.txt draft-two.txt
```

The guard's classification is a read-only existence check; neither the guard
nor the opt-in creates a file. An accepted missing path remains an empty,
unsaved buffer until you explicitly save it. Several existing paths continue to
open without the flag.

The first argument `update` selects the updater. Use `--` when a filename looks
like an option or is literally named `update`:

```sh
catomic -- --help
catomic -- update
catomic --allow-missing -- --draft.txt another-draft.txt
```

File arguments and file contents must be valid UTF-8. The editor also requires
a UTF-8 locale selected by the first non-empty value among `LC_ALL`,
`LC_CTYPE`, and `LANG`. Help and version output remain available when the locale
is invalid because they do not enter terminal raw mode.

## The editor screen

The main area contains the active buffer. The bottom line shows a transient
message when an operation needs attention; otherwise it shows persistent state,
including:

- the current mode (`plain` or `project`);
- the active path, or an untitled-buffer label;
- whether the active buffer has unsaved changes;
- file size, size tier, and text format when known;
- `buffer N/M` when several buffers are open; and
- page information and source byte range for a paged large file.

The small cat decoration is enabled by default. It changes presentation only
and can be disabled with `[cat] status_messages = false`.

Press `Ctrl+H` or `F1` at any time to open the built-in shortcut reference. It
is read-only: use the arrow keys, `Home`, `End`, `PageUp`, and `PageDown` to
navigate, then press `Escape` or `Ctrl+H` to return.

### Prompts and read-only views

`Ctrl+Shift+P` or `F2` opens the command prompt. Commands are entered without a
leading colon. For example, type `project`, not `:project`. This guide uses
plain command names for that reason.

Most prompts and read-only result views follow the same small interaction model:

- `Enter` accepts, opens, applies, or advances the current operation;
- `Escape` cancels or closes it;
- arrow keys and page keys navigate read-only content; and
- `Ctrl+Q` still reaches the normal quit guard.

## Editing and navigation

Catomic uses familiar modeless editing. Type to insert text. `Backspace` and
`Delete` remove text, and `Enter` inserts a newline.

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
| Delete previous or next word | `Ctrl+Backspace` / `Ctrl+Delete` |

Add `Shift` to the movement forms to extend the selection. `Ctrl+A` selects the
active ordinary buffer or the current page of a paged file.

### Mouse selection

- A left click moves the cursor.
- A left-button drag selects text.
- A double click selects the word or punctuation run under the pointer.

Mouse coordinates follow visible terminal cells, including tabs, wide
characters, line numbers, and soft-wrapped lines.

A click in the line-number gutter moves to the start of that displayed row; for
a soft-wrapped continuation, that is the continuation's first document column.
A click past the rendered end of a line moves to its end. The bottom status row
is not document content and ignores clicks. Prompts and read-only views also
ignore document clicks; close the active surface before positioning the
editable source cursor.

### Clipboard and paste

`Ctrl+C`, `Ctrl+X`, and `Ctrl+V` use Catomic's process-local clipboard. The
clipboard is shared by all buffers in the current session. Copying also sends
the text through OSC 52 when the terminal supports it, so the terminal or host
clipboard may receive the same value.

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

Every accepted command-line path opens in argument order. Multi-file startup
with a missing path requires the [explicit `--allow-missing` opt-in](#starting-catomic).
You can also create or open buffers during a session:

| Action | Shortcut | Command |
| --- | --- | --- |
| Open a path | `Ctrl+O` | `open PATH`, `edit PATH`, `e PATH` |
| New untitled buffer | `Ctrl+N` | `new` |
| Close active buffer | `Ctrl+W` | `close` |
| Discard and close a dirty buffer | — | `close!` |
| Next buffer | `Alt+PageDown` | — |
| Previous buffer | `Alt+PageUp` | — |

Opening an already-open path switches to or reports the existing buffer rather
than creating a duplicate. Each buffer retains its cursor, viewport, selection,
dirty state, file watcher, display toggles, and large-file page position.

`Ctrl+S` saves only the active buffer. `Ctrl+Q` checks every open buffer; if any
are dirty, the first press warns and the second press quits without saving.

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

### Atomic saves

Catomic writes a temporary sibling, verifies the resulting file state, and
atomically replaces the destination. On Linux it preserves the mode, owner, and
group of an existing regular file. Replacing the destination changes its inode;
software that relies on inode identity should account for that.

An atomic save removes a matching `.catnap` recovery sidecar after the source
save succeeds.

### External file changes

File watching is enabled in both Plain and Project mode. Watch notifications are
hints; Catomic captures a fresh bounded disk identity before acting. Files up to
100 MiB use a streaming SHA-256 of the complete content in addition to size,
timestamps, and Unix device/inode/change time. This detects rapid same-length
rewrites even when every available metadata field collides.

- A clean buffer reloads automatically when the file changes or is deleted,
  unless `[files] auto_reload = false`.
- A dirty buffer is never discarded automatically.
- `Ctrl+R` explicitly checks for an external change or confirms a reload.

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
| Markdown preview | `F6` | Preview the buffer or active large-file page |
| Line numbers | `F7` | Toggle line numbers for the active buffer |
| Visible whitespace | `F8` | Show spaces and tabs |
| Soft wrapping | `F9` | Wrap at terminal width without inserting newlines |

Press `F6` again or `Escape` to leave Markdown preview. Soft-wrapped
continuations preserve document coordinates and mouse mapping. View settings
are stored independently per open buffer.

Markdown preview renders headings, nested quotes and lists, tasks, links,
footnotes, rules, and fenced code with terminal-native markers. Tables retain
their parsed column alignments, measure grapheme display cells (including wide
characters, combining marks, and emoji), and use a heavier separator below the
header. Inline formatting and escaped pipes stay inside their cells.

To keep explicit preview construction from amplifying a single huge cell,
individual table cells are capped at 40 terminal cells and clipped at a
grapheme boundary with `…`. A complete table can still be wider than the
terminal; use `Left`, `Right`, `Home`, and `End` to move the read-only preview
cursor and pan horizontally. No source text is changed. Raw HTML is displayed
as inert text; terminal control characters are converted to visible safe
glyphs by the normal renderer rather than executed.

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
- The status line shows the page number and source byte range.
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

## Plain mode and Project mode

Catomic always starts in Plain mode.

### Plain mode

Plain mode includes ordinary editing, file watching, Markdown, local
current-buffer completion, and explicitly invoked current-file model commands.
It does not construct repository scanners, linters, diagnostics, background
indexers, repository-aware model machinery, or a network client at startup.

### Project mode

Run `project` or `code` in the command prompt to opt in. The Project root is the
active file's parent directory, or the current working directory when the
buffer has no usable parent.

Project mode enables explicit file discovery, linting, diagnostics, cached-path
completion, and repository-aware model commands. These features remain lazy:
entering Project mode does not itself scan the tree, run a linter, or contact a
model.

Run `plain` or `text` to leave Project mode. This stops Project tasks, discards
the Project session and its cached discovery results, and restores the Plain
capability set.

### File discovery

In Project mode, run `files` to start a bounded, cancellable scan under the
Project root. Nothing scans before this command.

The result opens as a read-only picker:

- use arrows or page keys to move;
- press `Enter` to open the selected file in a buffer; and
- press `Escape` to close the picker or cancel a running scan.

The most recent discovery result also becomes the path-completion cache. The
scan is bounded to 4,096 returned files, 65,536 visited entries, and 64 levels
of depth; the result tells you when it is partial or contains unreadable
directories.

## Completion

Press `Ctrl+Space` or `Tab` to request completion.

- `Tab` and `Shift+Tab` cycle candidates.
- `Enter` accepts the active candidate.
- `Escape` dismisses the list.

Plain mode derives candidates only from a bounded window of the current buffer.
Project mode can additionally use path-like candidates from the last explicit
`files` result. Completion never starts a project scan by itself. If there is no
completion candidate, `Tab` performs normal indentation.

Accepting a completion is one undoable replacement. Catomic does not enable
continuous ghost text or a background completion service.

## Linters and diagnostics

Linting is explicit and Project-only. Configure a command for the file
extension, enter Project mode, save the active buffer, then run `lint`.

Language-specific configuration is preferred:

```toml
[languages.rs]
tab_size = 4
linter = "cargo check --message-format short {file}"

[languages.py]
tab_size = 4
linter = "ruff check {file}"

[languages.js]
tab_size = 2
linter = "eslint {file}"
```

Every linter command must contain `{file}`. Catomic shell-quotes and substitutes
the absolute active path, then runs the command from the Project root without
blocking typing. `Escape` cancels a running linter.

After a run:

- `diagnostics` or `dlist` opens the read-only result list;
- `dnext` jumps to the next diagnostic; and
- `dprev` jumps to the previous diagnostic.

Jumping can open an already-discovered diagnostic file in another buffer. The
parser accepts common `file:line:column: message`-shaped output. If a command
fails without parseable diagnostics, Catomic reports the exit state instead of
inventing a clean result.

The older extension table remains supported, but a `[languages.EXT]` linter
wins when both are present:

```toml
[linters]
rs = "cargo check --message-format short {file}"
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

Model support uses an OpenAI-compatible Chat Completions endpoint. It is
explicit, transient, and preview-first. Catomic does not construct an HTTP
client or read the configured API-key environment variable until you invoke a
model command and confirm its destination and context.

### Endpoint configuration

The defaults target a local service:

```toml
[llm]
base_url = "http://127.0.0.1:8080/v1"
model = "local-model"
api_key_env = "OPENAI_API_KEY"
timeout_secs = 120
```

The base URL must be plain HTTP or HTTPS without embedded credentials,
whitespace, query, or fragment. Timeouts must be 1–600 seconds.

Loopback HTTP may use an API key. Unauthenticated LAN HTTP is also allowed for
local models. Catomic refuses to send an API key to a non-loopback plaintext
HTTP endpoint; use HTTPS for an authenticated remote endpoint.

The client refuses redirects and ignores ambient proxy variables so context
cannot silently leave through a destination other than the one you confirmed.

### Current-file commands

These commands are available from Plain or Project mode:

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
instruction. An instruction beginning with `explain` requests a read-only answer
instead of an edit proposal.

Before sending, Catomic shows the canonical endpoint, model, exact context
extent, and warnings for a dotfile path or obvious secret-like lines. `Enter`
confirms the request; `Escape` cancels without constructing the client.

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
capped at 64 KiB for either command. Both commands require:

- explicit Project mode;
- a saved active file inside a Git repository; and
- a stable repository and active-file snapshot throughout the request.

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

No file is required. Unknown keys are ignored for forward compatibility, but a
malformed recognized value is a startup error for settings loaded at startup.
Project-only and model settings are loaded lazily when invoked.

### Complete example

```toml
[editor]
tab_size = 4

[big_files]
page_lines = 20_000

[files]
auto_reload = true

[cat]
status_messages = true

[recovery]
enabled = false
interval_secs = 30
max_bytes = 1_048_576

[languages.rs]
tab_size = 4
linter = "cargo check --message-format short {file}"

[languages.py]
tab_size = 4
linter = "ruff check {file}"

[keybindings]
"alt+s" = "save-as"
"alt+f" = "search"

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
base_url = "http://127.0.0.1:8080/v1"
model = "local-model"
api_key_env = "OPENAI_API_KEY"
timeout_secs = 120
```

### Setting limits and defaults

| Setting | Default | Valid values |
| --- | --- | --- |
| `editor.tab_size` | `4` | Integer `1`–`16` |
| `languages.EXT.tab_size` | global value | Integer `1`–`16` |
| `languages.EXT.linter` | none | String containing `{file}` |
| `big_files.page_lines` | `20000` | Positive integer |
| `files.auto_reload` | `true` | Boolean |
| `cat.status_messages` | `true` | Boolean |
| `recovery.enabled` | `false` | Boolean |
| `recovery.interval_secs` | `30` | Integer `5`–`3600` |
| `recovery.max_bytes` | `1048576` | Integer `1`–`16777216` |
| `commands.NAME.input` | `none` | `none`, `selection`, `buffer` |
| `commands.NAME.output` | `preview` | `preview`, `insert`, `replace-input` |
| `commands.NAME.timeout_secs` | `10` | Integer `1`–`300` |
| `llm.base_url` | `http://127.0.0.1:8080/v1` | Canonical HTTP(S) base URL |
| `llm.model` | `local-model` | Non-empty string |
| `llm.api_key_env` | `OPENAI_API_KEY` | Valid environment-variable name |
| `llm.timeout_secs` | `120` | Integer `1`–`600` |

Language extension names are case-normalized and may be written with or without
a leading dot. Command names may contain ASCII letters, digits, `-`, and `_`.

### Custom keybindings

Keybinding overrides translate a normal-mode chord to an existing action:

```toml
[keybindings]
"ctrl+w" = "save"
"alt+s" = "save-as"
"alt+f" = "search"
"ctrl+shift+g" = "command-prompt"
"alt+u" = "undo"
"alt+r" = "redo"
```

Chord modifiers are `ctrl`/`control`, `alt`, and `shift`. Keys may be one
character, `space`, `tab`, `enter`, `esc`, `backspace`, `delete`, an arrow key,
`pageup`, `pagedown`, `home`, `end`, or `f1` through `f12`.

Supported action names are:

```text
help save save-as open new close replace quit reload search goto-line
command-prompt undo redo complete next-buffer previous-buffer next-page
previous-page markdown-preview line-numbers whitespace soft-wrap
```

Overrides apply in normal editing mode. Prompt, picker, preview, and completion
keys remain local to the active interface.

## Shortcut reference

| Category | Action | Shortcut |
| --- | --- | --- |
| App | Help | `Ctrl+H` or `F1` |
| App | Quit; press twice to discard dirty buffers | `Ctrl+Q` |
| Files | Save | `Ctrl+S` |
| Files | Save As | `Ctrl+Shift+S` |
| Files | Open | `Ctrl+O` |
| Files | New buffer | `Ctrl+N` |
| Files | Close clean buffer | `Ctrl+W` |
| Files | Check/reload external change | `Ctrl+R` |
| Buffers | Previous / next buffer | `Alt+PageUp` / `Alt+PageDown` |
| Editing | Select/copy/cut/paste | `Ctrl+A` / `Ctrl+C` / `Ctrl+X` / `Ctrl+V` |
| Editing | Undo | `Ctrl+Z` |
| Editing | Redo | `Ctrl+Y` / `Ctrl+Shift+Z` |
| Editing | Indent / unindent | `Tab` / `Shift+Tab` |
| Editing | Delete previous / next word | `Ctrl+Backspace` / `Ctrl+Delete` |
| Navigation | Move by word | `Ctrl+Left` / `Ctrl+Right` |
| Navigation | Start / end of document | `Ctrl+Home` / `Ctrl+End` |
| Search | Find | `Ctrl+F` |
| Search | Replace next | `Ctrl+Shift+F` |
| Search | Go to line | `Ctrl+G` |
| Tools | Command prompt | `Ctrl+Shift+P` or `F2` |
| Tools | Completion | `Ctrl+Space` |
| View | Markdown preview | `F6` |
| View | Line numbers | `F7` |
| View | Visible whitespace | `F8` |
| View | Soft wrapping | `F9` |
| Large files | Previous / next page | `Ctrl+PageUp` / `Ctrl+PageDown` |

This table and the in-editor `Ctrl+H`/`F1` quick reference show built-in
defaults. `[keybindings]` overrides apply in normal editing mode, but neither
reference rewrites its labels to show the effective configured chords.

## Command reference

Open the prompt with `Ctrl+Shift+P` or `F2`. Do not add a leading colon.

| Command | Aliases | Purpose / requirement |
| --- | --- | --- |
| `help` | `shortcuts` | Open built-in help |
| `save` | `write`, `w` | Save active buffer |
| `save as PATH` | `save-as PATH`, `saveas PATH` | Save to a new path |
| `open PATH` | `edit PATH`, `e PATH` | Open path in a buffer |
| `new` | — | Create an untitled buffer |
| `close` | — | Close active clean buffer |
| `close!` | — | Discard and close active dirty buffer |
| `goto LINE` | `line LINE` | Go to a 1-based line |
| `replace` | — | Replace next match |
| `replace-all` | `replaceall` | Replace all in an ordinary buffer |
| `project` | `code` | Enter Project mode |
| `plain` | `text` | Leave Project mode and stop Project services |
| `files` | — | Run Project file discovery and open picker |
| `lint` | — | Run configured linter on saved active file in Project mode |
| `diagnostics` | `dlist` | Open last diagnostic list |
| `dnext` | — | Jump to next diagnostic |
| `dprev` | — | Jump to previous diagnostic |
| `run NAME` | — | Run a configured trusted command |
| `recover` | — | Preview a newer `.catnap` sidecar |
| `meow TEXT` | — | Send selection/instruction block to configured model |
| `bigmeow TEXT` | — | Send current ordinary file to configured model |
| `gitmeow TEXT` | — | Use focused bounded repository context in Project mode |
| `megameow TEXT` | — | Use broader bounded repository context in Project mode |
| `quit` | `q` | Use the normal guarded quit path |

## File formats and save safety

### Text encoding and line endings

Catomic accepts valid UTF-8 text. Ordinary buffers preserve:

- an optional UTF-8 byte-order mark;
- LF line endings;
- CRLF line endings; or
- CR-only line endings.

The detected format appears in the status line. UTF-16, arbitrary binary data,
and non-UTF-8 filenames are refused rather than decoded or rewritten
heuristically.

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
- files with more than one hard link are refused; and
- files carrying extended attributes or ACLs are refused because atomic inode
  replacement could silently discard those semantics.

On Linux, replacement must preserve mode, owner, and group. If the filesystem,
mount, container, or network share cannot provide the required atomic and
metadata behavior, saving fails with an error instead of downgrading silently.

Use a different tool for a refused target. Do not remove ACLs, attributes, or
links merely to appease the editor unless you understand why they exist.

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

### System clipboard copy or paste does not work

Catomic's internal `Ctrl+C`/`Ctrl+X`/`Ctrl+V` clipboard should still work within
the session. System clipboard export requires OSC 52 support, and external paste
depends on the terminal delivering bracketed paste. Some terminals reserve
`Ctrl+Shift+C` and `Ctrl+Shift+V`; those are terminal shortcuts, not Catomic's
internal clipboard bindings.

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
scrollback even when an application requested mouse input. Do not hold that
bypass modifier when testing Catomic. Check terminal mouse-reporting settings,
then include whether click, drag, double-click, and wheel events work in a bug
report along with the Catomic version and terminal dimensions; Catomic cannot
recover coordinates that never reach its PTY.

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
may expose weaker or different atomic-replacement behavior. Use another editor
for a target Catomic cannot preserve safely.

### `lint` reports no configured linter

Confirm that you:

1. entered Project mode with `project`;
2. saved the active buffer;
3. configured its normalized extension under `[languages.EXT]` or `[linters]`;
4. included `{file}` in the command; and
5. installed the linter in the environment where Catomic runs.

### A model command cannot connect or refuses the endpoint

Check the configured base URL and whether the service implements an
OpenAI-compatible Chat Completions endpoint. Make sure `api_key_env` names an
environment variable rather than containing the key itself. Authenticated remote
endpoints require HTTPS; redirects, embedded URL credentials, ambient proxies,
and non-loopback plaintext keys are intentionally refused.

An edit response must be a single-file unified patch for the confirmed active
path, or the strict selected-region replacement envelope. Prose or full-file
replacement output can be viewed as an explanation only when requested; it
cannot bypass the edit parser.

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
