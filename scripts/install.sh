#!/usr/bin/env bash
# Purpose: install Catomic from this checkout and provision its private user config.
# Owns: the documented source-install command and first-install config creation.
# Must not: replace an existing config, weaken its directory, or hide Cargo failures.
# Invariants: a new config is owner-only and published atomically from the canonical template.

set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cargo install --path "$repo_root" --locked "$@"

if [[ -n "${XDG_CONFIG_HOME:-}" && "$XDG_CONFIG_HOME" == /* ]]; then
  config_root="$XDG_CONFIG_HOME"
elif [[ -n "${HOME:-}" && "$HOME" == /* ]]; then
  config_root="$HOME/.config"
else
  echo "catomic install: XDG_CONFIG_HOME and HOME are not absolute" >&2
  exit 1
fi

config_dir="$config_root/catomic"
config_path="$config_dir/config.toml"
template="$repo_root/src/config/config_template.toml"

if [[ -e "$config_path" || -L "$config_path" ]]; then
  echo "catomic install: preserved existing config $config_path"
  exit 0
fi
if [[ -L "$config_dir" ]]; then
  echo "catomic install: refusing symlinked config directory $config_dir" >&2
  exit 1
fi

umask 077
mkdir -p -- "$config_dir"
if [[ ! -d "$config_dir" ]]; then
  echo "catomic install: config parent is not a directory: $config_dir" >&2
  exit 1
fi

directory_mode="$(stat -c '%a' -- "$config_dir")"
if (( (8#$directory_mode & 077) != 0 )); then
  echo "catomic install: config directory must be user-only (mode 0700): $config_dir has mode $directory_mode" >&2
  exit 1
fi

staged="$(mktemp "$config_dir/.config.toml.install.XXXXXX")"
cleanup() {
  rm -f -- "$staged"
}
trap cleanup EXIT
cp -- "$template" "$staged"
chmod 0600 -- "$staged"
sync -f -- "$staged"

if ln -- "$staged" "$config_path"; then
  rm -f -- "$staged"
  trap - EXIT
  echo "catomic install: created private config $config_path"
elif [[ -e "$config_path" || -L "$config_path" ]]; then
  echo "catomic install: preserved config created concurrently at $config_path"
else
  echo "catomic install: could not create config $config_path" >&2
  exit 1
fi
