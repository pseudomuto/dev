//! Where dev and the tools it installs keep things, after every env override.
//!
//! The one place these rules live. The installers use them, and so do the
//! shell setup and the GitHub Action, through `dev shell-init` and
//! `dev action-paths`, so none of them can disagree about where things are.
//!
//! Each rule reads env vars through a [`Vars`], so it can come from the
//! process or a map. An empty value counts as unset, as each tool's own
//! installer has it.

use std::{collections::BTreeMap, env, ffi::OsString, path::PathBuf};

use color_eyre::eyre::eyre;

use crate::Result;

/// Reads an env var, `None` when it's unset.
pub type Vars<'a> = dyn Fn(&str) -> Option<OsString> + 'a;

/// Reads the process env.
pub fn process(name: &str) -> Option<OsString> {
    env::var_os(name)
}

/// zerobrew's root when it isn't in the XDG data dir: always on macOS, and
/// anywhere it already exists. It's as long as `/opt/homebrew`, so bottles
/// built for Homebrew can be patched to it in place.
pub const OPT_ZEROBREW: &str = "/opt/zerobrew";

/// What zerobrew's root depends on besides env vars.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Host {
    pub macos: bool,
    /// Whether [`OPT_ZEROBREW`] exists.
    pub opt_zerobrew: bool,
}

impl Host {
    /// This machine.
    pub fn current() -> Self {
        Self {
            macos: cfg!(target_os = "macos"),
            opt_zerobrew: std::path::Path::new(OPT_ZEROBREW).exists(),
        }
    }
}

/// Every location at once, as the shell setup and the Action need them.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// The generated shell setup, see [`shell_dir`].
    pub shell_dir: PathBuf,
    pub mise_bin: PathBuf,
    pub mise_data: PathBuf,
    pub cargo_home: PathBuf,
    pub rustup_home: PathBuf,
    /// The dir `zb`, `zb-brew`, and `zbx` go in.
    pub zb_bin: PathBuf,
    pub zerobrew_root: PathBuf,
}

impl Layout {
    /// The layout for `env`, e.g. a shell's or a test's.
    pub fn from_env(env: &BTreeMap<String, String>, host: Host) -> Result<Self> {
        Self::new(&|name| env.get(name).map(OsString::from), host)
    }

    /// The layout for this process and machine.
    pub fn current() -> Result<Self> {
        Self::new(&process, Host::current())
    }

    pub fn new(vars: &Vars<'_>, host: Host) -> Result<Self> {
        Ok(Self {
            shell_dir: shell_dir(vars)?,
            mise_bin: mise_bin(vars)?,
            mise_data: mise_data(vars)?,
            cargo_home: cargo_home(vars)?,
            rustup_home: rustup_home(vars)?,
            zb_bin: zb_bin(vars)?,
            zerobrew_root: zerobrew_root(vars, host)?,
        })
    }
}

/// `name`, unless it's unset or empty.
fn var(vars: &Vars<'_>, name: &str) -> Option<PathBuf> {
    vars(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// `$name`, or `default` under `$HOME`.
fn var_or_home(vars: &Vars<'_>, name: &str, default: &str) -> Result<PathBuf> {
    match var(vars, name) {
        Some(path) => Ok(path),
        None => Ok(home(vars)?.join(default)),
    }
}

/// `$HOME`, failing when it's unset or empty.
pub fn home(vars: &Vars<'_>) -> Result<PathBuf> {
    var(vars, "HOME").ok_or_else(|| eyre!("HOME is not set"))
}

/// `$XDG_DATA_HOME`, or `~/.local/share`.
pub fn data_home(vars: &Vars<'_>) -> Result<PathBuf> {
    var_or_home(vars, "XDG_DATA_HOME", ".local/share")
}

/// The generated shell setup `share/shell` loads: `dev/shell` in the XDG
/// data dir.
pub fn shell_dir(vars: &Vars<'_>) -> Result<PathBuf> {
    Ok(data_home(vars)?.join("dev/shell"))
}

/// Where mise's installer puts mise: `$MISE_INSTALL_PATH`, or
/// `~/.local/bin/mise`.
pub fn mise_bin(vars: &Vars<'_>) -> Result<PathBuf> {
    var_or_home(vars, "MISE_INSTALL_PATH", ".local/bin/mise")
}

/// `$MISE_DATA_DIR`, or `mise` in the XDG data dir, as mise itself has it.
pub fn mise_data(vars: &Vars<'_>) -> Result<PathBuf> {
    match var(vars, "MISE_DATA_DIR") {
        Some(dir) => Ok(dir),
        None => Ok(data_home(vars)?.join("mise")),
    }
}

/// `$CARGO_HOME`, or `~/.cargo`.
pub fn cargo_home(vars: &Vars<'_>) -> Result<PathBuf> {
    var_or_home(vars, "CARGO_HOME", ".cargo")
}

/// `$RUSTUP_HOME`, or `~/.rustup`.
pub fn rustup_home(vars: &Vars<'_>) -> Result<PathBuf> {
    var_or_home(vars, "RUSTUP_HOME", ".rustup")
}

/// Where zerobrew's installer puts its binaries: `$ZEROBREW_BIN`, or
/// `~/.local/bin`.
pub fn zb_bin(vars: &Vars<'_>) -> Result<PathBuf> {
    var_or_home(vars, "ZEROBREW_BIN", ".local/bin")
}

/// zerobrew's root (its store, db, and on macOS its packages), picked the
/// way `zb` picks it: `$ZEROBREW_ROOT`, then [`OPT_ZEROBREW`] if it exists or
/// on macOS, then `zerobrew` in the XDG data dir.
///
/// `zb` takes an empty `ZEROBREW_ROOT` or `XDG_DATA_HOME` as set; dev never
/// passes one, so here they count as unset like everywhere else.
pub fn zerobrew_root(vars: &Vars<'_>, host: Host) -> Result<PathBuf> {
    if let Some(root) = var(vars, "ZEROBREW_ROOT") {
        return Ok(root);
    }
    if host.macos || host.opt_zerobrew {
        return Ok(PathBuf::from(OPT_ZEROBREW));
    }
    Ok(data_home(vars)?.join("zerobrew"))
}
