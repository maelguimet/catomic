# 0005 - Paged Oversized Files

Date: 2026-07-16

Status: accepted

## Decision

Catomic will not reject a regular UTF-8 file solely because it exceeds a fixed
byte threshold. Files beyond the normal editable policy open in a bounded,
read-only paged mode.

- A page contains a configured number of logical lines.
- The default is 20,000 lines.
- Users set `[big_files] page_lines = N` in
  `$XDG_CONFIG_HOME/catomic/config.toml` or
  `~/.config/catomic/config.toml`.
- Page navigation reads only the active page plus bounded scan state.
- Whole-file Ctrl+F streams across all pages, preserves matches across read
  boundaries, can be cancelled, and jumps to the page containing a match.
- Resource or content errors are reported; byte size alone is not refusal.
- Paged mode remains read-only until safe cross-page edit/save semantics exist.

The configuration is Plain-safe: it performs one small local file read during
startup and constructs no Project, LLM, background, or network services.

## Rationale

A fixed 1 GiB refusal wastes the bounded descriptor-read foundation and prevents
useful log inspection. Line-count pages let users tune metadata residency for
their machine while keeping content reads bounded. Streaming search retains
whole-file usefulness without building a global in-memory index.
