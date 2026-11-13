use std::{env, ffi::OsString, path::Path};

use color_eyre::eyre::{WrapErr, eyre};

use crate::{
    Context,
    Result,
    report::{self, Status},
};

pub mod brew;
pub mod mise;
pub mod packages;
pub mod rust;

pub trait Dependency {
    fn name(&self) -> &str;
    fn desc(&self) -> &str;
    fn examples(&self) -> Vec<Example> {
        Vec::new()
    }

    /// The steps that make up this dependency, in order. Empty means it's met
    /// by calling [`Dependency::meet`] directly.
    fn tasks(&self) -> &[Box<dyn Task>] {
        &[]
    }

    fn meet(&self, ctx: &mut Context) -> Result<()>;
    fn is_met(&self, ctx: &mut Context) -> Result<bool>;
    fn down(&self, ctx: &mut Context) -> Result<()>;
}

/// Applies each of `tasks` that isn't already applied, in order, reporting
/// each one's progress to `ctx.reporter` as a step at depth 1. Stops at the
/// first failure, naming the task.
///
/// Each task is checked just before it's applied, since later tasks can rely
/// on earlier ones (e.g. packages need zerobrew).
///
/// For dependencies made of tasks, as their [`Dependency::meet`].
pub fn meet_tasks(tasks: &[Box<dyn Task>], ctx: &mut Context) -> Result<()> {
    for task in tasks {
        let name = task.name();
        report::step(ctx, 1, name, Status::Checking)?;
        let applied = task.is_applied(ctx);
        let applied =
            report::failing(ctx, 1, name, applied).wrap_err_with(|| format!("failed to check task: {name}"))?;

        if applied {
            report::step(ctx, 1, name, Status::Met)?;
            continue;
        }

        report::step(ctx, 1, name, Status::Applying)?;
        let res = task.apply(ctx);
        report::failing(ctx, 1, name, res).wrap_err_with(|| format!("task failed: {name}"))?;
        report::step(ctx, 1, name, Status::Done)?;
    }

    Ok(())
}

/// Returns true when every one of `tasks` is applied. Stops at the first one
/// that isn't, so later (possibly slow) checks are skipped.
///
/// For dependencies made of tasks, as their [`Dependency::is_met`].
pub fn tasks_met(tasks: &[Box<dyn Task>], ctx: &mut Context) -> Result<bool> {
    for task in tasks {
        let applied = task
            .is_applied(ctx)
            .wrap_err_with(|| format!("failed to check task: {}", task.name()))?;

        if !applied {
            return Ok(false);
        }
    }

    Ok(true)
}

/// Puts `dir` first on the `PATH` that `ctx` gives commands, unless it's
/// already on it. Starts from a `PATH` the project env set, or else dev's
/// own.
pub(crate) fn prepend_path(ctx: &mut Context, dir: &Path) -> Result<()> {
    let current = match ctx.env.iter().find(|(name, _)| name == "PATH") {
        Some((_, value)) => OsString::from(value),
        None => env::var_os("PATH").unwrap_or_default(),
    };
    if env::split_paths(&current).any(|p| p == dir) {
        return Ok(());
    }

    let joined = env::join_paths(std::iter::once(dir.to_path_buf()).chain(env::split_paths(&current)))
        .wrap_err_with(|| format!("can't add {} to PATH", dir.display()))?;
    let joined = joined
        .into_string()
        .map_err(|p| eyre!("PATH isn't valid UTF-8: {p:?}"))?;

    ctx.env.retain(|(name, _)| name != "PATH");
    ctx.env.push(("PATH".to_string(), joined));
    Ok(())
}

pub trait Task {
    fn name(&self) -> &str;
    fn apply(&self, ctx: &mut Context) -> Result<()>;
    fn is_applied(&self, ctx: &mut Context) -> Result<bool>;
}

pub struct Example {
    pub name: String,
    pub yaml: String,
}

impl Example {
    pub fn new<T: Into<String>>(name: T, yaml: T) -> Self {
        Self {
            name: name.into(),
            yaml: yaml.into(),
        }
    }
}
