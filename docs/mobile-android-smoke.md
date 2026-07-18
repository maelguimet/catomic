# Android/Termux smoke-test record

Use this checklist for every Android configuration that Catomic claims as
hardware-validated. Run it on non-sensitive fixtures in Termux private storage
and, separately, probe shared storage only to verify clear fail-closed behavior.
Never exercise model steps against a live endpoint; cancel at the confirmation
or use an explicitly configured local fake server.

## Current reference record

- Result: **NOT RUN — no Android/Termux device was attached to the issue #66
  implementation environment.**
- Tester and date: pending
- Catomic commit: pending
- Device/model and CPU ABI: pending
- Android version/API and security patch: pending
- Termux version and install source: pending
- Terminal renderer and `$TERM`: pending
- `rustc --version` and `clang --version`: pending
- Locale (`locale` and relevant environment): pending
- Soft keyboard name/version and Termux extra-key configuration: pending
- Private-storage filesystem and mount: pending
- Shared-storage provider/mount: pending

## Install and environment

- [ ] Install the documented Termux build from F-Droid or official GitHub.
- [ ] Run the documented package and Cargo install commands from a clean clone.
- [ ] Record exact command output and confirm `catomic --version` and `--help`.
- [ ] Launch in Termux private `$HOME` with no hardware keyboard connected.
- [ ] Confirm the mobile action row appears automatically and help is reachable.

## Core touch workflow

- [ ] Open an existing file and create, Save As, switch, and close buffers from
      the action palette.
- [ ] Tap before/inside/after ASCII, a tab, `e` plus a combining accent, `猫`, and
      an emoji sequence; confirm every cursor lands on a grapheme boundary.
- [ ] Repeat with line numbers, soft wrapping, horizontal scroll, and a 20-column
      viewport. Confirm status/action taps never move the document cursor.
- [ ] Select across wrapped Unicode using mark-start/tap-end; also test direct
      drag if the terminal reports drag events.
- [ ] Copy, cut, internal paste, Termux/system paste, undo, and redo. Confirm OSC
      52 reaches Android clipboard without printing control bytes into the file.
- [ ] Finger-scroll without changing the cursor or corrupting selection; use
      Page Up/Down, cursor actions, Home/End, and large-file page actions.
- [ ] Find, Find/Replace, goto, help, command prompt, Markdown preview, line
      numbers, whitespace, and soft-wrap actions; dismiss each without Escape.
- [ ] Save, switch buffers, and quit from touch paths. Repeat guarded dirty quit.

## Narrow views and model safety

- [ ] At 20x6, scroll/dismiss help, every prompt, overwrite/reload warnings,
      recovery preview, command output, diagnostics, and Markdown preview.
- [ ] Configure a loopback fake endpoint or stop before send. Verify endpoint and
      context confirmation Info/No/Yes, proposal preview scrolling, apply, cancel,
      task cancellation, and undo—never a live model or public endpoint.
- [ ] Rotate portrait/landscape with dirty text, an active selection, every
      prompt stage, a scrolled preview, and a proposal. Confirm state survives.
- [ ] Background and foreground Termux in each state; confirm coherent redraw.

## Filesystem, watcher, and recovery

- [ ] On private storage, save a new and existing file; verify bytes, line
      endings, mode, owner/group, no sibling temp, and parent-directory durability.
- [ ] Verify a regular-file symlink keeps the final symlink and updates its
      referent; verify dangling and non-regular targets are refused.
- [ ] Verify hard-linked and xattr/ACL-bearing files are refused unchanged.
- [ ] Edit/replace/delete the file externally while Catomic is clean and dirty;
      verify auto-reload/confirmation, overwrite guard, and repeated drift checks.
- [ ] Enable `.catnap`, background long enough to create it, kill the process,
      reopen, preview, apply, save, and verify owner-only mode and cleanup.
- [ ] Run `termux-setup-storage`; probe a disposable shared-storage copy. Record
      actual rename, metadata, watcher, and symlink behavior. Any unsupported
      operation must fail clearly and preserve the original bytes.

## Teardown and evidence

- [ ] Quit normally and send `SIGINT`, `SIGTERM`, and `SIGHUP`; verify raw,
      alternate-screen, mouse, focus, and bracketed-paste modes are restored.
- [ ] Force-stop once, record unavoidable teardown behavior, run `reset`, and
      verify the last explicit save plus any completed recovery sidecar.
- [ ] Attach sanitized command output, screenshots/video, fixture hashes, and the
      completed environment fields to the release/issue evidence record.
