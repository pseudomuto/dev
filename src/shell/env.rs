//! Loads `dev.yaml` env into an interactive shell, and takes it back out.
//!
//! `share/shell` runs `dev hook-env` before every prompt and evals what it
//! prints. The env comes from layers of `dev.yaml` files, outermost first:
//!
//! 1. `${XDG_CONFIG_HOME:-$HOME/.config}/dev.yaml`, everywhere
//! 2. each `dev.yaml` from just below `$HOME` down to `$PWD`
//!
//! Outside `$HOME`, only `$PWD/dev.yaml` is checked, never all the way to `/`.
//!
//! Later layers override earlier ones, and `${VAR}` in a layer sees what the
//! layers before it set. `${PWD}` is the layer's own directory (`$HOME` for
//! the global file), so `${PWD}/bin` means the same thing from any subdir.
//!
//! What the shell has applied lives in [`STATE_VAR`]: the layers, and each
//! var's value from before dev touched it (the baseline). Every change starts
//! over from the baseline, so leaving a dir needs no unwinding and
//! `PATH: "${PWD}/bin:${PATH}"` never grows.

use std::{
    collections::{BTreeMap, BTreeSet},
    env::VarError,
    fs,
    io,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use color_eyre::eyre::{WrapErr, bail, eyre};
use serde::{Deserialize, Serialize};

use crate::{Result, project::Project};

/// The exported var holding the applied [`State`] as JSON. Must match
/// `share/shell`.
pub const STATE_VAR: &str = "__DEV_ENV";

/// The project file looked for in each dir.
const FILE: &str = "dev.yaml";

/// What the shell has applied.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// Applied in this order.
    pub layers: Vec<Layer>,
    /// Each var the layers set, and its value before that. `None` means it
    /// was unset.
    pub baseline: BTreeMap<String, Option<String>>,
}

/// One applied `dev.yaml`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub path: PathBuf,
    /// What `${PWD}` means in this file.
    pub root: PathBuf,
    /// Nanoseconds since the epoch. A change means the file was edited.
    pub modified: u64,
}

impl Layer {
    /// The layer for `path`, or `None` if there's no such file.
    fn find(path: PathBuf, root: PathBuf) -> Result<Option<Self>> {
        let meta = match fs::metadata(&path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).wrap_err_with(|| format!("failed to read {}", path.display())),
        };
        let modified = meta
            .modified()
            .wrap_err_with(|| format!("failed to read the mtime of {}", path.display()))?
            .duration_since(UNIX_EPOCH)
            .wrap_err_with(|| format!("{} was modified before 1970", path.display()))?
            .as_nanos()
            .try_into()
            .wrap_err_with(|| format!("the mtime of {} doesn't fit in 64 bits", path.display()))?;
        Ok(Some(Self { path, root, modified }))
    }
}

