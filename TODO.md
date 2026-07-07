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
- File watching (when `file_watch`) is Plain-allowed. App owns a best-effort gated FileWatcher (notify) when a watchable path exists; runtime loop checks once per iteration via a non-blocking helper (signals are hints only; fresh observe_external_file + apply_check_observation is truth). No auto-reload; Modified/Deleted only arm manual confirmation (Ctrl+R). Unchanged/NoPath suppress self-save noise (or clear stale pending). Deterministic seams in tests; live smoke is ignored/manual.
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

### Phase 3 — Comfort & Search Basics

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
- Acceptance tests defined and passing per the Measurement / Test Discipline (golden for search/replace flows, PTY for Ctrl+F etc., perf for search on medium files).

### Phase 4 — Markdown & Light Syntax

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
- Whitespace indicators (optional)

### Phase 5 — Tooling (Linters + Autocomplete)

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
- Project file discovery (simple "find in dir" for open file)

### Phase 6 — LLM (Powerful but Caged)

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

### Phase 7 — Config, Hooks & First Extensibility

- TOML config with good defaults (no config file required)
- Per-language settings (linters, tab size, etc.)
- Keybinding configuration (simple overrides)
- Hooks: on-save, on-open, before-llm, etc. (external commands first)
- External command execution from within editor (insert output, etc.)
- Plugin system **only after** everything above is solid:
  - Start with "external commands + hooks"
  - Scripting / editor commands / overlays much later

### Phase 8 — Cat Features & Polish (ongoing delight)

- Cat-themed status messages (toggleable, default on but tasteful)
- `:meow` command (or just fun easter egg)
- Panic handler that prints helpful + funny cat messages
- `.catnap` autosave / recovery files (simple, opt-in or safe default)
- Fun but never annoying: occasional "purrs saved" or similar

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

**Current status** (2026-06):
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
- PTY harness for full key-driven (Ctrl+S then Ctrl+Z) save/undo is still stub-only (see src/tests/pty.rs); semantics covered at golden + unit level.
- No LLM/Project in Plain.
- Phase 2-a (foundation) complete:
  - atomic_write_string helper: same-dir temp, create_new, full write+flush+sync_all, rename, best-effort parent dir fsync on Unix; temp cleanup on error.
  - FileState { path: Option<PathBuf>, dirty: bool } replaces raw Option<String>.
  - App saves exclusively through atomic_write_string (Ctrl+S); remembers untitled.txt when needed.
  - Dirty=false after open (existing or missing-file); =true on insert/newline/delete/undo/redo keys; =false after successful save.
  - No dirty on movement/render.
  - Dirty flag is conservative (Phase 2-a historical); exact save-point token tracking landed in Phase 2-j (see below).
  - Tests: atomic unit (bytes/overwrite/no-temp), app/file-state lifecycle via keys, golden that exercises atomic helper for exact save content.
  - Existing undo-across-save golden tests still pass. All Phase 1 tests green.

- Completed Phase 2 progress (detailed notes for 2-b through 2-p) is archived in `docs/progress/phase-2-progress.md`.
- Phase 2-a (foundation) complete: see archive for b–p details; active goals and exit criteria remain in the Phase 2 section above.

Key unresolved limitations that still matter:
- watcher signals are hints only (runtime loop polls once/iter via helper); no auto-reload,
- no live OS notify integration tests (deterministic seams only),
- metadata-only external detection,
- same-length/same-mtime overwrite limitation,
- big-file tiers/perf still unfinished,
- rendering still full clear/no partial redraw unless already changed.

- Phase 2-r (narrow pass): manual external-file status check (Ctrl+R; message only via existing snapshot; no reload/mutation).
- Phase 2-s (narrow pass): manual reload via two-step Ctrl+R using status foundation (Modified reloads content, Deleted clears buffer; drift re-arms; no watcher).
- Phase 2-t (narrow harden): newline clear fix, read-fail no-lie, logic extracted to reload.rs; no watcher.
- Phase 2-u (narrow cleanup): observe single-capture via pure helper; one ExternalFileObservation for Ctrl+R status+arm; handle reuses obs (no double observe); tightened generic newline regression; no watcher/background/polling/hash/full-read/new deps.
- Phase 2-v (narrow cleanup): genuine single-capture observe_external_file (preserve Result, no fs::metadata fallback on error); live_snapshot=None + Unknown on hard meta error; NotFound still Absent; no watcher/background/polling/hash/full-read/new deps.

