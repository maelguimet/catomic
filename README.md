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