/// The sh code that brings a shell with `env` up to date, or `None` when it
/// already is.
///
/// Reads `PWD`, `HOME`, `XDG_CONFIG_HOME`, and [`STATE_VAR`] from `env`.
pub fn hook(env: &BTreeMap<String, String>) -> Result<Option<String>> {
    let home = non_empty(env, "HOME").ok_or_else(|| eyre!("HOME is not set"))?;
    let pwd = non_empty(env, "PWD").ok_or_else(|| eyre!("PWD is not set"))?;
    let config = non_empty(env, "XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));

    let state = state(env)?;
    let layers = layers(&pwd, &home, &config.join(FILE))?;
    render(env, state.as_ref(), layers)
}

/// Whether the shell with `env` already has the env of the `dev.yaml` at
/// `path`, as it is on disk now.
///
/// Paths are compared after resolving symlinks, since layers come from the
/// shell's `$PWD` and `path` may not.
pub fn applied(env: &BTreeMap<String, String>, path: &Path) -> Result<bool> {
    let Some(state) = state(env)? else {
        return Ok(false);
    };
    let Ok(path) = fs::canonicalize(path) else {
        return Ok(false);
    };
    let Some(layer) = state
        .layers
        .iter()
        .find(|layer| fs::canonicalize(&layer.path).is_ok_and(|p| p == path))
    else {
        return Ok(false);
    };
    let now = Layer::find(layer.path.clone(), layer.root.clone())?;
    Ok(now.as_ref() == Some(layer))
}

/// The layers that apply in `pwd`, outermost first. `global` is the user's
/// own `dev.yaml`.
pub fn layers(pwd: &Path, home: &Path, global: &Path) -> Result<Vec<Layer>> {
    let mut found = Vec::new();
    if let Some(layer) = Layer::find(global.to_path_buf(), home.to_path_buf())? {
        found.push(layer);
    }

    let dirs: Vec<&Path> = if pwd.starts_with(home) {
        pwd.ancestors().take_while(|dir| *dir != home).collect()
    } else {
        vec![pwd]
    };
    for dir in dirs.into_iter().rev() {
        if let Some(layer) = Layer::find(dir.join(FILE), dir.to_path_buf())? {
            found.push(layer);
        }
    }
    Ok(found)
}

/// The sh code that takes a shell with `env` and `state` applied to `layers`,
/// or `None` when it's already there.
pub fn render(env: &BTreeMap<String, String>, state: Option<&State>, layers: Vec<Layer>) -> Result<Option<String>> {
    let empty = State::default();
    let state = state.unwrap_or(&empty);
    if state.layers == layers {
        return Ok(None);
    }

    // The env as it was before dev touched it.
    let mut restored = env.clone();
    for (name, value) in &state.baseline {
        match value {
            Some(value) => restored.insert(name.clone(), value.clone()),
            None => restored.remove(name),
        };
    }

    let mut current = restored.clone();
    let mut touched = BTreeSet::new();
    for layer in &layers {
        let lookup = |var: &str| match var {
            "PWD" => layer
                .root
                .to_str()
                .map(str::to_string)
                .ok_or_else(|| VarError::NotUnicode(layer.root.clone().into_os_string())),
            _ => current.get(var).cloned().ok_or(VarError::NotPresent),
        };
        let project = Project::load_with(&layer.path, &lookup)?;

        for (name, value) in project.env {
            if !is_identifier(&name) {
                bail!(
                    "env `{name}` in {} isn't a valid shell variable name",
                    layer.path.display()
                );
            }
            current.insert(name.clone(), value);
            touched.insert(name);
        }
    }

    let mut script = String::new();
    for (name, value) in &state.baseline {
        if touched.contains(name) {
            continue;
        }
        match value {
            Some(value) => script.push_str(&format!("export {name}={}\n", quote(value))),
            None => script.push_str(&format!("unset {name}\n")),
        }
    }
    for name in &touched {
        script.push_str(&format!("export {name}={}\n", quote(&current[name])));
    }

    if layers.is_empty() {
        script.push_str(&format!("unset {STATE_VAR}\n"));
    } else {
        let baseline = touched
            .into_iter()
            .map(|name| {
                let before = restored.get(&name).cloned();
                (name, before)
            })
            .collect();
        let json = serde_json::to_string(&State { layers, baseline })?;
        script.push_str(&format!("export {STATE_VAR}={}\n", quote(&json)));
    }
    Ok(Some(script))
}

/// Parses [`STATE_VAR`] from `env`, if it's set.
fn state(env: &BTreeMap<String, String>) -> Result<Option<State>> {
    env.get(STATE_VAR)
        .map(|json| {
            serde_json::from_str(json).wrap_err_with(|| format!("{STATE_VAR} isn't valid; unset it to start over"))
        })
        .transpose()
}

fn non_empty(env: &BTreeMap<String, String>, name: &str) -> Option<PathBuf> {
    env.get(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Whether `name` can be `export`ed as is: a letter or `_`, then letters,
/// digits, and `_`.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Single-quotes `value` for sh, so nothing in it is expanded.
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}