- Phase 2-w (narrow foundation): added explicit `file_watch: bool` cap (Plain+Project both true; is_plain_safe unchanged); FileWatcher::new now takes &Capabilities (returns Option); fixed stale "must not Plain" contract in watcher header + minimal sketches in TODO/0001. No notify dep, no impl, no threads, no polling, no App ctor, no reload changes.

- Phase 2-x (narrow pass): real notify-backed FileWatcher (gated ctor returning Result<Option>, FileWatchSignal, try_recv, parent-dir watch + lexical filter, pure helpers). Deterministic helper tests only (no live fs events, no App wiring). notify 8.x added with justification; App, reload, event loop untouched.

- Phase 2-y (narrow cleanup): extracted pure helpers to file::watch_path (pub(crate)); replaced absolutize with real lexical Component normalize (., .., root safe); added parent-after-norm + rename-hint tests/comments. No App wiring, no new deps, no live events.

- Phase 2-z (narrow pass): App now owns gated FileWatcher (best-effort construct on new(path) and after successful first save from untitled). Lifecycle via app/watch.rs refresh/clear. No signal consumption, no try_recv in runtime, no reload behavior. Focused non-live tests only.

- Phase 2-aa (narrow pass): non-runtime signal helper seams only (apply_file_watch_signal + check_file_watcher_once in app/watch). try_recv only inside the drain helper. Signals are hints; always fresh observe + apply_check_observation (arms like first Ctrl+R). No runtime wiring, no auto-reload, no behavior change to manual paths. Deterministic tests only.

- Phase 2-ab (broader pass): wired check_file_watcher_once_and_render into App::run (once per iteration, near top before poll); signals consumed only via the helper as hints (never auto-reload content, never direct try_recv in mod.rs, never from handle/save/reload/render). Deterministic no-watcher + no-signal seam tests via the render helper (arm-via-OS left for integration). Split watcher_signal tests out of lifecycle; updated stale comments. No new deps/threads/async. All required tests green.

- Phase 2-ac (broader pass): polished runtime watcher signal behavior (apply_file_watch_signal now returns bool for visible outcome; watcher Changed/Deleted + Unchanged/NoPath observations are ignored to avoid self-save noise — no message overwrite, no arm, no render; Error/Modified/Deleted/Unknown remain visible). Added cfg(test)-only queued-signal seam (tiny InnerWatcher TestStub + inject_signal/new_for_test in FileWatcher; replace helper in watch; no public API, real new unchanged, no live OS). Added deterministic queued-signal + render tests (watcher_runtime.rs split for size hygiene). Manual Ctrl+R and save-conflict semantics untouched. Stale comments cleaned. Key unresolved limitations noted (see below). All mandated tests + full suite green; rustfmt --check on touched; git diff --check clean.

- Phase 2-ad (broader pass): watcher Unchanged/NoPath now clear a stale pending_reload (if present) and surface the corresponding status message, returning visible so the loop renders once; when no pending they continue to be fully ignored (no overwrite). Added required deterministic stale-pending cleanup tests (watcher runtime + direct apply seams). Tightened one-call-one-signal test to prove two visible queued signals are each consumed by separate calls with observable true + render. Split watcher.rs tests to watcher_tests.rs (main file <300). Fixed stale "not consumed / not yet consumed" wording in watcher.rs with truthful description of current App-owned + once-per-loop helper model. Added optional ignored live smoke. All mandated tests + full suite green; rustfmt --check; git diff --check clean. (See final response for hashes and explicit behavior note.)

