#!/usr/bin/env bash
# Purpose: install Catomic from this checkout and provision its user environment.
# Owns: Rust bootstrap, Cargo PATH setup, source install, and first-install config creation.
# Must not: replace existing config/profile bytes or hide Cargo failures.
# Invariants: PATH setup is conditional and idempotent; config is private and atomic.

set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ -n "${CARGO_HOME:-}" ]]; then
  cargo_home="$CARGO_HOME"
elif [[ -n "${HOME:-}" && "$HOME" == /* ]]; then
  cargo_home="$HOME/.cargo"
else
  echo "catomic install: HOME must be absolute when CARGO_HOME is unset" >&2
  exit 1
fi
if [[ "$cargo_home" != /* ]]; then
  echo "catomic install: CARGO_HOME must be absolute" >&2
  exit 1
fi
cargo_bin="$cargo_home/bin"

path_contains_directory() {
  case ":${PATH:-}:" in
    *:"$1":*) return 0 ;;
    *) return 1 ;;
  esac
}

quote_shell_word() {
  local value="$1"
  printf "'%s'" "${value//\'/\'\\\'\'}"
}

append_path_setup() {
  local profile="$1"
  local line="$2"
  local comment="# Added by the Catomic installer for Cargo-installed binaries."

  if [[ -e "$profile" && ! -f "$profile" ]]; then
    echo "catomic install: shell profile is not a regular file: $profile" >&2
    exit 1
  fi
  if [[ -f "$profile" ]] && grep -Fqx -- "$line" "$profile"; then
    return
  fi
  printf '\n%s\n%s\n' "$comment" "$line" >> "$profile"
}

persist_cargo_path() {
  if [[ -z "${HOME:-}" || "$HOME" != /* ]]; then
    echo "catomic install: HOME must be absolute to configure shell PATH" >&2
    exit 1
  fi

  local quoted_bin
  quoted_bin="$(quote_shell_word "$cargo_bin")"
  local shell_path="${SHELL:-/bin/sh}"
  local shell_name="${shell_path##*/}"
  local -a profiles=()
  local line

  case "$shell_name" in
    bash)
      profiles+=("$HOME/.bashrc")
      if [[ -e "$HOME/.bash_profile" ]]; then
        profiles+=("$HOME/.bash_profile")
      elif [[ -e "$HOME/.bash_login" ]]; then
        profiles+=("$HOME/.bash_login")
      else
        profiles+=("$HOME/.profile")
      fi
      line="export PATH=$quoted_bin:\"\$PATH\""
      ;;
    zsh)
      profiles+=("$HOME/.zshrc")
      line="export PATH=$quoted_bin:\"\$PATH\""
      ;;
    fish)
      local fish_config_root="${XDG_CONFIG_HOME:-$HOME/.config}"
      if [[ "$fish_config_root" != /* ]]; then
        echo "catomic install: XDG_CONFIG_HOME must be absolute to configure fish PATH" >&2
        exit 1
      fi
      mkdir -p -- "$fish_config_root/fish"
      profiles+=("$fish_config_root/fish/config.fish")
      line="fish_add_path --global $quoted_bin"
      ;;
    *)
      profiles+=("$HOME/.profile")
      line="export PATH=$quoted_bin:\"\$PATH\""
      ;;
  esac

  local profile
  for profile in "${profiles[@]}"; do
    append_path_setup "$profile" "$line"
  done
  echo "catomic install: configured $cargo_bin for future $shell_name shells; open a new shell to use catomic"
}

bootstrap_cargo() {
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

  if ! command -v cargo >/dev/null 2>&1; then
    echo "catomic install: Rust bootstrap completed without installing Cargo" >&2
    exit 1
  fi
}

cargo_path_was_missing=false
if ! path_contains_directory "$cargo_bin"; then
  cargo_path_was_missing=true
  export PATH="$cargo_bin:${PATH:-}"
fi
if ! command -v cargo >/dev/null 2>&1; then
  bootstrap_cargo
fi
cargo install --path "$repo_root" --locked "$@"
if [[ "$cargo_path_was_missing" == true ]]; then
  persist_cargo_path
fi

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
