# Catomic

[![CI](https://github.com/maelguimet/catomic/actions/workflows/ci.yml/badge.svg)](https://github.com/maelguimet/catomic/actions/workflows/ci.yml)

Catomic is the editor I wanted when Nano felt too bare and everything else felt
like moving into somebody else's operating system. It is Linux-first, modeless,
quick to open, and full of shortcuts that already make sense.

It is also in open beta. Use it, break it, tell me what got weird. But back up
anything precious and read the [file-semantics limitations](#limitations) before
making it your only editor.

![Catomic terminal text editor open on a Rust source file](docs/assets/catomic.jpg)

## The good stuff

- Familiar editing: selection, mouse input, search/replace, goto line,
  undo/redo, multiple buffers, and GUI-style shortcuts.
- Proper Unicode cursor movement and layout, including grapheme clusters, wide
  characters, emoji sequences, and tabs.
- Large-file paging, so opening a monster log does not mean swallowing the whole
  thing into one enormous string.
- Atomic saves, external-change detection, overwrite confirmation, and optional
  `.catnap` recovery for when reality happens.
- Fast, viewport-only highlighting for Markdown, Rust, Python, and JSON, plus a
  read-only Markdown preview.
- Project mode when you ask for it: files, linting, diagnostics, and cached path
  completion. Plain mode stays plain and does not quietly scan your repository.
- Cat-themed model commands, because of course. They are explicit and
  preview-first: nothing is sent until you invoke a command and confirm where it
  is going and what context it gets.
- Optional inline writing suggestions stay off until you confirm one bounded
  active-buffer scope for the session; ghost text never changes the file unless
  you accept it with `Tab`.

## Install from source

Catomic currently targets Linux and stable Rust. Clone the repository, then
build an optimized binary:

```sh
git clone https://github.com/maelguimet/catomic.git
cd catomic
cargo build --release --locked
./target/release/catomic
```

To install `catomic` into Cargo's binary directory and create its private,
commented user configuration:

```sh
./scripts/install.sh
```

The installer never replaces an existing configuration.

For a clean official `master` checkout, either launch method supports the
state-preserving updater:

```sh
catomic update --check
catomic update --backup
```

The updater never rewrites Catomic configuration or local source changes. See
[Updating, backup, and rollback](docs/user-guide.md#updating-backup-and-rollback)
for supported install methods and recovery behavior.

## Start editing

Open one file, or start with an untitled buffer:

```sh
catomic notes.md
catomic hello world.md
catomic
```

All non-command words form one filename, joined with spaces, so the second
example opens exactly `hello world.md`. Quoting filenames remains supported but
is not required. A missing path opens as an empty named buffer and is never
created until explicitly saved.

Run `catomic --help` for command-line behavior and examples. Inside the editor,
press `Ctrl+H` or `F1` for the built-in default-key and prompt-command quick
reference; it identifies how configured keybinding overrides relate to the
displayed defaults.
On Android/Termux, the automatic action row and its **Menu** palette expose the
same essential workflow without a hardware keyboard. See the
[mobile guide](docs/mobile.md) for the supported environment and storage limits.
For installation, editing workflows, configuration, safety behavior, and
troubleshooting, see the [complete user guide](docs/user-guide.md).

### Essential shortcuts

| Action | Shortcut |
| --- | --- |
| Save / Save As | `Ctrl+S` / `Ctrl+Shift+S` |
| Open / new / close buffer | `Ctrl+O` / `Ctrl+N` / `Ctrl+W` |
| Previous / next buffer | `Alt+PageUp` / `Alt+PageDown` |
| Undo | `Ctrl+Z` |
| Redo | `Ctrl+Y` / `Ctrl+Shift+Z` |
| Insert / overwrite typing | `Insert` |
| Find / replace / goto line | `Ctrl+F` / `Ctrl+Shift+F` / `Ctrl+G` |
| Previous / next paragraph | `Ctrl+Up` / `Ctrl+Down` |
| Select / copy / cut / paste | `Ctrl+A` / `Ctrl+C` / `Ctrl+X` / `Ctrl+V` |
| Local completion | `Ctrl+Space` |
| Command prompt | `Ctrl+Shift+P` or `F2` |
| Inline clanker / clear its change marks | `F3` / `Shift+F3` |
| Markdown preview | `F6` |
| Line numbers / whitespace / soft wrap | `F7` / `F8` / `F9` |
| Select model/backend for this session | `F10` |
| Previous / next large-file page | `Ctrl+PageUp` / `Ctrl+PageDown` |
| Quit | `Ctrl+Q` |
| Immediate interrupt | `Ctrl+Shift+C` |

Catomic keeps `Ctrl+Shift+Z` as a default GUI-style redo alias. If a terminal
reports that chord without the Shift modifier, it is indistinguishable from
`Ctrl+Z`, so Catomic performs undo. The `undo` and `redo` actions remain
remappable through `[keybindings]`.

Terminal emulators and multiplexers can intercept or rewrite some key chords.
Bracketed paste is inserted as one undoable edit; Catomic also has an internal
clipboard and sends copied text through OSC 52 when the terminal supports it.
Completed mouse selections use the same clipboard path. In Ghostty, `Shift`
keeps the terminal's native selection and copy-on-select bypass available while
Catomic owns unmodified clicks and drags.
The mouse wheel scrolls the viewport without moving the editing cursor or
selection; the next keyboard or editing action reveals the logical cursor.

### Essential prompt commands

Open the prompt with `Ctrl+Shift+P` or `F2`. Enter these commands without a
leading `:`.

| Command | Purpose |
| --- | --- |
| `open PATH`, `new`, `close`, `close!` | Manage buffers; `close!` drops edits |
| `save`, `save as PATH` | Save the active buffer |
| `config` | Open the resolved user configuration (confirm before first creation) |
| `goto LINE`, `replace`, `replace-all` | Navigate and edit |
| `project`, `plain` | Enter or leave opt-in Project mode |
| `files`, `lint`, `diagnostics`, `dnext`, `dprev` | Run Project tools |
| `run NAME` | Run a configured, trusted external command |
| `recover` | Preview and apply a newer `.catnap` sidecar |
| `model`, `models` | Search configured model/backend presets |
| `run-clanker`, `inline-meow` | Run the inline instruction (`selection → catblocks → bounded full file`) |
| `clear-clanker-changes` | Dismiss applied-model highlighting without editing text |
| `autocomplete on`, `autocomplete off` | Enable or disable confirmed inline suggestions |
| `meow TEXT`, `bigmeow TEXT` | Ask a model about this file or selection |
| `gitmeow TEXT`, `megameow TEXT` | Ask a model using repository context |

## Configuration

Catomic reads TOML from `$XDG_CONFIG_HOME/catomic/config.toml` or
`~/.config/catomic/config.toml`. The source installer creates that file from the
commented owner-only template without replacing existing configuration. This
example shows the most common settings:

Use `catomic config` from the shell, or `config` in the command prompt, to open
the exact active path inside Catomic. `catomic config edit` is a compatible alias;
`catomic config path` and `catomic config check` remain non-editor utilities. If
the file is later removed, the editor confirms before recreating the same private
template atomically. Restart Catomic after saving configuration changes.

```toml
[editor]
tab_size = 4

[big_files]
page_lines = 20000

[files]
auto_reload = true

[view]
line_numbers = false

[cat]
status_messages = true

[recovery]
enabled = false
interval_secs = 30
max_bytes = 1048576

[autocomplete]
enabled = false
idle_debounce_ms = 750
minimum_prefix_length = 20
max_context_before = 2048
max_context_after = 512
max_generated_tokens = 64
allow_remote = false

[theme]
name = "default"

[theme.colors]
selection = { fg = "black", bg = "cyan" }
status_warning = { fg = "black", bg = "yellow" }

[keybindings]
save = ["ctrl+s", "alt+s"]
# help = [] # unbind every default for this action

[languages.rs]
tab_size = 4
linter = "cargo check --message-format short {file}"

[llm]
default = "local"

[[llm.backends]]
name = "local"
type = "openai-compatible"
base_url = "http://127.0.0.1:8080/v1"
model = "local-model"
models = ["local-model-small"]

[[llm.backends]]
name = "hosted"
type = "openai-compatible"
base_url = "https://openrouter.ai/api/v1"
model = "provider/model-id"
api_key_env = "OPENROUTER_API_KEY"

[llm.inline]
instruction_prefix = ">>"
context_open = "<catblock>"
context_close = "</catblock>"
warn_lines = 500
block_mode = "combined"
queue_limit = 16
stop_on_error = true
remove_instruction_after_apply = true
```

`F7` changes line numbers for the whole session and atomically remembers that
choice under the XDG state directory for later launches. The saved choice takes
precedence over `[view].line_numbers`; Catomic never rewrites `config.toml`.

Recovery is disabled by default. Named commands and hooks invoke the platform's
POSIX shell (`$PREFIX/bin/sh` on Android/Termux, normally `/bin/sh` on Linux)
and are trusted user configuration; their input, output, and runtime are
bounded, but the command itself can have effects outside Catomic.

LLM presets can use an OpenAI-compatible HTTP endpoint or a configured headless
command with a declared structured-output adapter. Press `F10` or run `models`
to switch the process-local session preset without invoking it or rewriting
configuration. Model actions show the preset, model, destination, and exact
context extent before sending; edits then open read-only and
require a second confirmation before becoming one undoable buffer change.
`F3` discovers a one-line inline instruction, then chooses the active selection,
delimited catblocks, or a bounded full file in that order. Applied model output
remains visibly marked until dismissed, superseded, undone, or invalidated.
Plain HTTP is allowed for loopback models and unauthenticated LAN models. If an
API key is present, Catomic refuses to send it over non-loopback HTTP; use HTTPS
for authenticated remote endpoints. See
[the LLM safety rules](docs/llm-rules.md) for the full boundary.

Inline autocomplete is a narrower opt-in exception to per-call confirmation.
Even when `autocomplete.enabled = true`, the first enable in every process
shows the selected preset, adapter, model, canonical destination, and exact
before/after bounds and waits for `Enter`. While enabled, bounded active-buffer
context is sent automatically after typing pauses. Non-loopback HTTP endpoints
also require `autocomplete.allow_remote = true`. Catomic supplies no repository,
path, filesystem, or tool context; a configured command adapter is trusted user
code and may itself contact services after confirmation.

## Limitations

- Linux terminals and the documented Android/Termux environment are supported
  for this beta, within the exact boundaries in the
  [compatibility matrix](docs/compatibility.md) and
  [mobile guide](docs/mobile.md). Windows, macOS, native Android UI, and
  iOS/iPadOS are not supported, and an untested terminal or mount is not
  silently certified by a different Linux result.
- Editor sessions require a UTF-8 locale, and files must contain valid UTF-8.
  UTF-16 and arbitrary byte-oriented files are refused.
- Ordinary buffers preserve UTF-8 BOMs and LF, CRLF, or CR line endings. Paged
  large files support LF and CRLF; BOM-prefixed or CR-only files must remain
  below the paging threshold.
- Atomic save replaces the destination inode. On Linux, Catomic preserves mode,
  owner, and group, but refuses files with multiple hard links or any extended
  attributes/ACLs rather than silently discarding those semantics. Save As also
  refuses FIFOs, sockets, directories, and symlinks resolving to them. Use
  another tool for a refused target.
- External-change checks fully hash files through 100 MiB. Huge/Extreme paged
  files use metadata plus fixed start/middle/end samples so checks stay bounded;
  an in-place rewrite outside those samples that also preserves size, inode, and
  all available timestamps remains best-effort.
- Terminal clipboard behavior depends on the emulator. Some environments
  intercept `Ctrl`/`Ctrl+Shift` chords or do not support OSC 52.
- Syntax highlighting is deliberately lexical and viewport-only. Catomic does
  not provide tree-sitter, a full LSP client, split views, or a plugin ABI.
- LLM edits are limited to the confirmed active file. Wide multi-file proposals
  and `:feralmeow` are not implemented.

If Catomic crashes, corrupts content, or behaves differently on a particular
filesystem, please use the [bug report form](https://github.com/maelguimet/catomic/issues/new?template=bug_report.yml).
Security-sensitive findings should follow [SECURITY.md](SECURITY.md).

## Project documentation

- [User guide](docs/user-guide.md)
- [Contributing](CONTRIBUTING.md)
- [Architecture and development boundaries](docs/architecture.md)
- [Design decisions](docs/decisions/)
- [Performance discipline and measurements](docs/performance.md)
- [Linux terminal and filesystem compatibility](docs/compatibility.md)
- [LLM safety rules](docs/llm-rules.md)
- [Active bugs, features, and priorities](https://github.com/maelguimet/catomic/issues)
- [Historical roadmap, research, and design record](docs/progress/roadmap-history.md)
- [Release process and artifact verification](docs/releasing.md)
- [Historical v0.1 roadmap acceptance record](docs/v0.1-acceptance.md)

## License

Catomic is available under either the [MIT License](LICENSE-MIT) or the
[Apache License 2.0](LICENSE-APACHE), at your option.