- Phase 2-ae (broader pass): close external-file/watch safety arc (test hygiene, acceptance coverage, docs). Split oversized watcher_* app tests (pending module added; each <300 lines per claim at time). Added deterministic watcher + manual Ctrl+R acceptance tests (arm then second press reloads/clears/discard-warn/re-arms). Live smoke tightened (ignore reason, bounded, skip-clean, no CI reliance). Removed stale current-state "no watcher / not consumed / non-runtime / implements none" wording (except clearly historical notes for 2-l/2-s etc). Added "External-file safety current state after 2-ae" note + concrete Phase 2A acceptance checklist. No auto-reload, no content read outside confirmed Ctrl+R, no new deps, no manual threads, no save-conflict or manual Ctrl+R behavior changes. All required tests green.

Key unresolved limitations (still current post 2-aq):
- (size classification + pre-read guardrails + Large/Huge warn + Extreme refuse now exist; manual baselines recorded + split harness exist; first visible large-file mode status marker landed; still no lazy loading, no mmap, no rope rewrite)
- watcher signals are runtime hints only; App-owned best-effort; runtime checks watcher once per loop via helper (try_recv inside check_file_watcher_once only); Unchanged/NoPath from watcher clear stale pending_reload when armed, otherwise fully ignored (suppress self-save noise);
- no auto-reload; Modified/Deleted (from watcher or Ctrl+R) only arm confirmation; second Ctrl+R performs actual reload using fresh observe + pending match (or clears for Deleted);
- no content read from watcher signal path except the existing confirmed Ctrl+R reload path;
- metadata-only external detection (len+mtime via observe_external_file / capture / compare); same-size/same-mtime overwrite limitation remains (no hash/content);
- default test suite uses deterministic queued-signal seams only (TestStub/inject + replace_file_watcher_for_test); live OS notify smoke is ignored/manual and must not be required for CI;
- big-file tiers/perf: open guardrails + metadata + split harness + manual baselines + 2026-07-07 open-path phase split + initial persistent "large-file mode" bottom status marker now exist; App open has an explicit content plan (untitled/missing empty vs present full-read), avoids the extra LF-only read-buffer clone, and LineIndex build uses std newline search; 100 MiB/1 GiB present files still full read + full materialization (no lazy storage mode); status size label is on-disk metadata only; no thresholds declared or enforced yet.
- Phase 2-af (broader pass) began Phase 2B big-file discipline foundation while closing watcher test hygiene: split watcher_pending (>400) into watcher_pending (stale cleanup only) + watcher_acceptance (<300 each); fixed false "each <300" wording via split. Added src/file/size.rs (FileSizeTier + SMALL/LARGE/HUGE consts at binary 10/100/1024 MiB; pure classify + label; file_size_bytes metadata helper). Threaded size_bytes/Optional<tier> into FileState (None for no-path/missing/deleted). App::new captures size (None for missing); save and confirmed Ctrl+R Modified update from post meta (fallback len only on meta fail after write); Deleted clears to None. Focused file_size tests (new App::new cases, save from untitled/existing, reload Modified/Deleted, failed save no-update, no side effects on snapshot/conflict). No open refusal, no lazy, no perf harness, no large-file mode, no >small allocs in default tests, no watcher/reload behavior change. All mandated tests (file::size, file::io, watcher_*, file_state::*, app::, full) green; fmt; diff--check; commits per AGENTS.

Next intended Phase 2B steps (post 2-aq):
- Use the advisory budgets + updated hotspot inventory (see docs/performance.md) to choose the next narrow implementation target.
- The 2026-07-07 phase split is recorded; after LF-only, owned-input, and newline-search reductions, `read_to_string` is now the largest measured editor-owned subphase for the synthetic no-newline 100 MiB file, while full materialization remains the larger design limitation.
- Keep manual large-file tests ignored; do not add or enable default 10/100 MiB or 1 GiB tests.
- Do not enforce thresholds yet; budgets remain advisory and must not become pass/fail gates in this or the immediate next pass.

External-file safety current state after 2-ae:
- manual Ctrl+R status/reload exists and is the confirmation path (first press arms, second performs if snapshot matches; drift re-arms).
- save conflict guard exists (first S refuses on Modified/Deleted/Unknown against live snapshot; second forces only on exact match).
- watcher exists and is App-owned best-effort (gated by caps.file_watch + watchable parent path; constructed in Plain on new(path) and after first successful save from untitled).
- runtime checks watcher once per loop (check_file_watcher_once_and_render near top of run); try_recv only inside that helper.
- signals are hints only; source of truth is always fresh metadata observation (observe_external_file) + apply_check_observation (same path used by Ctrl+R).
- Modified/Deleted arm pending (like first Ctrl+R); Unchanged/NoPath from watcher suppress noise or clear stale pending only.
- no auto-reload ever; no content read except on confirmed Ctrl+R second press.
- same-size/same-mtime limitation remains (metadata-only).
- tests: deterministic seams cover arming + manual follow-up; live smoke is #[ignore] and manual.

