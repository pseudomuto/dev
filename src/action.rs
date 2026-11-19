//! What the GitHub Action (`action.yaml`, `share/action/dev-action.sh`)
//! needs from dev, printed as JSON by `dev action-paths`.
//!
//! The Action runs from its own checkout, so its scripts can be a different
//! release than the dev they call. Keys are only ever added, never renamed
//! or removed, and the script checks dev's version before calling this.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::{
    deps::mise,
    layout::{Host, Layout},
    shell::SETUP_FILES,
};

/// Everything the Action puts on `PATH`, exports, and caches.
#[derive(Debug, Serialize)]
pub struct Paths {
    /// Highest priority first, in the order `dev shell-init` puts them on
    /// `PATH`.
    pub path: Vec<PathEntry>,
    /// Exported to later steps, so mise's shims find the project's tools.
    pub env: BTreeMap<String, String>,
    pub mise: PathBuf,
    /// The main cache, for `actions/cache`'s `path`; `!` excludes.
    pub cache: Vec<String>,
    /// The brew cache. Empty on macOS: zerobrew lives in `/opt/zerobrew`
    /// there, and restoring into root-owned `/opt` would need sudo, which
    /// the Action doesn't use.
    pub brew_cache: Vec<String>,
    /// zerobrew's root: it exists once `dev up` installed a brew package.
    pub brew_root: PathBuf,
}

/// One `PATH` entry.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PathEntry {
    /// A dir to add if it exists.
    Dir(PathBuf),
    /// A setup file to source, adding what it puts on `PATH`.
    Setup(PathBuf),
}

/// The paths for `layout` on `host`, with dev itself in `dev_dir` and the
/// project in `project`.
pub fn paths(layout: &Layout, host: Host, dev_dir: &Path, project: &Path) -> Paths {
    let shell = |name: &str| layout.shell_dir.join(name);

    // Later setups come first on PATH. mise's setup only works from a prompt
    // hook, so its shims dir stands in for it.
    let mut path: Vec<PathEntry> = SETUP_FILES
        .iter()
        .rev()
        .map(|&name| match name {
            mise::SHELL_FILE => PathEntry::Dir(layout.mise_data.join("shims")),
            _ => PathEntry::Setup(shell(name)),
        })
        .collect();
    path.push(PathEntry::Dir(dev_dir.to_path_buf()));

    let mut env = BTreeMap::new();
    if project.join(mise::PROJECT_CONFIG).is_file() {
        env.insert(mise::CONFIG_FILENAME_VAR.to_string(), mise::PROJECT_CONFIG.to_string());
    }

    let s = |p: PathBuf| p.to_string_lossy().into_owned();
    let not = |p: PathBuf| format!("!{}", s(p));
    let zerobrew_setup = shell(crate::deps::brew::SHELL_FILE);

    // The stable toolchain ships with GitHub's images. zerobrew has its own
    // cache. dev's own mise config isn't cached: `dev up` rewrites it from
    // dev.yaml on every run.
    let toolchains = layout.rustup_home.join("toolchains");
    let cache = vec![
        s(toolchains.clone()),
        not(toolchains.join("stable-*")),
        s(layout.rustup_home.join("update-hashes")),
        s(layout.mise_bin.clone()),
        s(layout.mise_data.join("installs")),
        s(layout.mise_data.join("shims")),
        s(layout.shell_dir.clone()),
        not(zerobrew_setup.clone()),
    ];

    let brew_cache = if host.macos {
        Vec::new()
    } else {
        let mut brew: Vec<String> = ["zb", "zb-brew", "zbx"]
            .iter()
            .map(|bin| s(layout.zb_bin.join(bin)))
            .collect();
        brew.push(s(layout.zerobrew_root.clone()));
        brew.push(s(zerobrew_setup));
        brew
    };

    Paths {
        path,
        env,
        mise: layout.mise_bin.clone(),
        cache,
        brew_cache,
        brew_root: layout.zerobrew_root.clone(),
    }
}
