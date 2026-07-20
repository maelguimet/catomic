# Linux Terminal and Filesystem Compatibility

Catomic's Linux-first claim is deliberately narrower than “works in every
terminal on every mount.” Support is attached to an exact binary and evidence
record, not inherited from a branch name or a different release.

The reproducible harness lives in [`scripts/compatibility/`](../scripts/compatibility/).
Each result follows [`result-schema-v1.json`](compatibility/result-schema-v1.json)
and records the source commit, binary checksum and size, host, kernel, mount,
locale, terminal path, expected scenarios, exit statuses, file hashes, terminal
restoration, and any focused defect link.

## Support policy

“Supported” below is the product boundary. A particular release may claim a row
only when its release-candidate matrix links a passing result for the exact
tested artifact. “Required target” is not a claim that the current artifact was
tested there. “Best effort” means useful reports are welcome, but failure does
not block a release until that row is promoted intentionally.

### Terminal paths

| Path | Classification | Evidence required |
| --- | --- | --- |
| Direct Linux PTY, UTF-8 locale | Supported baseline | Deterministic real-binary PTY coverage in normal CI and an artifact-bound result |
| Current tmux on Linux | Required release target | Artifact-bound input, resize, signal, and teardown result; host clipboard behavior may require manual OSC 52 evidence |
| SSH into Linux, then current tmux | Required release target | Exact SSH client/server path and versions plus the complete terminal checklist |
| GNOME Terminal / current VTE | Required GUI target | Operator-attested checklist in the real GUI terminal |
| Current Kitty or Alacritty | Required GUI target | Operator-attested checklist in the real GUI terminal |
| Ghostty, where available | Required target, otherwise explicitly untested | Operator-attested checklist; absence is recorded as untested, never inferred |
| Other Linux emulators and multiplexers | Best effort | Complete result encouraged |
| Termux/Android, Windows, and macOS | Untested/not supported by this matrix | Separate platform work is required |

A release-candidate gate requires three materially different passing terminal
paths for core open/edit/save/quit and restoration. It also requires shortcut
and input delivery to pass manually in two terminals whose result category is
`gui`. An automated PTY result cannot satisfy that GUI requirement.

Every terminal result names all of these scenarios: input delivery, shifted and
Unicode text, F1/F2 fallback keys, SGR mouse mapping, bracketed paste, OSC 52,
resize, signals, terminal restoration, and the core file flow. A capability can
be `unsupported` with an exact explanation; it cannot disappear from a report.

### Filesystem paths and objects

| Path or object | Classification | Expected boundary |
| --- | --- | --- |
| Local ext4 | Supported | Atomic save, both conflict sizes, frozen-mtime collision, and recovery must pass |
| Local tmpfs | Supported | Same required scenarios as ext4 |
| Deliberately frozen mtime | Required deterministic condition | Same-size in-place rewrite is detected despite restored mtime |
| Symlink to a regular file | Supported | Final symlink remains; regular referent is atomically replaced |
| Read-only regular file | Supported refusal | Save fails without changing bytes, inode, or mode |
| Multiple hard links | Supported | Staged in-place save updates every alias without changing inode identity or link count |
| xattrs or POSIX ACLs | Supported preservation | Atomic save retains and verifies the metadata before commit |
| FIFO, directory, socket, device, other non-regular target | Supported refusal | Open/save fails without blocking or replacement |
| overlayfs, NFS, SMB, FUSE, container bind mounts | Best effort | Record the exact mount and result; do not generalize from ext4/tmpfs |

The filesystem harness creates a fresh sandbox under a user-supplied existing
directory and never mounts anything. The ext4 and tmpfs rows are separate runs;
the harness verifies the actual mount with `findmnt`. Hard-link saves always
check inode, link count, mode, owner, and group, and also check user xattrs and
ACLs when the mount and host tools expose them. Standalone ACL coverage is
marked unsupported if `setfacl`/`getfacl` or mount support is absent. User
xattrs are tested independently through the Python standard library.

## Produce evidence

Use a clean candidate checkout and one already-built release-shaped binary.
Copying is allowed, but every run must point at byte-identical contents:

```sh
cargo build --release --locked
candidate_commit="$(git rev-parse HEAD)"
candidate_binary="$(cargo metadata --no-deps --format-version 1 | \
  python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/release/catomic"
mkdir -p compatibility-results

python3 scripts/compatibility/run_terminal.py automated \
  --binary "$candidate_binary" --commit "$candidate_commit" \
  --path direct-pty --operator "$USER" \
  --output compatibility-results/direct-pty.json

python3 scripts/compatibility/run_terminal.py automated \
  --binary "$candidate_binary" --commit "$candidate_commit" \
  --path tmux --operator "$USER" \
  --output compatibility-results/tmux.json
```

Run the manual command from inside each real terminal path. Supply names and
versions explicitly; the script will not guess an emulator from environment
variables:

```sh
python3 scripts/compatibility/run_terminal.py manual \
  --binary "$candidate_binary" --commit "$candidate_commit" \
  --path-id gnome-vte-local --category gui \
  --terminal "GNOME Terminal / VTE" --terminal-version "EXACT_VERSION" \
  --operator "$USER" --output compatibility-results/gnome-vte.json
```

Choose directories that `findmnt` proves are on the intended mounts:

```sh
python3 scripts/compatibility/run_filesystem.py \
  --binary "$candidate_binary" --commit "$candidate_commit" \
  --root /path/on/ext4 --environment-id ext4-local --operator "$USER" \
  --output compatibility-results/ext4.json

python3 scripts/compatibility/run_filesystem.py \
  --binary "$candidate_binary" --commit "$candidate_commit" \
  --root /dev/shm --environment-id tmpfs-local --operator "$USER" \
  --output compatibility-results/tmpfs.json
```

Aggregate exact result filenames. `--release-candidate` fails closed on mixed
artifacts, missing ext4/tmpfs results, fewer than three terminal paths, fewer
than two manual GUI terminals, missing scenarios, or unmet core results:

```sh
python3 scripts/compatibility/build_report.py \
  compatibility-results/direct-pty.json \
  compatibility-results/tmux.json \
  compatibility-results/gnome-vte.json \
  compatibility-results/kitty.json \
  compatibility-results/ext4.json \
  compatibility-results/tmpfs.json \
  --release-candidate \
  --output-json compatibility-results/matrix.json \
  --output-markdown compatibility-results/matrix.md
```

Result and report paths are create-only so an earlier record is not silently
replaced. The harness uses non-sensitive generated fixtures, makes no network
request, and removes sandboxes unless `--keep-sandbox` is explicit. Results do
contain operator, OS, kernel, mount paths/options, terminal versions, and locale;
review those fields before publication.

## Failures and release candidates

Do not turn a defect into “best effort” after observing it. First create a
focused GitHub issue with the exact result, artifact SHA-256, expected/observed
scenario, exit status, hashes, mount/terminal path, and a minimal fixture. Then
rerun with `--failure-issue URL`; validation rejects a `fail` result without
that link. Security-sensitive evidence follows the private reporting path.

For a release candidate, publish `matrix.json`, `matrix.md`, the tested binary,
and its checksum together under one durable URL. The candidate acceptance note
must link that matrix and name the same binary SHA-256. A final release does not
inherit results from another checksum, a rebuilt binary, a descendant commit,
or an expiring local path.

The separate Acceptance workflow builds one exact artifact and retains its
automated direct-PTY, tmux, runner-filesystem, and tmpfs bundle. Those checks are
useful prerequisites, not substitutes for ext4 classification or two real GUI
results when the runner does not provide them.
