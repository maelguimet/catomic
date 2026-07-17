# Historical Catomic Build Plan

> Archived from `TODO.md` on 2026-07-17 after phases 0–8 were completed.
> This preserves the original design research, phase specifications, acceptance
> criteria, and implementation record. It is historical evidence, not the
> current roadmap. See [`TODO.md`](../../TODO.md) for active priorities.

# Catomic Build Plan

This document defines the ordered feature roadmap, recommended tech stack, and research on whether to fork existing editors.

See [README.md](./README.md) for the core vision and constraints.

## Goals

- Deliver the "Nano that isn't afraid of useful shortcuts" experience.
- Own a clean, fast piece-table-based core.
- Make big files and external edits first-class.
- Add markdown, linting, autocomplete, and LLM features **without** compromising the editing feel.
- Cat personality as delightful polish, never in the way.
- "Text cursor that feels good" before any cathedral features.

## Product Modes

Catomic has one core editor and two user-facing modes:

### Text Mode (internal: Plain)

Pure writing/editing mode. Fast, calm, obvious.

Enabled:
- open/edit/save/undo/search/goto
- markdown rendering
- file watching
- local, current-buffer word completion only; no background process, no project index
- current-file LLM commands like `:meow` / `:bigmeow` (network impossible unless the user explicitly invokes and confirms endpoint/context)

Disabled by default:
- linters
- LSP
- repo scanning
- aggressive autocomplete
- project diagnostics
- background indexing
- multi-file clanker context

### Code Mode (internal: Project)

IDE-shaped, but not cursed.

Enabled:
- syntax highlighting
- linters
- project file discovery
- repo-aware commands
- `:gitmeow` / `:megameow`
- diagnostics list
- project-aware autocomplete (and later full LSP)
- later LSP, if it earns its keep

**Rules**:
- Code Mode (Project) must never slow down Text Mode (Plain).
- Code Mode features must be opt-in, lazy, and killable.
- No background daemon goblin unless the user asked for it.
- The same buffer/render/editor core powers both modes.

(This design may require adjustments to the phase ordering and feature placement below.)

**Naming note**: Internally the modes are called `Plain` and `Project`. User-facing presentation and commands can stay cute (e.g. `:meow` for current-text/Plain work, `:megameow` for project-aware work). Avoid "catnap" or similar as a mode name unless it earns its keep.

## Capabilities

The `Mode` is not just a label. It is a hard construction bouncer. Every subsystem and background concern must be gated by an explicit `Capabilities` value computed from the active mode (plus any minimal user overrides). Features do not get to exist and then be "unused."

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Plain,
    Project,
}

