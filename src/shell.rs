//! The per-user shell setup `dev` generates, which `share/shell` loads.
//!
//! Each tool gets a file in the [`ShellDir`], e.g. `zerobrew`, holding its own
//! setup. Every write also replaces `.stamp` with a new token. Shells check the
//! stamp before each prompt and reload the setup when it changed, so open
//! shells pick up changes without re-sourcing anything.

use std::path::PathBuf;

use color_eyre::eyre::eyre;

mod dir;
pub mod env;

pub use dir::ShellDir;

/// `share/shell`, printed by `dev shell-init` for rc files to eval. Built in
/// so it always matches the `dev` that prints it.
pub const SETUP: &str = include_str!("../share/shell");

/// `$HOME`, failing when it's unset or empty.
pub fn home() -> crate::Result<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| eyre!("HOME is not set"))
}

/// `$XDG_CONFIG_HOME`, or `~/.config` when it's unset or empty.
pub fn config_home() -> crate::Result<PathBuf> {
    match std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()) {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => Ok(home()?.join(".config")),
    }
}

/// `$XDG_DATA_HOME`, or `~/.local/share` when it's unset or empty.
pub fn data_home() -> crate::Result<PathBuf> {
    match std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()) {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => Ok(home()?.join(".local/share")),
    }
}
