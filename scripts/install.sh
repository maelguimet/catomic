#!/usr/bin/env bash
# Purpose: install Catomic from this checkout and provision its private user config.
# Owns: Rust bootstrap, the documented source install, and first-install config creation.
# Must not: replace an existing config, mutate shell profiles, or hide Cargo failures.
# Invariants: bootstrapping is conditional; config is private and published atomically.

set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

bootstrap_cargo() {
  if [[ -z "${HOME:-}" || "$HOME" != /* ]]; then
    echo "catomic install: HOME must be absolute to install Rust" >&2
    exit 1
  fi

  local cargo_home="${CARGO_HOME:-$HOME/.cargo}"
  if [[ "$cargo_home" != /* ]]; then
    echo "catomic install: CARGO_HOME must be absolute to install Rust" >&2
    exit 1
  fi

  echo "catomic install: Cargo not found; installing a minimal stable Rust toolchain"
  export CARGO_HOME="$cargo_home"
  if command -v curl >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 --fail --silent --show-error https://sh.rustup.rs |
      sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path
  elif command -v wget >/dev/null 2>&1; then
    wget -qO- https://sh.rustup.rs |
      sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path
  else
    echo "catomic install: Cargo is missing and Rust bootstrap requires curl or wget" >&2
    exit 1
  fi

  export PATH="$cargo_home/bin:$PATH"
  if ! command -v cargo >/dev/null 2>&1; then
    echo "catomic install: Rust bootstrap completed without installing Cargo" >&2
    exit 1
  fi
}

if ! command -v cargo >/dev/null 2>&1; then
  bootstrap_cargo
fi
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