#[derive(Clone, Debug)]
struct Capabilities {
    /// Markdown rendering + .md-aware display (headings, lists, etc.).
    markdown: bool,
    /// Local, current-buffer-only word completion. Zero processes, zero indexes.
    local_completion: bool,
    /// File watching (Plain-safe external edit detection; does not imply Project).
    file_watch: bool,
    /// Linter execution (manual or otherwise).
    linters: bool,
    /// LSP client.
    lsp: bool,
    /// Any repo/project scanning, file discovery, or indexing.
    repo_scan: bool,
    /// LLM operations that need repo context (gitmeow, :megameow, multi-file, symbol awareness, etc.).
    repo_llm: bool,
    /// Any network-backed LLM activity whatsoever.
    network_llm: bool,
}
```

**Construction rules** (the bouncer):
- At startup (and on any explicit mode switch), compute one `Capabilities` from the current `Mode`.
- A subsystem (linter runner, project scanner, LSP client, repo LLM broker, network LLM client, etc.) is **only instantiated** when its corresponding flag is `true`.
- File watching (when `file_watch`) is Plain-allowed. App owns a best-effort gated FileWatcher (notify) when a watchable path exists; runtime loop checks once per iteration via a non-blocking helper (signals are hints only; fresh observe_external_file is truth). Clean Modified/Deleted buffers auto-reload by default; `[files] auto_reload = false` disables it. Dirty buffers always arm manual confirmation (Ctrl+R). Unchanged/NoPath suppress self-save noise (or clear stale pending). Deterministic seams in tests; live smoke is ignored/manual.
- "Constructed but dormant", "lazy but the factory lives at startup", or "we have the object but we promise not to call it" all fail the rule. If the capability is false, the type must not be present in the running application at all.
- Plain mode **must** produce:
  - `linters: false`, `lsp: false`, `repo_scan: false`, `repo_llm: false`
  - `network_llm: false` (network is unreachable until the user does an explicit current-file LLM action *and* confirms the endpoint + context)
  - `file_watch: true` (Plain-safe; file watching must not imply repo/LSP/network/Project services)
  - `markdown: true`, `local_completion: true` are the main optional positives.
- Code Mode (Project) can turn the rest on, but still lazily and only on demand within the allowed set.

**Test requirement**:
- Plain-mode tests must assert non-construction, not merely non-use.
- Use dependency construction order, DI container assertions, or "no such component in the app state" checks. Counting "we never called .run()" is not sufficient.
- These assertions are part of the Mode Acceptance Tests below.

## Stack Decisions

**Primary recommendation: Rust + crossterm (raw path)**

- **Language**: Rust
  - Excellent performance + safety for a text buffer.
  - Outstanding terminal crates.
  - Easy single static binary distribution.
- **Terminal I/O**: `crossterm`
  - Pure Rust, no system curses dependency.
  - Direct raw mode, ANSI output, event handling (keys + mouse + resize).
  - Matches the "avoid curses if possible" and "draw with ANSI" rules exactly.
- **Rendering philosophy**: Keep it dumb and predictable. Direct writes + cursor control first. Only introduce a higher-level TUI crate (e.g. ratatui) later for optional widgets if it doesn't hurt latency or complexity.
- **Buffer**: Define a `Buffer` trait / interface **immediately** (in Phase 0), before any significant editor loop or UI code. v0 implementation can be dead simple (`Vec<String>`). Later phases only swap the impl behind the trait. Avoid iterator-over-everything as the primary render path in the trait (trait objects hate `impl Iterator`). See the API sketch in Phase 0.
- **File watching**: `notify` crate (cross-platform watcher, but we only care about Linux for v1).
- **Markdown**: `pulldown-cmark` for parsing + a small custom ANSI renderer (headings, lists, fences, quotes). Evaluate `termimad` for preview mode.
- **Config**: Simple TOML (or INI for familiarity). Use `serde` + `toml`.
- **CLI**: `clap` (derive) for flags + file args.
- **LLM**: `reqwest` (async or blocking) targeting OpenAI-compatible endpoints. Local via anything that speaks the same wire format (llama.cpp server, ollama, etc.).
- **LSP (later)**: Consider `tower-lsp` client or raw JSON-RPC. Not before Phase 5+.
- **Syntax highlighting (later)**: Start with simple per-language rules or syntect. Tree-sitter only if we need structure for indent/smart features.
- **Testing**: See the dedicated "## Measurement / Test Discipline" section below. Every phase must ship with unit tests, golden tests, PTY smoke tests, perf targets, and manual UX checklist. No phase is complete until its acceptance tests pass.
- **Packaging**: `cargo build --release`. Provide static binaries. Consider `cargo-binstall` later.

**Avoid (per spec + philosophy)**

- ncurses / pancurses / ncurses-rs
- Full Electron/webview anything
- Heavy widget frameworks until the raw loop is proven fast
- Premature LSP, tree-sitter, or plugin scripting

## Early Design Decision: Column / Cursor Coordinates

**Do this once, early, and write it down.**

What does `col` represent?

- Byte offset into the line?
- Unicode scalar value index (`char` in Rust)?
- Grapheme cluster count?
- Terminal display width (wcwidth / grapheme width)?

**v0 decision**: char index (Unicode scalar value). We treat content as "ASCII-ish UTF-8". Cursor movement, insert, and delete use `.chars().nth(...)` style thinking.

- This is a deliberate temporary simplification.
- Complex text (combining marks, CJK, emoji, tabs, etc.) will be wrong in interesting ways until we decide and implement the real model.
- Before adding selection, search, word movement, or proper rendering, we **must** revisit this decision and update the Buffer trait + all call sites.
- Document the choice and the known limitations in code comments and here.

Do not let "it mostly works on my ASCII files" turn into "we accidentally chose the wrong model for the whole editor."

## Terminal Realities: Copy/Paste, Ctrl+C/V, and the Feral Raccoon

**This is not a small challenge.** Document it here so it isn't forgotten.

In raw mode we have full control:

- Ctrl+C arrives as a normal `KeyEvent` (usually `KeyCode::Char('c')` + `modifiers: CONTROL`). We can choose to treat it as "Copy" instead of letting it become SIGINT. Good: we can implement GUI-like `Ctrl+C = copy`.

- But the terminal / emulator / OS layer is a lawless place:

  - **Paste** is frequently `Ctrl+Shift+V` (because `Ctrl+V` is often claimed by the app or readline muscle memory).
  - Many terminals support **bracketed paste**: you send an enable sequence (`\x1b[?2004h`), and pasted text arrives wrapped in `ESC [ 200 ~ ... ESC [ 201 ~`. When enabled, you get a distinct paste event/stream instead of individual key events. This is the clean way to know "this was a paste, not the user typing fast".
  - Some terminal emulators / SSH / multiplexers (tmux, screen, etc.) mangle or intercept things.
  - On macOS it's often Cmd+V at the emulator level, which may bypass your raw input entirely and just inject characters.
  - Right-click paste, middle-click, menu paste, etc. also just "type" characters.
  - Bracketed paste is not universally supported or enabled.
  - Ctrl+C inside the app for Copy conflicts with users who expect the terminal to handle SIGINT or the emulator's copy.

Catomic's vision wants the familiar shortcuts (`Ctrl+C` copy, `Ctrl+V` paste, etc.) to work *inside the editor*.

**Reality check**: terminal paste will sometimes be a feral raccoon. You will sometimes have to tell users "use Shift+Ctrl+V or your terminal's paste" and it will "just work" by injecting characters (which for many cases is acceptable). For a first-class experience we still want:

- Proper handling of bracketed paste when the terminal supports it (treat large pastes efficiently, maybe disable auto-indent during paste, etc.).
- A way to make `Ctrl+V` (or whatever we bind) do an internal paste from our clipboard.
- Reasonable copy that tries to talk to the system clipboard (OSC 52 escape sequence is the portable-ish way, but support is spotty; fall back to `wl-copy`, `xclip`, `pbcopy`, etc. on best-effort).
- Internal selection + copy buffer that always works even when system clipboard is unreachable.
- Clear documentation of the quirks.

**Implementation guidelines for the goblin loop**:
- Do **not** hard-code "Ctrl+C always means copy" without also considering whether we are in a paste context.
- Prefer enabling bracketed paste early and routing `Paste(data)` events distinctly.
- For shortcuts that conflict with terminal (especially Ctrl+C/V), be prepared to document "Catomic inside the app" vs "what your actual terminal emulator does".
- Test on several terminals (alacritty, kitty, gnome-terminal, foot, wezterm, konsole, over ssh, under tmux...).

We'll get this right, but it will be ugly in places. Own the mess explicitly.

**Ctrl+S footnote**: In classic terminals Ctrl+S is XOFF (pause output / flow control). When you put the terminal into raw mode, crossterm (and the underlying termios) should disable the `IXON` flag so Ctrl+S and Ctrl+Q reach the application as normal keys. It usually works, but:
- Explicitly verify (some setups or older libcs can be surprising).
- If it ever freezes your terminal, `Ctrl+Q` from the shell is the traditional un-freeze.
- Document this for users. Consider a tiny note on first run or in `--help` if we ever see reports.

## Fork / Study Existing Editors — Research Summary

We researched current and classic terminal editors focusing on:
- Modeless / GUI-like shortcut philosophy (matches catomic)
- Raw terminal + ANSI (no curses where possible)
- Buffer data structures (piece table / gap / rope)
- Big file handling
- License friendliness for forking or heavy borrowing
- Maturity vs "do not build cathedral early"

### 1. Fresh (sinelaw/fresh) — https://github.com/sinelaw/fresh (GPL-2.0)

- **Closest spiritual match** to the catomic vision.
- Modeless, standard shortcuts (`Ctrl+S`, `Ctrl+Z`, `Ctrl+F`, `Ctrl+P` etc.), mouse support, command palette, feels like "GUI in terminal".
- Excellent real-world performance on multi-GB files using a **lazy-loading piece tree**.
- Lots of advanced features already shipping: splits, tabs, LSP (multi-server), TypeScript plugins (sandboxed QuickJS), git review, markdown preview, themes, integrated terminal, SSH remote, Vim mode (optional), etc.
- Uses crossterm (confirmed via deps).
- Very popular (7k+ stars), actively developed as of 2026, polished installers everywhere.
- **Recommendation on forking**:
  - **Do not fork the whole project** as the foundation for catomic.
    - Different personality and scope (aims to be a full terminal IDE).
    - GPL-2.0 (derivative must stay GPL).
    - Already solves 70-80% of the listed features; we would spend more time deleting/customizing than building the specific experience.
  - **Do study aggressively**:
    - Huge file loading strategy and piece tree implementation (see their blog posts).
    - Input handling, command palette routing, multi-cursor UX.
    - How they keep latency low.
  - Opportunity: catomic can be the "lighter, cat-flavored, more opinionated cousin" that stays smaller longer and owns different LLM UX (instruction blocks) and stricter "no productivity-hostile gimmicks".

### 2. Microsoft Edit (microsoft/edit) — https://github.com/microsoft/edit (MIT)

- "A simple editor for simple needs." Homage to classic MS-DOS Edit with modern VS Code-style controls.
- Written in Rust, modeless, focused on accessibility for terminal newcomers.
- MIT license is very fork-friendly.
- Smaller scope than Fresh — good reference architecture.
- **Recommendation**: Strong candidate to **study as a clean skeleton** or even lightly fork/adapt the core event loop + rendering for Phase 0/1 if we want a fast high-quality base with good license. Less "IDE", more "editor". Evaluate its buffer approach.

### 3. Kibi (ilai-deutel/kibi) + original Kilo (antirez/kilo) — MIT / public domain spirit

- Kibi: configurable Rust editor in ≤1024 LOC (originally), crossterm-style minimal deps, UTF-8, syntax, search, etc.
- Kilo: the famous ~1k-line C tutorial that started many modern minimal editors.
- **Recommendation**: **Primary teaching/reference material** for the first 1-2 phases.
  - Use to internalize raw mode discipline, minimal redraw strategy, and "get the cursor working beautifully before anything else".
  - Do not copy large amounts of code (license allows, but we want to own the piece table path).

### 4. Other notable projects

- **Amp** (jmacdonald/amp): Rust, gap buffer based, complete terminal editor. Good for gap buffer comparison.
- **Micro** (zyedidia/micro): Go, very popular, Lua plugins, excellent shortcut ergonomics. Great UX inspiration even if we don't use the code.
- **Piece table crates**: `astahfrom/piecetable` (Rust). VS Code's piece tree (TypeScript, well documented). We can implement our own first for learning, then evaluate swapping in a crate.
- Smaller experiments: various "gap buffer in Rust" tutorials, "zord", etc. Minor.

### Final Fork Decision

**Build our own core.** 

Rationale:
- The README explicitly says "Do not build a cathedral before the text cursor works" and calls for a real piece table.
- Owning the buffer + raw render loop is central to the "fast + obvious + hard to destroy work" identity.
- Licenses and scopes of the closest projects don't perfectly align for a clean long-term fork.
- We can move extremely fast in Phase 0-2 by following the kilo/kibi discipline + Microsoft/edit structure while using Fresh as the advanced reference implementation.

**Heavy borrowing / research is encouraged**:
- Data structure lessons from Fresh + VSCode piece tree.
- UX patterns and shortcut expectations from Fresh + Micro.
- Minimal raw terminal code from Kibi/Kilo.
- Never copy-paste large modules without understanding.

If at any milestone the core feels too slow to develop, we can revisit forking Microsoft/edit (MIT) for the skeleton.

## Ordered Feature Phases

Phases are strictly ordered. Do not start work on a later phase until the previous phase's "done" criteria **and its acceptance tests** (see "Measurement / Test Discipline") are met and the editor feels usable for the scope of that phase. No phase is complete until its acceptance tests pass.

**Plain vs Project (Text vs Code Mode) placement rule**: Early phases (0–4) deliver core Plain mode behavior and must produce a `Capabilities` with only Plain flags true. Project/Code features (linters, repo scan, repo LLM, LSP, network LLM clients) land in Phase 5+ and are implemented behind the `Capabilities` bouncer. They must not be constructed (not merely dormant) in Plain mode. See the Capabilities section and Mode Acceptance Tests.

### Phase 0 — One Blessed Goblin Loop (Ultra-minimal MVP)

**Goal**: The absolute smallest editor that is still useful for typing. Cursor moves, you can insert characters (including newlines), delete characters, open a file, save it, and quit. That's it.

No status messages. No dirty prompts. No resize handling/polish. No page up/down. No word movement. No selection. No encoding gracefulness. No mouse. No multiple buffers. Keep it brutally small.

- Cargo project skeleton (`Cargo.toml`, `src/main.rs`).
- CLI: `catomic [optional-file]`.
- Terminal bootstrap (once):
  - Enter raw mode + alternate screen.
  - Basic restore/cleanup on normal exit and panic (use `std::panic::set_hook` + `Drop` guard or similar).
  - Read initial size if needed; **ignore** resize events for Phase 0.
- **Define the `Buffer` interface right now**, before writing the loop. Use a shape that works with trait objects and multiple impls later:

  ```rust
  pub struct Cursor { pub row: usize, pub col: usize }

  pub struct LineView {
      // For v0: String is simplest. Avoid handing out huge borrowed slices across the trait.
      pub content: String,
  }

  trait Buffer {
      fn line_count(&self) -> usize;
      fn line(&self, row: usize) -> Option<Cow<'_, str>>;
      fn visible_lines(&self, start: usize, height: usize) -> Vec<LineView>;
      fn cursor(&self) -> Cursor;

      fn insert_char(&mut self, ch: char);
      fn delete_back(&mut self);
      fn delete_forward(&mut self);

      fn save_text(&self) -> String;   // fine for v0; streaming later if needed
      // load, move_cursor primitives, etc. as needed for Phase 0
  }
  ```

  - **v0 implementation**: `SimpleBuffer { lines: Vec<String>, cursor: Cursor }`. It must be good enough that Phases 1A–1C can swap only the concrete type.
  - **col semantics (v0)**: char index within the line (0-based Unicode scalar value index). Treat as "ASCII-ish UTF-8" for now. We are **not** doing grapheme clusters or terminal display width yet. Write this down so future self doesn't quietly assume it's "solved". Later phases must explicitly decide byte vs scalar vs grapheme vs wcwidth before touching selection, search, or rendering of complex text.
- The **one blessed goblin loop** (keep it in one place, keep it obvious):

  ```rust
  // pseudocode — keep the whole thing boring
  let mut buffer: impl Buffer = SimpleBuffer::new(...);
  loop {
      let height = ...; // from initial size or 24
      render_visible(&buffer, /* start row */ 0, height); // uses visible_lines + cursor
      match read_key_event() {
          // arrows -> move_cursor primitives on buffer
          Char(c) if c == '\n' || c == '\r' => buffer.insert_char('\n'),
          Char(c) if !c.is_control() => buffer.insert_char(c),
          Backspace => buffer.delete_back(),
          DeleteKey => buffer.delete_forward(),
          Ctrl('s') => { let _ = save(&buffer); },
          Ctrl('q') => break,
          // Ctrl+C/V stubbed/minimal here (see realities)
          _ => {}
      }
  }
  ```

- Minimal render: use `buffer.visible_lines(start, height)` + `buffer.cursor()` to position. No status bar, no line numbers, no frills. Just the text area. (This API shape is intentional so we don't have to change render much when we get a real piece table.)
- File I/O (dead simple):
  - On start with filename: `std::fs::read_to_string` → `buffer.load(...)` (split on '\n').
  - Save (`Ctrl+S`): `buffer.save_text()` → write to the filename (or prompt for name? keep ultra-simple: require a name or use "untitled.txt").
  - No "new file" special handling beyond starting empty.
- Cursor movement (via buffer methods): arrow keys + clamping. col is char index (see decision above).
- Insert/delete must keep the cursor correct.
- Encoding: assume UTF-8. `?` or `.unwrap()` or lossy is acceptable for v0. No graceful degradation yet.
- Quit on `Ctrl+Q` — hard exit, no questions.

**Exit criteria**:
- Run `catomic foo.txt`, type stuff (letters, Enter for new lines), move with arrows, backspace/delete, `Ctrl+S` to save, `Ctrl+Q` to leave.
- The saved file matches what you typed (modulo trivial newline details).
- The entire editing core lives in one obvious loop + the Buffer abstraction.
- Codebase is tiny and not scary to throw away pieces of.

**Phase 0 acceptance tests (per Measurement / Test Discipline)**:
- Golden test: open foo.txt, simulate insert/delete/newline/save, compare resulting file exactly.
- PTY/panic test: raw mode cleanup (panic or normal exit) does not leave the terminal in a broken state.
- Perf target: keypress to render < 16ms on small files (measured).
- Manual UX: cursor feels responsive and correct on simple ASCII input.

**Important**: Do the `Buffer` interface first. Write the goblin loop against the interface. The concrete `Vec<String>` thing is just the first implementation behind it. This is the only way Phases 1A–1C won't become surgery.

### Phase 1A — Piece Table Core (correctness-first; queries may scan; no undo, no line index)

**Tightened definition (2026-06-21)**: Phase 1A removes a prior contradiction ("piece table core, no line index" vs requiring O(1) line ops). Phase 1A is strictly correctness-first PieceTable storage + edits behind the stable `Buffer` trait. Phase 1B is the separate optimization + index phase.

Allowed (may be slow, scans ok):
- `line_count()`, `line(row)`, `visible_lines()`
- cursor row/col <-> internal offset mapping (scan to find char position on row)

Forbidden in 1A:
- undo/redo (1C)
- line index / incremental cursor mapping accel (1B)
- scrolling / viewport / UI changes
- Project / LLM / config / any new subsystems
- Full-file clones or hot-path quadratic work beyond what's needed for correctness

**Internal coordinates decision (do before coding)**:
- Public API (and `Cursor`): `row` (line) + `col` (Unicode scalar / char count within line). Matches Phase 0 SimpleBuffer.
- Piece table internal: byte offsets only.
  ```rust
  enum Source { Original, Add }
  struct Piece {
      source: Source,
      start: usize, // byte offset into the source String
      len: usize,   // byte length
  }
  ```
- Invariant: `Piece` ranges are always on UTF-8 character boundaries. Never split a multi-byte char. (Conversion from public (row, char-col) to byte offset scans pieces + counts scalars within affected lines. Slow ok for 1A.)
- `to_string()` and `line()` must reconstruct correctly.

**API requirements** (add before heavy impl):
- `PieceTable::new() -> Self`
- `PieceTable::from_text(text: &str) -> Self`

**Pre-work / oracle hygiene**:
- `SimpleBuffer::from_text` cursor position changed from "EOF" to `(0, 0)` (matches `new()` and "open a file, start at top" editor convention). Update all golden tests and comments that asserted the old EOF behavior before using SimpleBuffer as the reference oracle for PieceTable parity tests. Otherwise the oracle teaches bad cursor religion to the new impl.

**Implementation order (small coherent tasks)**:
1. PieceTable storage only (Source, Piece, PieceTable struct with original/add/pieces/cursor; from_text + to_string; line_count/line/visible_lines/cursor as scans; no edits yet). Tests for empty/single-line/trailing-nl/CRLF parity with SimpleBuffer.
2. insert_char + insert_newline only.
3. delete_back + delete_forward + movement.
4. Wire App (and/or test paths) to use `PieceTable::from_text` (or a build flag / test dual-run). Keep goblin loop + render untouched.
5. Property tests: same edit script run against SimpleBuffer vs PieceTable (or vs naive String model) must match on to_string / lines / cursor / roundtrips.

The goblin loop + render must continue to work with zero (or one-line) changes when you swap `SimpleBuffer` for the piece table impl. Keep all public trait methods stable.

Focus only on the data structure and basic edit operations. **No undo history yet.** (That sentence is how projects get haunted.)

Document the col model (char index / scalar within line for this subphase; internal bytes).

**Exit criteria**:
- PieceTable can replace SimpleBuffer and the editor still types/saves/moves correctly on real usage.
- PieceTable passes **the exact same golden tests** that SimpleBuffer passes (when tests are pointed at it).
- Property tests vs dumb String model (or cross-impl parity) pass.
- No line index code, no undo code, no scrolling/UI changes landed.

**Phase 1A acceptance tests**:
- Golden roundtrips (basic edits, delete+join, trailing newline preservation, CRLF normalization) using PieceTable.
- Property / script tests: random (or seeded) sequences of insert/newline/delete/move produce identical observable state (`to_string`, `line(i)`, `cursor`, `lines()`) as SimpleBuffer (the oracle) and/or a naive `String` model.
- from_text(empty), single-line, trailing \n, \r\n inputs produce byte-identical to_string() to SimpleBuffer.

### Phase 1B — Line Index + Cursor Mapping (optimization phase)

- Build (or integrate) a line index on top of / inside the piece table so `line_count`, `line(row)`, and cursor <-> offset conversions are fast and correct.
- Proper maintenance of row/col when inserting and deleting (including across piece boundaries).
- Cursor movement methods (or helpers) that respect the chosen col semantics.
- This is the hard part that makes "big file" and "move around" actually work.
- Still no undo.

**Exit criteria**:
- Editing and moving the cursor on 100k+ line files feels instant.
- All `line(i)` and visible window queries are fast.
- Col arithmetic is consistent and documented (still "char index" unless you explicitly changed the decision).

### Phase 1C — Undo/Redo

- Add undo/redo on top of the now-working piece table + line index.
- **Do not** store previous full text (tempting, easy, cursed). The right shape is an operation log that records *inverse edits* against the piece table (e.g. the inverse of an insert is a delete of that range), probably grouped into transactions later.
- Undo must correctly restore cursor position.
- Wire `Ctrl+Z` / `Ctrl+Y` (or `Ctrl+Shift+Z`) in the goblin loop.
- Keep the loop and render unchanged.

**Exit criteria**:
- Solid tests: insert/delete sequences, undo, redo, undo after save, mixed operations.
- Swapping the full Phase 1 buffer did not require surgery on the editor core.
- You have a real piece table + line index + undo, not "accidentally wrote VS Code's text buffer in a trenchcoat."
- First serious attempt at Ctrl+C/V (bracketed paste + clipboard) can now be done on a real buffer.

**Phase 1 acceptance tests (1A+1B+1C)**:
- Property tests + fuzzing for piece table and undo/redo: random edits + undo/redo sequences must match dumb String model (non-negotiable).
- PTY smoke + golden tests covering undo across saves.

### Phase 2 — Robustness, Files, Big File Discipline

- File watching (`notify`):
  - On external change: if clean → reload (with message)
  - If dirty → warning + choice (reload / keep / save as conflict copy)
  - Optional small diff preview
- Big file mode (explicit tiers):
  - 10 MB: target is smooth (everything should just work).
  - 100 MB: target is usable (navigation + local edits should be fine, some features limited).
  - 1 GB: "maybe" — read + local edits with strict limits (no full highlighting, no global operations, large-file mode forced).
  - Heuristic detection on open (size + line count)
  - Disable full syntax highlighting for anything beyond small files
  - Lazy line indexing (already provided by Phase 1B)
  - "Large file mode" banner + limited operations (no syntax, throttled linters, etc.)
- Rendering optimizations:
  - Only redraw dirty regions where easy
  - Avoid full screen clear every frame
  - Virtual scrolling / viewport only
- Multiple buffers (simple):
  - Open several files
  - Basic next/prev buffer or list
  - (Tabs or splits come much later)
- Better dirty tracking + crash safety (write .catnap? simple version)
- Improve status bar (mode hints, encoding, file size)
- Error messages that don't suck (and are occasionally catty)

**Exit criteria**:
- Survives external edits from another program or `echo >> file`.
- 10 MB files: smooth experience.
- 100 MB files: perfectly usable for normal editing.
- 1 GB files: can at least open + navigate + do small local edits without blowing up (with large-file limits applied).
- No data loss paths left in basic usage.

**Phase 2 acceptance tests**:
- Benchmarks: verify 10MB smooth, 100MB usable, 1GB limited (time + memory ceilings documented).
- External edit tests using temp files + PTY smoke.
- File watch golden tests (clean reload, dirty conflict paths).
- Current implementation/evidence matrix: [`docs/phase-2-acceptance.md`](docs/phase-2-acceptance.md).

### Phase 3 — Comfort & Search Basics ([progress](docs/progress/phase-3-progress.md))

- Incremental search (`Ctrl+F`):
  - Live highlight matches
  - Next / prev
  - Escape to exit
- Goto line (`Ctrl+G`)
- Basic command surface (even if just a prompt for now)
- Selection expansion (double click words, etc.)
- Mouse support (Phase 3 or 4):
  - Click to move cursor
  - Drag to select
- Cut/copy/paste of selections (not just whole lines)

**Exit criteria**:
- Core comfort features work and feel good.
- Acceptance tests defined and passing per the Measurement / Test Discipline (golden for search/replace flows, PTY for Ctrl+F etc., perf for search on medium files); current evidence: [`docs/phase-3-acceptance.md`](docs/phase-3-acceptance.md).

### Phase 4 — Markdown & Light Syntax ([progress](docs/progress/phase-4-progress.md))

- Syntax highlighting (minimal at first):
  - A few built-in languages or extension-based simple rules
  - Only highlight visible region for big files
- Native markdown handling:
  - Detect .md
  - Style headings (bold/underline/color via ANSI)
  - Lists, blockquotes, inline code
  - Fenced code blocks (maybe different bg or just no highlight inside)
- Preview mode:
  - Key to toggle rendered view (use pulldown-cmark + renderer)
  - Later: split view if width > 120 cols or user requests
- Line numbers (toggleable, off by default or on for code)
- Whitespace indicators (optional); acceptance: [`docs/phase-4-acceptance.md`](docs/phase-4-acceptance.md)

### Phase 5 — Tooling (Linters + Autocomplete) ([progress](docs/progress/phase-5-progress.md))

**Project / Code Mode features are behind the `Capabilities` bouncer.** When `Capabilities` for Plain is active, none of the Project services here are constructed.

- Full linters, project file discovery, cross-file completion, and LSP are Project-only.
- A minimal "local, current-buffer word completion only" (no process, no index) is allowed under Plain's `local_completion` capability and may be implemented earlier or carried into Plain.

- Linter support:
  - Config file mapping extension → shell command (`{file}` placeholder)
  - Run on demand (key or command)
  - Parse common formats (`file:line:col: message`)
  - Show list, jump to error, next/prev error
  - Never run automatically on every keystroke without heavy throttling + user opt-in
- Autocomplete:
  - Local current-buffer word completion (Plain `local_completion` — no process, no index)
  - Project-aware: file path completion, keywords, snippets, etc. (requires Project capabilities)
  - Trigger on Ctrl+Space or Tab in some contexts
  - Dismiss on Esc
- Project file discovery (simple "find in dir" for open file); acceptance: [`docs/phase-5-acceptance.md`](docs/phase-5-acceptance.md)

### Phase 6 — LLM (Powerful but Caged) ([acceptance](docs/phase-6-acceptance.md), [progress](docs/progress/phase-6-progress.md))

**LLM surface is split by `Capabilities`.** In Plain mode `network_llm` is false and no network client exists. Current-file commands (`:meow`, `:bigmeow`) may only create transient network use after explicit user invocation + confirmation of endpoint/context. Repo-aware / multi-file / broker LLM (`:megameow`, `:gitmeow` etc.) require `repo_llm` (Project only). All construction is gated; nothing network-related for LLM exists in a Plain process until the user forces a confirmed current-file action.

- Instruction block parser:
  - Recognize `>>> catomic` ... `<<<` blocks
  - Also support selection + explicit instruction
- LLM action entry (command palette or dedicated key)
- Safe context collection:
  - Only selection or current block by default
  - Clear user-visible "sending X lines to <model>"
- Backend:
  - OpenAI-compatible HTTP (configurable base URL + key)
  - Local-first friendly
- Output handling (strong preference order):
  1. Unified diff / patch
  2. Full replacement of a marked region
- Always: show preview diff, require confirmation to apply
- Every applied change is undoable
- Safety rails:
  - Max context size hard limit
  - Never send dotfiles or obvious secrets unless explicitly allowed
  - Telemetry-free by default; remote usage is obvious
- Basic "refactor", "explain", "write tests" actions

**LLM Context Broker & Commands** (the model does not get the whole repo; it works through a controlled interface):

- `:gitmeow` (and family) must:
  - Detect repo root
  - Capture git status
  - Capture current branch + base branch
  - Capture `git diff --stat`
  - Capture `git diff --name-only`
  - Build a file tree / map summary
  - Optionally include recent commits
  - Expose read-only retrieval commands to the model (the "clanker"):
    - list files
    - read file (range / offset+limit)
    - grep / ripgrep
    - show diff for file
    - (later) show symbols
    - (later, with confirmation) run tests
- The model gets a **context budget**, not the whole repo. It can request more context through Catomic’s broker.
- Every proposed edit still returns as a patch, gets previewed in the UI, and applies **only after explicit user confirmation**.
- Never silent writes.

**Suggested modes / commands** (invoked as colon commands or palette entries):

Plain / Text Mode scope (current-file, explicitly invoked):
- `:meow` — current selection / block only.
- `:bigmeow` — current file.

Project / Code Mode scope (repo-aware, opt-in):
- `:gitmeow` / `:megameow` — git branch / repo-aware (uses the gitmeow broker machinery).
- `:feralmeow` — dangerous / wide mode: can inspect wider repo and propose multi-file patches. Still **no silent writes**, still preview+confirm.

All still respect the Plain/Project rules: Project features are lazy/opt-in and must not affect Plain mode startup or latency.

**Critical safety rail for gitmeow**:
- Before the LLM call, snapshot: HEAD commit, current branch name, and dirty state (index + working tree).
- If any tracked or relevant files change on disk while the model is "thinking", **refuse blind apply**.
- Force a rebase/review of the patch against the new reality.
- This prevents "clanker time-travel bugs" (the model proposes edits against a world that no longer exists).

All LLM paths must respect the Measurement / Test Discipline (golden tests for patch application, PTY tests for preview/confirm flow, etc.).

Phase 6 acceptance is complete. The shipped contract is current-buffer edits
and repo-brokered single-file edits only. `:feralmeow` was a suggested wide
mode, not an exit requirement, and remains deliberately unimplemented because
multi-file apply needs a separate safety design.

### Phase 7 — Config, Hooks & First Extensibility

- TOML config with good defaults (no config file required)
- Per-language settings (linters, tab size, etc.)
- Keybinding configuration (simple overrides)
- Hooks: on-save, on-open, before-llm, etc. (external commands first)
- External command execution from within editor (insert output, etc.)
- Plugin system **only after** everything above is solid:
  - Start with "external commands + hooks"
  - Scripting / editor commands / overlays much later

Phase 7 acceptance is complete. Typed TOML, per-language settings, keybinding
overrides, guarded external commands, and ordered lifecycle hooks are verified
in `docs/phase-7-acceptance.md`. External commands plus hooks are the first
extensibility surface; scripting, a plugin ABI, editor-command APIs, and overlays
remain explicitly deferred rather than Phase 7 exit requirements.

### Phase 8 — Cat Features & Polish (ongoing delight)

- Cat-themed status messages (toggleable, default on but tasteful)
- `:meow` command (or just fun easter egg)
- Panic handler that prints helpful + funny cat messages
- `.catnap` autosave / recovery files (simple, opt-in or safe default)
- Fun but never annoying: deterministic badge/notices only; no random purr timer

Phase 8 acceptance is complete. The default-on badge is toggleable, `:meow`
retains its useful confirmed workflow, panic output is helpful, and opt-in
`.catnap` recovery is bounded, private, preview-first, drift-safe, undoable, and
removed only after a successful explicit save. See `docs/phase-8-acceptance.md`.

## Non-Goals for v1 / Early Versions

- First-class Windows/macOS support (Linux terminal is the target)
- Full tree-sitter + structural editing early
- Complete LSP client
- Vim keybindings (explicitly not a goal)
- Heavy theming engine
- Mouse as primary input
- Distributed / collaborative editing
- Web UI or Tauri/Electron "terminal" wrapper

## Development Practices

- **Buffer interface first.** Define the trait/API and write the goblin loop against it *before* investing in fancy UI or the final data structure. This prevents "ripping organs out with pliers" when doing Phases 1A–1C.
- Red/green for buffer logic (tests first when possible). See "Measurement / Test Discipline".
- Profile before optimizing (especially redraw and piece table access).
- Always have a way to open huge synthetic files for testing (see perf targets per phase).
- Keep the main (goblin) loop extremely boring and in one obvious place.
- Every LLM or external action must be previewable + undoable.
- **Plain/Project mode discipline**: All feature construction is gated by `Capabilities` (see the Capabilities section). In Plain mode, Project services (`linters`, `repo_scan`, `repo_llm`, `lsp`, `network_llm`) must not be constructed. "Lazy but the factory exists" fails the test. The same core powers both modes, and switching must produce the correct construction profile. See Product Modes, Capabilities, and Mode Acceptance Tests.
- No phase is complete until its defined acceptance tests (unit + golden + PTY + perf + manual checklist) pass.
- When in doubt, make the safe / obvious choice for the user.
- Own the terminal paste / shortcut quirks explicitly (see "Terminal Realities" section). Don't pretend the OS/terminal will be cooperative.

## Measurement / Test Discipline

Every phase must define:
- Unit tests: pure logic correctness.
- Golden tests: input file + edit sequence => exact output file.
- PTY smoke tests: launch editor, send keys, assert screen/output/save.
- Perf targets: max time / memory for synthetic files.
- Manual UX checklist: things that must feel correct but are hard to automate.

No phase is complete until its acceptance tests pass.

### Mode Acceptance Tests

**Text Mode (Plain) passes only if:**

- startup is near-instant
- `Capabilities` for Plain mode has `linters`, `lsp`, `repo_scan`, `repo_llm`, and `network_llm` all `false`
- No Project services are *constructed* at all (not just unused or lazily skipped). A `LinterManager`, `ProjectIndex`, `LSPClient`, `RepoLLMBroker`, or network LLM client must not exist in the process.
- No repo scanning or background indexing occurs (by construction).
- Network is impossible: no `reqwest` client or equivalent is created for LLM use, unless the user explicitly invokes `:meow` / `:bigmeow` *and* confirms the endpoint + context at call time.
- Local completion is only current-buffer word completion; no background process and no project index is ever built.
- Markdown/text editing stays clean and quiet.
- Disabling Project features or switching back to Plain removes the services from existence (full return to Plain behavior).

**Code Mode (Project) passes only if:**

- `Capabilities` correctly enables the requested Project flags.
- Project services are constructed only when their capability is enabled, and only on explicit user action or first use (lazy within the allowed mode).
- Diagnostics and other Project features never block typing.
- LLM repo context (when `repo_llm` is true) is always brokered and budgeted.
- All edits (including LLM-proposed ones) remain previewable + undoable.
- Switching to Plain immediately yields a process whose construction and capabilities match a fresh Plain start.

Concrete examples:

**Phase 0:**

- Golden test: open foo.txt, simulate insert/delete/newline/save, compare file.
- Panic test: raw mode cleanup doesn’t leave terminal cursed.
- Tiny perf target: keypress/render under, say, 16ms on small files.

**Phase 1:**

- Property tests for piece table: random edits must match a dumb String model.
- Undo/redo fuzzing. This is non-negotiable. Text buffers are bug farms.

**Phase 2:**

- Benchmarks: 10MB smooth, 100MB usable, 1GB limited.
- Memory ceiling per file size.
- External edit tests with temp files.

## Milestones (high level)

1. Phase 0 complete → "The goblin loop + Buffer interface exist. I can type, move, save, and quit without the program exploding."
2. Phases 1A + 1B + 1C complete → "Real piece table + line index + undo are in place behind the stable interface. This feels like a proper editor core (not a trenchcoat)."
3. Phase 2 complete → "I trust it with my real work and big logs (10MB smooth, 100MB usable, 1GB with limits)"
4. Phase 4 complete → "I reach for it for markdown"
5. Phase 6 complete → "The LLM stuff is actually useful and safe (Plain :meow/:bigmeow only after explicit invoke+confirm with no pre-constructed network; Project :megameow with broker + budget; `Capabilities` construction gate enforced in tests; preview+undo always)"

Update this file as decisions are made or phases complete. Add concrete issues or PRs under each phase when work starts.

---

**Current status** (2026-07):
- Phases 0 through 8 complete; acceptance records are in `docs/`.
- Phase 8 complete: toggleable cat status, panic-safe messaging, and opt-in
  bounded `.catnap` recovery are accepted.
- Phase 7 complete: typed TOML, per-language settings, keybinding overrides,
  guarded external commands, ordered lifecycle hooks, preview, and one-step undo
  are accepted.
- Phase 6 complete: explicit `:meow`/`:bigmeow`, Project-only
  `:gitmeow`/`:megameow`, bounded broker context, transient confirmed network,
  drift-safe preview, and one-step undo are accepted.
- Post-v0.1 core usability: interactive Open/New/Close and Save As, standard and
  word navigation, Find/Replace, automatic indentation, built-in shortcut help,
  grapheme/cell-aware terminal editing, bounded soft wrap, and UTF-8 BOM plus
  newline-style preservation are implemented and covered by regression tests.
- Phase 1A complete: PieceTable behind Buffer, parity correct, app using it.
- Phase 1B-a complete:
  - Real LineIndex (in buffer/line_index.rs) with rebuild bridge.
  - Queries (line_count, line, visible_lines, to_string, lines) use index + slice_to_string (no full logical_text materialization in render path).
  - cursor_byte_offset present; seeded random + multibyte (é猫🙂 etc) parity tests (incl. boundary delete/backspace/nl joins).
  - PT golden/perf smokes added (previously only SimpleBuffer).
  - Coalescing wired + tested; module split done.
- Phase 1B-b complete:
  - row_for_byte uses binary search (was linear).
  - piece_starts prefix + binary find_piece_for_byte: slice_to_string/split_point no longer head-scan for normal spans; bounded lookup.
  - non-newline edits use adjust_index_for_simple_delta().
  - newline insert and newline-join deletes use incremental adjust (no full rebuild).
  - App / goblin has only required undo key bindings.
- Phase 1C complete:
  - Undo/redo using piece-level Transactions (CursorState + PieceEdit Insert/Delete of Vec<Piece> descriptors). No full-text snapshots.
  - Redo of insert reuses stored pieces (no re-append to add). Save is not undoable; undo only affects buffer.
  - No-op edits produce no history entries. History apply suppresses recording.
  - New edit after undo clears redo.
  - Keys: Ctrl+Z (undo), Ctrl+Y / Ctrl+Shift+Z (redo) wired (and precedence fixed).
  - Undo tests (insert/delete/newline/join/multibyte/no-op + reuse + clear-redo).
  - Added golden undo-across-save test (actual fs write + buffer undo + disk unchanged assertion).
- PTY harness now includes root integration smokes (`tests/pty_smoke.rs`) that drive the real binary through save/undo/save, external-edit confirmation/reload, Ctrl+F, and multiple-file buffer switching flows, followed by clean quit; broader terminal coverage remains intentionally narrow.
- No LLM/Project in Plain.
- Phase 2-a (foundation) complete:
  - atomic_write_string helper: same-dir temp, create_new, full write+flush+sync_all, rename, best-effort parent dir fsync on Unix; existing Unix mode bits are preserved and temp files are cleaned up on error.
  - FileState { path: Option<PathBuf>, dirty: bool } replaces raw Option<String>.
  - App saves through the atomic path; untitled Ctrl+S opens Save As for an explicit target.
  - Dirty=false after open (existing or missing-file); =true on insert/newline/delete/undo/redo keys; =false after successful save.
  - No dirty on movement/render.
  - Dirty flag is conservative (Phase 2-a historical); exact save-point token tracking landed in Phase 2-j (see below).
  - Tests: atomic unit (bytes/overwrite/no-temp), app/file-state lifecycle via keys, golden that exercises atomic helper for exact save content.
  - Existing undo-across-save golden tests still pass. All Phase 1 tests green.

- Completed Phase 2 progress (detailed notes for 2-b through 2-p) is archived in `docs/progress/phase-2-progress.md`.
- Phase 2-a (foundation) complete: see archive for b–p details; active goals and exit criteria remain in the Phase 2 section above.

- Detailed completed Phase 2-r through 2-ae notes are archived in `docs/progress/phase-2-progress.md`.

Current limitations after Phase 2 acceptance:
- Small/Large files still use a full-read PieceTable. Huge/Extreme files use editable file-backed PieceTable pages; only the active page and pages with edit history are retained. Page boundaries remain anchored to source byte ranges during the session and rebalance after reload/reopen; a single giant logical line can still make one page span the file. No mmap or rope rewrite.
- watcher signals are runtime hints only; App-owned best-effort; runtime checks watcher once per loop via helper (try_recv inside check_file_watcher_once only); Unchanged/NoPath from watcher clear stale pending_reload when armed, otherwise fully ignored (suppress self-save noise);
- clean Modified/Deleted watcher observations auto-reload by default; `[files] auto_reload = false` restores confirmation-only behavior. Dirty buffers always retain the exact-snapshot Ctrl+R confirmation path;
- content is read only after a fresh clean Modified observation when auto-reload is enabled,
  or after exact Ctrl+R confirmation; raw watcher signals never supply content;
- metadata-only external detection uses len/mtime plus Unix device/inode/ctime via observe_external_file / capture / compare; same-size/same-mtime path replacement is detected without hashing, though a change that preserves every available metadata field can still evade detection;
- default test suite uses deterministic queued-signal seams only (TestStub/inject + replace_file_watcher_for_test); live OS notify smoke is ignored/manual and must not be required for CI;
- big-file tiers/perf: Huge/Extreme pages are editable. Cross-page undo/redo is global, Ctrl+S streams edited pages over untouched stable descriptor ranges, and explicit Ctrl+F searches the descriptor plus unsaved overlays across page seams. Descriptor drift fails reads/save closed. Performance budgets remain advisory.
- rendering repaints and clears each viewport row without a terminal-wide clear; it does not yet retain row state for dirty-row-only redraws.
- Detailed completed Phase 2-af through 2-ax notes are archived in `docs/progress/phase-2-progress.md`.
- Detailed completed Phase 2-ay through 2-br notes are archived in `docs/progress/phase-2-progress.md`.

Phase 2 acceptance status (post 2-ca):
- Complete: automatic clean reload, dirty-buffer protection, configurable editable Huge/Extreme pages, cross-page history, whole-document atomic save, and whole-file Ctrl+F are implemented and verified.
- Simple multiple-buffer foundations are complete: every positional CLI path opens in argument order; Alt+PageDown/PageUp switches a state-preserving ring; the status shows the active position; and quit checks dirty inactive buffers. Unit and real PTY coverage exercise the path.
- The 2026-07-07 phase split is recorded; editable Small/Large PieceTable opens still fully materialize content. Huge/Extreme paged opens scan the active configured source page and retain edited pages; whole-file search starts only after explicit Ctrl+F invocation.
- Keep manual large-file tests ignored; do not add or enable default 10/100 MiB or 1 GiB tests.
- Do not enforce thresholds yet; budgets remain advisory and must not become pass/fail gates in this or the immediate next pass.

External-file safety current state after 2-ae:
- manual Ctrl+R status/reload exists and is the confirmation path (first press arms, second performs if snapshot matches; drift re-arms).
- save conflict guard exists (first S refuses on Modified/Deleted/Unknown against live snapshot; second forces only on exact match).
- watcher exists and is App-owned best-effort (gated by caps.file_watch + watchable parent path; constructed in Plain on new(path) and after first successful save from untitled).
- runtime checks watcher once per loop (check_file_watcher_once_and_render near top of run); try_recv only inside that helper.
- signals are hints only; source of truth is always a fresh metadata observation from `observe_external_file`.
- clean Modified/Deleted auto-reload when enabled; dirty/config-disabled cases arm pending like first Ctrl+R. Unchanged/NoPath suppress noise or clear stale pending only.
- default-on auto-reload reads content only after a fresh clean Modified observation; dirty/config-disabled cases retain confirmed Ctrl+R reload.
- metadata-only detection now catches same-size/same-mtime path replacement on Unix through device/inode/ctime; no content hash is performed.
- tests: deterministic seams cover arming + manual follow-up; live smoke is #[ignore] and manual.

Phase 2A external-file safety acceptance checklist (concrete):
- open file captures disk snapshot (Present or explicit Absent).
- successful save updates disk_snapshot + clears dirty + clears pending_reload/quit/save-conflict.
- external modified/deleted is detected by manual Ctrl+R (arms with correct message).
- save conflict: first press refuses (keeps dirty, sets msg, records pending); second forces only on identical observation.
- Ctrl+R reload confirmation handles Modified (reload content + snapshot + "Reloaded") and Deleted (clear buffer + Absent + cleared msg).
- watcher signals trigger a fresh metadata observation; clean buffers auto-reload by default, while dirty or config-disabled buffers only arm Ctrl+R confirmation.
- watcher self-save noise suppressed (Unchanged/NoPath ignored when no pending).
- stale watcher pending clears when disk resolves (Unchanged/NoPath with prior pending surfaces msg + clears).
- tests cover deterministic seams (apply + queued + render) + manual Ctrl+R follow-ups.
- live notify smoke is ignored/manual; never runs in default cargo test.
