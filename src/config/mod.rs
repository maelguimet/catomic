//! Configuration (TOML), keymaps, per-language settings.
//!
//! Per TODO:
//! - Simple TOML with good defaults. No config file required.
//! - Per-language settings (linters, tab size, ...)
//! - Keybinding configuration (simple overrides)
//!
//! Mode and Capabilities may be influenced by config, but the hard
//! Plain vs Project construction gates are still enforced.

pub mod keymap;