Phase 2A external-file safety acceptance checklist (concrete):
- open file captures disk snapshot (Present or explicit Absent).
- successful save updates disk_snapshot + clears dirty + clears pending_reload/quit/save-conflict.
- external modified/deleted is detected by manual Ctrl+R (arms with correct message).
- save conflict: first press refuses (keeps dirty, sets msg, records pending); second forces only on identical observation.
- Ctrl+R reload confirmation handles Modified (reload content + snapshot + "Reloaded") and Deleted (clear buffer + Absent + cleared msg).
- watcher signal only arms/updates status (via helper); never reloads or mutates content/dirty/snapshot.
- watcher self-save noise suppressed (Unchanged/NoPath ignored when no pending).
- stale watcher pending clears when disk resolves (Unchanged/NoPath with prior pending surfaces msg + clears).
- tests cover deterministic seams (apply + queued + render) + manual Ctrl+R follow-ups.
- live notify smoke is ignored/manual; never runs in default cargo test.

Phase 2-af (2B start) note: external-file safety arc closed by 2-ae; 2-af added pure size classification (file::size) + App/FileState size metadata bookkeeping only. No changes to snapshot/conflict/reload/watcher behavior or messages.

- Phase 2-ag (broader pass): pre-read open-size guardrails added. OpenSizeDecision + open_size_decision / open_size_warning_message / open_size_refusal_message / format_file_size pure helpers + exhaustive threshold + message + formatter tests in file::size. App::new now probes size via file_size_bytes before any read_to_string: Extreme refuses (io::Error InvalidData containing "File too large to open safely") with no read, no watcher, no App; Large/Huge proceed to read then set initial app.message warning after construction; Small unchanged. Added focused split tests (file_size_open.rs): Small existing no warning; Large (~10 MiB generated temp) warning; Extreme sparse via set_len (skip-clean if fs limit); missing = empty+None+no-warn; small invalid UTF-8 still errors. Replaced weak global-untitled size tolerance test with deterministic explicit-path first-save size update test (no contended "untitled.txt", no chdir). Clarified FileState + save.rs docs: size is metadata-first (fs::metadata); the sole allowed content-derived case is post-successful-save len fallback when stat of our own write fails (no extra read; file::size::file_size_bytes strictly metadata-only). No lazy loading, no mmap, no rope, no new deps, no 100 MiB/1 GiB reads in default tests, no committed large fixtures, no flaky timing, no watcher/reload/save-conflict/manual-Ctrl+R behavior changes beyond size/open bookkeeping. All required tests (file::size, file::io, file_state::*, watcher_*, app::, full), rustfmt --check, git diff --check, commits per AGENTS. Phase 2B now has pre-read guardrails + Large/Huge warn + Extreme refuse; perf harness and large-file mode still future.

- Phase 2-ah (broader pass): size.rs tests split to size_tests.rs for line hygiene (<300 both). Added no-deps generated-file helpers (dense streaming chunks + sparse via set_len) + cheap default smokes (64 KiB/1 MiB max) + ignored manual big-file smokes under src/tests/perf.rs. Default smokes prove harness: size match, App::new records metadata, open/render no panic. Ignored tests: manual_open_10mib_generated_file_smoke (Large warn), manual_open_100mib_generated_file_smoke, manual_sparse_extreme_refusal_smoke (refuse before read, skip-clean). measure_elapsed present for ignored only; no perf pass/fail thresholds yet. No lazy loading, no mmap, no rope rewrite, no new deps, no default 100 MiB/1 GiB reads, no committed fixtures, no watcher/reload/save-conflict changes, no editing/render changes.
  Exact commands for manual:
    cargo test manual_open_10mib_generated_file_smoke -- --ignored --nocapture
    cargo test manual_open_100mib_generated_file_smoke -- --ignored --nocapture
    cargo test manual_sparse_extreme_refusal_smoke -- --ignored --nocapture
  Phase 2B now has: size classification + pre-read guardrails, no-deps generated-file helper, cheap default harness smokes, ignored manual 10/100/sparse-1G smokes, no perf thresholds, no lazy/large-file mode yet. All mandated tests + full suite + fmt + diff--check + commits per AGENTS.

