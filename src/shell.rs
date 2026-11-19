//! The per-user shell setup `dev` generates, which `share/shell` loads.
//!
//! Each tool gets a file in the [`ShellDir`], e.g. `zerobrew`, holding its own
//! setup. Every write also replaces `.stamp` with a new token. Shells check the
//! stamp before each prompt and reload the setup when it changed, so open
//! shells pick up changes without re-sourcing anything.

use std::path::PathBuf;

use crate::layout;

mod dir;
pub mod env;

pub use dir::ShellDir;

/// `share/shell`, printed by `dev shell-init` (after [`init_script`]'s
/// preamble) for rc files to eval. Built in so it always matches the `dev`
/// that prints it.
pub const SETUP: &str = include_str!("../share/shell");

/// The setup files dev writes to the [`ShellDir`], in the order shells load
/// them. mise goes last, so the tools it manages come first on `PATH`.
pub const SETUP_FILES: [&str; 3] = [
    crate::deps::brew::SHELL_FILE,
    crate::deps::rust::SHELL_FILE,
    crate::deps::mise::SHELL_FILE,
];

/// What `dev shell-init` prints: the shell dir, its stamp, and
/// [`SETUP_FILES`] as sh variables, then [`SETUP`], which reads them.
pub fn init_script(dir: &ShellDir) -> String {
    let path = |p: &std::path::Path| env::quote(&p.to_string_lossy());
    // Plain lowercase names, one per line, which `share/shell` reads in order.
    debug_assert!(
        SETUP_FILES
            .iter()
            .all(|name| name.chars().all(|c| c.is_ascii_lowercase()))
    );
    format!(
        "__dev_shell_dir={}
__dev_stamp_file={}
__dev_setups={}
{SETUP}",
        path(dir.path()),
        path(&dir.stamp_file()),
        env::quote(&SETUP_FILES.join("\n")),
    )
}

/// `$HOME`, failing when it's unset or empty.
pub fn home() -> crate::Result<PathBuf> {
    layout::home(&layout::process)
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
    layout::data_home(&layout::process)
}

/// `$XDG_STATE_HOME`, or `~/.local/state` when it's unset or empty.
pub fn state_home() -> crate::Result<PathBuf> {
    match std::env::var_os("XDG_STATE_HOME").filter(|d| !d.is_empty()) {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => Ok(home()?.join(".local/state")),
    }
}
