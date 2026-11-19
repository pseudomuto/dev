//! Finding the tools dev bootstraps (mise, rustup) on `PATH`, and deciding
//! who looks after the one that's found.

use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use color_eyre::eyre::{WrapErr, bail};

use crate::{Context, Result, process};

/// Which binary to use, and who looks after it.
#[derive(Clone, Debug, PartialEq)]
pub enum Found {
    /// Not installed. dev will install it here.
    Missing(PathBuf),
    /// Installed where dev installs it, so dev looks after it.
    Managed(PathBuf),
    /// Installed by something else, which looks after it.
    External(PathBuf),
}

impl Found {
    pub fn path(&self) -> &Path {
        match self {
            Found::Missing(p) | Found::Managed(p) | Found::External(p) => p,
        }
    }
}

/// `name` on `search` (`PATH` syntax), compared with where dev installs it
/// (`managed`). The first one on `search` wins, as it would in a shell.
pub fn locate(search: &OsStr, name: &str, managed: &Path) -> Found {
    match find_on_path(search, name) {
        Some(path) if path == managed => Found::Managed(path),
        Some(path) => Found::External(path),
        None if is_executable(managed) => Found::Managed(managed.to_path_buf()),
        None => Found::Missing(managed.to_path_buf()),
    }
}

/// The `PATH` commands from `ctx` run with: the one in `ctx.env` (a
/// project's env, or what [`prepend_path`](super::prepend_path) added),
/// else dev's own, else empty. Binaries are looked up here, so dev uses the
/// same one a command would.
pub fn search_path(ctx: &Context) -> OsString {
    match ctx.env.iter().find(|(name, _)| name == "PATH") {
        Some((_, value)) => OsString::from(value),
        None => env::var_os("PATH").unwrap_or_default(),
    }
}

/// The first executable `name` in `path` (`PATH` syntax). Relative entries
/// are skipped, so where dev runs from can't change which binary it finds.
pub fn find_on_path(path: &OsStr, name: &str) -> Option<PathBuf> {
    env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

pub fn is_executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// An installer script, fetched and piped into a shell.
pub struct Script<'a> {
    /// Names it in errors, e.g. `mise` in "mise installer (<url>) failed".
    pub what: &'a str,
    /// Any URL curl understands, including `file://`.
    pub url: &'a str,
    /// `sh` or `bash`, whichever the script is written for.
    pub shell: &'a str,
    /// Passed to the script.
    pub args: &'a [&'a str],
    pub env: &'a [(&'a str, &'a OsStr)],
    /// Keeps stdin attached, for installers that may run sudo.
    pub interactive: bool,
    /// What the script installs: it must be executable afterwards.
    pub expect: &'a Path,
}

/// Runs `script` as `curl -fsSL <url> | <shell> -s -- <args>`, with its output
/// going to `ctx.log`, and checks it installed what it should have.
pub fn run(ctx: &mut Context, script: Script<'_>) -> Result<()> {
    // pipefail matters: without it, a failed curl pipes nothing into the
    // shell, which then exits 0 and the install looks like it succeeded.
    let mut cmd = ctx.command("bash");
    cmd.args([
        "-c",
        r#"set -o pipefail; url=$1; sh=$2; shift 2; curl -fsSL "$url" | "$sh" -s -- "$@""#,
        "bash",
        script.url,
        script.shell,
    ])
    .args(script.args)
    .envs(script.env.iter().copied());

    let ran = if script.interactive {
        process::run_interactive(ctx, &mut cmd)
    } else {
        process::run(ctx, &mut cmd)
    };
    ran.wrap_err_with(|| format!("{} installer ({}) failed", script.what, script.url))?;

    if !is_executable(script.expect) {
        let bin = script.expect.file_name().map_or_else(
            || script.expect.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        bail!(
            "the {} installer ({}) finished, but there's no {bin} at {}",
            script.what,
            script.url,
            script.expect.display()
        );
    }
    Ok(())
}
