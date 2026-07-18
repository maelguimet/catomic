#!/usr/bin/env bash
# Purpose: independently verify downloaded Catomic release assets and a real PTY smoke.
# Owns: manifest, architecture, version, installed-byte, and terminal teardown checks.
# Must not: download, publish, replace, or trust assets outside the supplied directory.
# Invariants: every expected asset is regular, checksummed, and bound to one tag and SHA.

set -euo pipefail

if [[ $# -ne 4 ]]; then
  echo "usage: $0 ASSET_DIR TAG SOURCE_SHA BINARY_NAME" >&2
  exit 2
fi

tag="$2"
source_sha="$3"
binary_name="$4"
version="${tag#v}"
package_name="catomic-$version.crate"

die() {
  echo "release verification failed: $*" >&2
  exit 1
}

asset_input="$1"
[[ -d "$asset_input" && ! -L "$asset_input" ]] || \
  die "asset directory is not a real directory"
asset_dir="$(realpath -- "$asset_input")"

[[ "$tag" == v* && "$version" != "$tag" ]] || die "tag must start with v"
[[ "$source_sha" =~ ^[0-9a-f]{40}$ ]] || die "source SHA must be 40 lowercase hex characters"
[[ "$binary_name" == "catomic-x86_64-unknown-linux-gnu" ]] || die "unexpected binary name"

required_tools=(cmp file grep install mktemp python3 readelf realpath sha256sum stat)
for tool in "${required_tools[@]}"; do
  command -v "$tool" >/dev/null || die "required tool is missing: $tool"
done

manifest_names=(
  "$binary_name"
  "$binary_name.sha256"
  "$package_name"
  package-contents.txt
  source-verification.txt
  toolchain.txt
)
all_names=("${manifest_names[@]}" SHA256SUMS)
for name in "${all_names[@]}"; do
  [[ "$name" != */* && "$name" != .* ]] || die "unsafe expected asset name: $name"
  [[ -f "$asset_dir/$name" && ! -L "$asset_dir/$name" ]] || die "missing regular asset: $name"
done

expected_names="$(printf '%s\n' "${manifest_names[@]}" | LC_ALL=C sort)"
actual_names="$(
  while read -r digest name extra; do
    [[ -z "${extra:-}" ]] || die "malformed SHA256SUMS line"
    name="${name#\*}"
    [[ "$digest" =~ ^[0-9a-f]{64}$ ]] || die "malformed SHA-256 digest"
    [[ "$name" != */* && "$name" != .* ]] || die "unsafe manifest asset name: $name"
    printf '%s\n' "$name"
  done < "$asset_dir/SHA256SUMS" | LC_ALL=C sort
)"
[[ "$actual_names" == "$expected_names" ]] || die "SHA256SUMS does not name the exact asset set"

(
  cd "$asset_dir"
  sha256sum --check --strict SHA256SUMS
  sha256sum --check --strict "$binary_name.sha256"
)

read -r binary_digest checksum_name checksum_extra < "$asset_dir/$binary_name.sha256"
checksum_name="${checksum_name#\*}"
[[ -z "${checksum_extra:-}" && "$checksum_name" == "$binary_name" ]] || \
  die "per-binary checksum does not name $binary_name"
[[ "$binary_digest" == "$(sha256sum "$asset_dir/$binary_name" | cut -d ' ' -f 1)" ]] || \
  die "per-binary checksum digest differs from downloaded bytes"

file_info="$(file -b -- "$asset_dir/$binary_name")"
[[ "$file_info" == *"ELF 64-bit LSB"* && "$file_info" == *"x86-64"* ]] || \
  die "binary is not an x86-64 Linux ELF: $file_info"
readelf -h "$asset_dir/$binary_name" | grep -Eq \
  'Machine:[[:space:]]+Advanced Micro Devices X86-64' || die "ELF machine is not x86-64"

temp_dir="$(mktemp -d)"
trap 'rm -rf -- "$temp_dir"' EXIT
mkdir -p "$temp_dir/bin" "$temp_dir/config" "$temp_dir/data" "$temp_dir/state"
installed="$temp_dir/bin/catomic"
install -m 0755 "$asset_dir/$binary_name" "$installed"
cmp --silent "$asset_dir/$binary_name" "$installed" || \
  die "installed copy differs from public asset"

# Release downloads are regular data files and need not retain an executable bit.
# Execute only the byte-identical installed copy whose mode we control.
reported_version="$("$installed" --version)"
[[ "$reported_version" == "catomic $version" ]] || \
  die "binary reported $reported_version instead of catomic $version"
grep -Fxq Cargo.toml "$asset_dir/package-contents.txt" || die "package list omits Cargo.toml"
grep -Fq 'rustc ' "$asset_dir/toolchain.txt" || die "toolchain record omits rustc"
grep -Fq 'cargo ' "$asset_dir/toolchain.txt" || die "toolchain record omits cargo"
grep -Fxq "tag=$tag" "$asset_dir/source-verification.txt" || die "verification tag differs"
grep -Fxq "source_sha=$source_sha" "$asset_dir/source-verification.txt" || \
  die "verification source SHA differs"

printf 'release PTY smoke\n' > "$temp_dir/fixture.txt"

python3 - "$installed" "$temp_dir/fixture.txt" "$temp_dir/pty-transcript" "$temp_dir" <<'PY'
import errno
import fcntl
import os
import pty
import select
import signal
import struct
import sys
import termios
import time

binary, fixture, transcript, temp_dir = sys.argv[1:]
environment = os.environ.copy()
environment.update(
    {
        "HOME": temp_dir,
        "XDG_CONFIG_HOME": f"{temp_dir}/config",
        "XDG_DATA_HOME": f"{temp_dir}/data",
        "XDG_STATE_HOME": f"{temp_dir}/state",
        "LANG": "C.UTF-8",
    }
)

pid, master = pty.fork()
if pid == 0:
    os.execve(binary, [binary, fixture], environment)

fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
output = bytearray()
sent_quit = False
status = None
deadline = time.monotonic() + 15

while time.monotonic() < deadline:
    readable, _, _ = select.select([master], [], [], 0.05)
    if readable:
        try:
            output.extend(os.read(master, 8192))
        except OSError as error:
            if error.errno != errno.EIO:
                raise
    if output and not sent_quit:
        os.write(master, b"\x11")
        sent_quit = True
    finished, candidate = os.waitpid(pid, os.WNOHANG)
    if finished:
        status = candidate
        break

if status is None:
    os.kill(pid, signal.SIGKILL)
    os.waitpid(pid, 0)
    raise SystemExit("catomic PTY smoke timed out")

while True:
    readable, _, _ = select.select([master], [], [], 0)
    if not readable:
        break
    try:
        chunk = os.read(master, 8192)
    except OSError as error:
        if error.errno == errno.EIO:
            break
        raise
    if not chunk:
        break
    output.extend(chunk)
os.close(master)

with open(transcript, "wb") as stream:
    stream.write(output)
if not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
    raise SystemExit(f"catomic PTY smoke exited with wait status {status}")
PY

grep -Fq $'\033[?1000l' "$temp_dir/pty-transcript" || die "PTY smoke did not disable mouse capture"
grep -Fq $'\033[?1004l' "$temp_dir/pty-transcript" || die "PTY smoke did not disable focus reporting"
grep -Fq $'\033[?2004l' "$temp_dir/pty-transcript" || \
  die "PTY smoke did not disable bracketed paste"
grep -Fq $'\033[?1049l' "$temp_dir/pty-transcript" || die "PTY smoke did not leave alternate screen"

echo "tag=$tag"
echo "source_sha=$source_sha"
echo "manifest=success"
echo "architecture=x86-64-linux-elf"
echo "version=$reported_version"
echo "installed_copy_byte_identical=yes"
echo "pty_start_quit_teardown=success"
