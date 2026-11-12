use std::{fmt, io::Write};

use color_eyre::eyre::WrapErr;

use crate::{Context, Result};

pub mod brew;
pub mod mise;

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

/// What a status line says about a dependency or task.
#[derive(Clone, Copy)]
pub(crate) enum Status {
    /// Nothing to do.
    Met,
    /// About to be met or applied.
    Applying,
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Status::Met => "met",
            Status::Applying => "applying",
        })
    }
}

/// How far task status lines are indented under their dependency's.
const TASK_INDENT: &str = "  ";

/// Writes a status line to `ctx.out`, e.g. `  Installing jq ... applying`.
/// Every status line goes through here, so they all look the same.
pub(crate) fn report(ctx: &mut Context, indent: &str, name: &str, status: Status) -> Result<()> {
    writeln!(ctx.out, "{indent}{name} ... {status}")?;
    Ok(())
}

/// Applies each of `tasks` that isn't already applied, in order, writing a
/// status line for each to `ctx.out`. Stops at the first failure, naming the
/// task.
///
/// Each task is checked just before it's applied, since later tasks can rely
/// on earlier ones (e.g. packages need zerobrew).
///
/// For dependencies made of tasks, as their [`Dependency::meet`].
pub fn meet_tasks(tasks: &[Box<dyn Task>], ctx: &mut Context) -> Result<()> {
    for task in tasks {
        let applied = task
            .is_applied(ctx)
            .wrap_err_with(|| format!("failed to check task: {}", task.name()))?;

        if applied {
            report(ctx, TASK_INDENT, task.name(), Status::Met)?;
        } else {
            report(ctx, TASK_INDENT, task.name(), Status::Applying)?;
            task.apply(ctx)
                .wrap_err_with(|| format!("task failed: {}", task.name()))?;
        }
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
