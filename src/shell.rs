//! The per-user shell setup `dev` generates, which `share/shell` loads.
//!
//! Each tool gets a file in the directory, e.g. `zerobrew`, holding its own
//! setup. Every write also replaces `.stamp` with a new token. Shells check the
//! stamp before each prompt and reload the setup when it changed, so open
//! shells pick up changes without re-sourcing anything.

use std::{
    env,
    fs,
    io::Write,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use color_eyre::eyre::{WrapErr, eyre};

/// The file shells read to tell whether the setup changed. Must match
/// `share/shell`.
const STAMP: &str = ".stamp";

/// A directory of generated shell setup.
pub struct ShellDir {
    path: PathBuf,
}

impl ShellDir {
    /// Uses `path` as the directory.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// `$XDG_DATA_HOME/dev/shell`, or `~/.local/share/dev/shell`. Must match
    /// `share/shell`.
    pub fn user_default() -> crate::Result<Self> {
        Ok(Self::new(data_home()?.join("dev/shell")))
    }

    /// Where the setup called `name` lives.
    pub fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }

    /// The current stamp, e.g. `"1762880000123456789"`.
    pub fn stamp(&self) -> crate::Result<String> {
        let path = self.file(STAMP);
        fs::read_to_string(&path).wrap_err_with(|| format!("failed to read {}", path.display()))
    }

    /// Replaces the setup called `name` with `contents`, then the stamp.
    ///
    /// Both are written to a temp file and renamed into place, so a shell
    /// never reads half a file. The stamp goes last, so a shell that sees a
    /// new stamp also sees the new setup.
    pub fn write(&self, name: &str, contents: &str) -> crate::Result<()> {
        fs::create_dir_all(&self.path).wrap_err_with(|| format!("failed to create {}", self.path.display()))?;
        self.replace(name, contents)?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .wrap_err("the system clock is before 1970")?;
        self.replace(STAMP, &format!("{}\n", now.as_nanos()))
    }

    /// Writes `contents` to a temp file in the directory, then renames it to
    /// `name`. Renames within a directory are atomic.
    fn replace(&self, name: &str, contents: &str) -> crate::Result<()> {
        let path = self.file(name);
        let mut tmp = tempfile::NamedTempFile::new_in(&self.path)
            .wrap_err_with(|| format!("failed to create a temp file in {}", self.path.display()))?;
        tmp.write_all(contents.as_bytes())
            .wrap_err_with(|| format!("failed to write {}", tmp.path().display()))?;
        tmp.persist(&path)
            .wrap_err_with(|| format!("failed to replace {}", path.display()))?;
        Ok(())
    }
}

/// `$HOME`, failing when it's unset or empty.
pub fn home() -> crate::Result<PathBuf> {
    env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| eyre!("HOME is not set"))
}

/// `$XDG_DATA_HOME`, or `~/.local/share` when it's unset or empty.
pub fn data_home() -> crate::Result<PathBuf> {
    match env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()) {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => Ok(home()?.join(".local/share")),
    }
}