- Phase 2-ai (broader pass): split src/tests/perf.rs (hub + perf_helpers.rs + perf_default.rs + perf_manual.rs, all <300 lines, #[path] style, identical discovery via cargo test tests::perf and bare names). Removed default timing gates (phase0/phase1b + harness now assert only deterministic functional: non-empty render, buffer changed as expected, exact size, App size metadata; elapsed may eprintln under --nocapture only, never fail). Added tiny no-deps PerfSample + measure_sample + print_perf_sample in helpers; wired ignored manual tests to emit stable "PERF sample: label=... bytes=... elapsed_ms=..." for generate/App::new/render (and set_len/App for sparse). Manual tests kept: manual_open_10mib... (Large warn + samples), manual_open_100mib... (Huge/Large + skip clean), manual_sparse_extreme_refusal_smoke (refuse + samples). No 1 GiB dense. Optional manual_render not added (open already covers render smoke). No thresholds, no lazy/mmap/rope/new-deps/committed-fixtures/default-large-reads/open-policy/watcher-reload/save-conflict/render or buffer edits. All required tests + full suite green; rustfmt (touched); diff--check; commits per AGENTS.
  Exact commands:
    cargo test manual_open_10mib_generated_file_smoke -- --ignored --nocapture
    cargo test manual_open_100mib_generated_file_smoke -- --ignored --nocapture
    cargo test manual_sparse_extreme_refusal_smoke -- --ignored --nocapture
    cargo test tests::perf -- --nocapture   # (shows default eprintln elapsed if any + harness)
  Note: no perf thresholds yet. Next pass: run the manual baselines on target hardware, record numbers, decide initial budgets and hotspots (without optimizing yet).
  Phase 2B now has a split no-deps perf harness with non-timing default smokes and ignored manual baseline reporting (PERF sample lines; baselines captured on 2026-06-24 hardware), but still no lazy loading, mmap, rope rewrite, perf thresholds, default 100 MiB/1 GiB reads, new deps, or watcher/reload/save-conflict behavior changes.

- Phase 2-aj (broader hygiene + status foundation pass): captured and documented Phase 2B manual baselines in docs/performance.md (with env, /usr/bin/time MaxRSS, exact PERF sample lines; all three manual runs succeeded on this hardware); split src/file/io tests to io_tests.rs + refreshed io.rs header to current truth (owns atomic + metadata snapshot/observe; must not construct watchers, read-for-detection, know App/Project/LLM); extracted pre-read open-size guardrails from App::new into src/app/open.rs (prepare_open_file_meta uses OpenSizeDecision; App::new shorter + still identical behavior for all cases incl. Extreme refuse before read); added tiny src/app/status.rs + wiring so that when app.message is None the bottom row shows persistent status (mode/path/[untitled]/modified|saved + format_file_size + tier label + "large-file mode" for Large/Huge); message still fully overrides status; render_buffer kept generic; App decides the string. Added cheap pure status tests + render/status override smoke (small files; Large marker proven via pure + existing Large open warning test). Updated TODO.md (adjusted stale "after 2-ag" / "perf harness unfinished" / "no ... yet" to reflect recorded baselines + first visible large-file status; kept all limitations honest) and docs/performance.md (points to split perf_*.rs; notes new status behavior). Large/Huge still do full content read on open; Extreme refuses pre-read; no lazy/mmap/rope; full clear render unchanged; no thresholds; no new deps; no watcher/reload/save-conflict changes. All required tests (incl. file_size_open, status, perf, full), fmt --check, git diff --check green; commits small/coherent per AGENTS. Phase 2B not claimed complete.
  Suggested next: identify first hotspots from the recorded numbers, propose (not implement) initial budgets; keep hardening.

- Phase 2-ak (hygiene extraction pass): docs updated with advisory candidate Phase 2B budgets + observed hotspot inventory (open/materialization dominant for 10/100 MiB full materialization paths); status size now labeled `disk <size>` using last-known on-disk metadata (fs::metadata or narrow post-save fallback), never live buffer size; `App::render` message path no longer clones the message (passes &str directly via Option<&str>); key handling extracted to `src/app/input.rs` (mod.rs thin delegation; no command enum or dispatcher introduced). No behavior changes claimed; no lazy loading, mmap, rope rewrite, or new dependencies; watcher/reload/save-conflict/manual-Ctrl+R semantics untouched. All required tests, rustfmt --check, git diff --check green; small coherent commits per AGENTS. Phase 2B not claimed complete.

- Phase 2-al (post-2-ak cleanup round): dedicated rustfmt baseline commit landed; global `cargo fmt --check` now truly green. Input cleanup helper centralized post-content-edit cleanup without behavior change. Render writes visible chars directly and avoids per-visible-line temporary `String` allocation. Stale imports/comments cleaned after input extraction. Performance docs got a short render/input hygiene note. No lazy/mmap/rope/new deps. No watcher/reload/save-conflict/manual-Ctrl+R behavior changes. Manual big-file tests stayed ignored. Phase 2B not complete.

- Phase 2-am (narrow open metadata hygiene): open metadata/snapshot capture in prepare_open_file_meta now uses one initial capture_file_snapshot (disk_snapshot carried in OpenFileMeta) instead of separate size + snapshot probes in App::new. Size/tier derived from that snapshot for present files. Not lazy loading and not a materialization fix; content is still fully read. Open/materialization hotspot remains read_to_string + PieceTable::from_text. Added focused snapshot asserts in guardrail tests. No watcher/reload/save-conflict changes, no new deps, manual tests stay ignored, Phase 2B not claimed complete.

- Phase 2-an (narrow measured optimization): recorded 2026-07-07 open-path phase split in docs/performance.md, then added an LF-only PieceTable::from_text normalization fast path (skip unconditional CR replace passes when no '\r'). CRLF/CR parity preserved; manual 10/100 MiB samples improved App::new and PieceTable::from_text roughly 2x on this hardware. No lazy/mmap/rope/new deps, no thresholds/gates, manual tests remain ignored, Phase 2B not claimed complete.

- Phase 2-ao (narrow open materialization cleanup): added PieceTable::from_owned_text and wired App::new to move the read_to_string buffer into PieceTable for opens, avoiding the remaining LF-only large-file clone. Manual perf labels now measure the owned constructor used by App open; 100 MiB App::new sample improved from ~679 ms post-2-an to ~620 ms on this hardware. CRLF/CR parity covered by owned-constructor tests. No lazy/mmap/rope/new deps, no thresholds/gates, manual tests remain ignored, Phase 2B not claimed complete.

- Phase 2-ap (narrow LineIndex build optimization): build_index now uses std string newline search (`match_indices('\n')`) instead of a hand-rolled byte-by-byte loop. Manual 100 MiB no-newline sample improved PieceTable::from_owned_text from ~603 ms to ~14 ms and App::new from ~620 ms to ~62 ms on this hardware; read_to_string is now the largest measured editor-owned subphase in that synthetic path. No lazy/mmap/rope/new deps, no thresholds/gates, manual tests remain ignored, Phase 2B not claimed complete.

- Phase 2-aq (narrow open-policy seam + reload clone cleanup): prepare_open_file_meta now carries an explicit OpenContentPlan (untitled empty, missing empty from the captured Absent snapshot, present full-read). App::new follows that plan, so missing paths no longer attempt a second content read after the initial Absent capture, while present files still read fully into PieceTable. Confirmed Ctrl+R Modified reload now uses file::io::read_to_string plus PieceTable::from_owned_text, avoiding the old read buffer clone on reload. Added focused open-plan tests. No lazy/mmap/rope/new deps, no thresholds/gates, no watcher signal or save-conflict policy change, manual tests remain ignored, Phase 2B not claimed complete.
