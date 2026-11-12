//! How `dev up` and `dev down` show progress.
//!
//! Dependencies and tasks report what they're doing as [`Status`] changes,
//! and a [`Reporter`] decides how that looks. [`Plain`] prints one line per
//! change, for pipes, CI, and tests. [`clack::Clack`] draws spinners on a
//! terminal.

pub mod clack;

use std::{fmt, io::Write, path::Path};

use color_eyre::Report;

use crate::{Context, Result};

/// Where a dependency (depth 0) or one of its tasks (depth 1) is at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Finding out whether there's anything to do.
    Checking,
    /// Nothing to do.
    Met,
    /// Being met or applied.
    Applying,
    /// Met or applied without errors.
    Done,
    /// Meeting, applying, or taking it down failed.
    Failed,
    /// Being taken down.
    TakingDown,
    /// Taken down.
    Down,
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Status::Checking => "checking",
            Status::Met => "met",
            Status::Applying => "applying",
            Status::Done => "done",
            Status::Failed => "failed",
            Status::TakingDown => "taking down",
            Status::Down => "down",
        })
    }
}

/// Shows the progress of a `dev up` or `dev down` run.
pub trait Reporter: Send {
    /// Called once, before any step, with the `dev` command being run (e.g.
    /// `up`) and the project's name.
    fn start(&mut self, command: &str, project: &str) -> Result<()>;

    /// A dependency (`depth` 0) or one of its tasks (`depth` 1) moved to
    /// `status`.
    fn step(&mut self, depth: usize, name: &str, status: Status) -> Result<()>;

    /// A line of output from a command the current step is running.
    fn output(&mut self, line: &str) -> Result<()>;

    /// Stops drawing, so a command can use the terminal (e.g. sudo asking
    /// for a password), until [`Reporter::resume`].
    fn pause(&mut self) -> Result<()>;

    /// Starts drawing again after [`Reporter::pause`].
    fn resume(&mut self) -> Result<()>;

    /// Called once, after the last step. `ok` says whether the run
    /// succeeded.
    fn finish(&mut self, ok: bool) -> Result<()>;

    /// Shows `err`, the error a failed run ended with, and `log`, the file
    /// with its full output. Returns false if it didn't, so the caller has
    /// to.
    fn error(&mut self, err: &Report, log: &Path) -> Result<bool>;
}

/// Prints a line when a step is met, starts applying, or is taken down, e.g.
/// `  Installing jq ... applying`. Everything else is left out, so the output
/// reads the same in a log or a CI run.
pub struct Plain {
    out: Box<dyn Write + Send>,
}

impl Plain {
    pub fn new(out: Box<dyn Write + Send>) -> Self {
        Self { out }
    }
}

impl Reporter for Plain {
    fn start(&mut self, _command: &str, _project: &str) -> Result<()> {
        Ok(())
    }

    fn step(&mut self, depth: usize, name: &str, status: Status) -> Result<()> {
        if let Status::Met | Status::Applying | Status::Down = status {
            writeln!(self.out, "{}{name} ... {status}", "  ".repeat(depth))?;
        }
        Ok(())
    }

    fn output(&mut self, _line: &str) -> Result<()> {
        Ok(())
    }

    fn pause(&mut self) -> Result<()> {
        Ok(())
    }

    fn resume(&mut self) -> Result<()> {
        Ok(())
    }

    fn finish(&mut self, _ok: bool) -> Result<()> {
        Ok(())
    }

    /// Leaves the error to the caller, which prints it in full.
    fn error(&mut self, _err: &Report, _log: &Path) -> Result<bool> {
        Ok(false)
    }
}

/// Reports `name` (a dependency at `depth` 0, a task at 1) moving to
/// `status`. Every step goes through here.
///
/// Applying also starts a section in `ctx.log` (`== Installing jq`), so the
/// output of the commands that follow is easy to find.
pub(crate) fn step(ctx: &mut Context, depth: usize, name: &str, status: Status) -> Result<()> {
    ctx.reporter.step(depth, name, status)?;
    if let Status::Applying = status {
        writeln!(ctx.log, "== {name}")?;
    }

    Ok(())
}

/// Passes `res` through, reporting `name` as [`Status::Failed`] first when
/// it's an error.
pub(crate) fn failing<T>(ctx: &mut Context, depth: usize, name: &str, res: Result<T>) -> Result<T> {
    if res.is_err() {
        // The original error is the one worth returning, so a failure to
        // report this one is dropped.
        let _ = step(ctx, depth, name, Status::Failed);
    }

    res
}
