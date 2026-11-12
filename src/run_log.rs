//! Logs of the commands `dev up` and `dev down` run, one file per run, in
//! `<project>/.dev/log`.

use std::{
    fs::{self, File},
    io::ErrorKind,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    process,
    time::{SystemTime, UNIX_EPOCH},
};

use color_eyre::eyre::WrapErr;

use crate::Result;

/// How many runs' logs to keep, the current one included.
pub const KEEP: usize = 5;

/// Points at the newest log, e.g. for `less .dev/log/latest`.
const LATEST: &str = "latest";

/// The log for one run.
pub struct RunLog {
    /// e.g. `.dev/log/1762956181123-4242-up.log`.
    pub path: PathBuf,
    pub file: File,
}

impl RunLog {
    /// Creates a log for a run of `command` (e.g. `up`) in
    /// `<root>/.dev/log`, points `latest` at it, and deletes all but the
    /// newest `keep` logs.
    ///
    /// Also writes `<root>/.dev/.gitignore` ignoring everything, unless one is
    /// already there, so `.dev` never shows up in git.
    pub fn create(root: &Path, command: &str, keep: usize) -> Result<Self> {
        let dev_dir = root.join(".dev");
        let dir = dev_dir.join("log");
        fs::create_dir_all(&dir).wrap_err_with(|| format!("failed to create {}", dir.display()))?;

        let gitignore = dev_dir.join(".gitignore");
        if !gitignore.exists() {
            fs::write(&gitignore, "*\n").wrap_err_with(|| format!("failed to write {}", gitignore.display()))?;
        }

        // Zero-padded, so sorting by name sorts by time. The pid keeps runs
        // that start in the same millisecond apart.
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .wrap_err("the system clock is before 1970")?
            .as_millis();
        let name = format!("{millis:013}-{}-{command}.log", process::id());
        let path = dir.join(&name);
        let file = File::options()
            .write(true)
            .create_new(true)
            .open(&path)
            .wrap_err_with(|| format!("failed to create {}", path.display()))?;

        point_latest_at(&dir, &name)?;
        prune(&dir, keep)?;
        Ok(Self { path, file })
    }
}

/// Points `<dir>/latest` at `name`. The link is made under a temp name and
/// renamed into place, which replaces the old one atomically.
fn point_latest_at(dir: &Path, name: &str) -> Result<()> {
    let latest = dir.join(LATEST);
    let tmp = dir.join(format!(".{LATEST}-{}", process::id()));

    match fs::remove_file(&tmp) {
        Err(e) if e.kind() != ErrorKind::NotFound => {
            return Err(e).wrap_err_with(|| format!("failed to remove {}", tmp.display()));
        }
        _ => {}
    }
    symlink(name, &tmp).wrap_err_with(|| format!("failed to create {}", tmp.display()))?;
    fs::rename(&tmp, &latest).wrap_err_with(|| format!("failed to replace {}", latest.display()))
}

/// Deletes all but the newest `keep` logs in `dir`. Other files are left
/// alone.
fn prune(dir: &Path, keep: usize) -> Result<()> {
    let mut logs: Vec<PathBuf> = fs::read_dir(dir)
        .wrap_err_with(|| format!("failed to read {}", dir.display()))?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<std::io::Result<_>>()
        .wrap_err_with(|| format!("failed to read {}", dir.display()))?;
    logs.retain(|p| p.extension().is_some_and(|ext| ext == "log") && p.is_file());
    logs.sort();

    let extra = logs.len().saturating_sub(keep);
    for old in &logs[..extra] {
        match fs::remove_file(old) {
            // Another run pruned it first.
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            res => res.wrap_err_with(|| format!("failed to delete old log {}", old.display()))?,
        }
    }
    Ok(())
}
