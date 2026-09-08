# Open-beta daily-driver polish gate

This is the operator protocol for the focused open-beta gate tracked by
[#72](https://github.com/maelguimet/catomic/issues/72). It coordinates one
human release-candidate session; it does not replace the implementations or
acceptance evidence owned by child issues #64, #63, #56, #57, #54, and #55.

## Entry conditions

Run the gate only when every listed child issue has either a merged closing PR
or a written deferral rationale suitable for the final issue comment. Use a
clean checkout at the exact release-candidate SHA and the complete bundle
downloaded from that commit's successful Acceptance workflow run. Do not use a
debug build, `cargo run`, a rebuilt binary, an ambient Catomic configuration, or
a command that contacts a public service.

The harness validates the bundle's clean managed-build metadata, source SHA,
partial automated matrix, checksum, size, and candidate bytes before copying
the accepted binary into a new private session directory. It then hashes it,
creates several real text/code files in an isolated Git repository, and launches
that preserved binary. The isolated configuration enables local catnap recovery
and contains no named commands or hooks.

## Run the session

From the terminal being accepted, supply the emulator and multiplexer versions
explicitly. `$TERM` alone is not a terminal version.

```sh
scripts/daily-driver-gate.sh run /absolute/new/session-directory \
  --acceptance-bundle /path/to/catomic-compatibility-commit \
  --terminal "kitty 0.42.2" \
  --multiplexer "tmux 3.5a"
```

Use `none` when no multiplexer is present. The directory must not already
exist; the harness never overwrites or deletes an acceptance session. A loose
binary, a bundle for another source commit, changed bytes, or contradictory
matrix/build metadata is rejected before the editor session starts.

Treat Catomic as an ordinary editor for a sustained session of at least eight
minutes. The validator rejects a shorter record, keeping the session inside the
ten-minute process bound while preventing a start/quit smoke from satisfying
this gate. Use only `Ctrl+H` or `F1` to discover common actions. Complete open,
edit, undo/redo, find, save, and close before consulting external documentation.
Continue through selection, replace, clipboard, soft wrap, multiple buffers,
Save As, external-change handling, and recovery. Resize to both a narrow and a
normal width and inspect normal status, prompts, warnings/confirmations, errors,
Markdown source, and F6 preview. The generated `external-change.sh` is an
explicit second-terminal helper for the external-change scenario.

## Record and validate

After the editor exits, the session directory retains:

- the preserved release binary and its SHA-256 binding;
- `session.env` with exact source, terminal, locale, timestamps, duration, exit,
  and terminal-restoration evidence;
- before/after fixture hashes; and
- `final-comment.md`, the required issue-comment schema.

Review the saved files against intended content. Complete every checkbox and
placeholder in `final-comment.md`. Every child issue needs a closing PR URL or
an explicit `Deferred:` rationale. Every observed defect needs its own focused
GitHub issue and exact evidence; write `None observed.` only when that is true.

Then validate the packet:

```sh
scripts/daily-driver-gate.sh verify /absolute/session-directory
```

Validation fails for unchecked scenarios, placeholder text, unexplained child
issues, missing defect links, a nonzero editor exit, changed release bytes, or
terminal settings that were not restored exactly. A successful validation says
only that the record is structurally complete and bound to the captured binary;
the human operator remains responsible for the truth of the observations.

Post the validated `final-comment.md` through the release-owner workflow. The
harness does not push, publish, comment, or close issues.
