use std::{
    io::{self, Write},
    process::Command,
};

use crate::report::{Plain, Reporter};

/// Shared state passed to every [`Dependency`](crate::deps::Dependency) and
/// [`Task`](crate::deps::Task) operation.
///
/// Dependencies and tasks should write through the context instead of calling
/// `println!` or `eprintln!` directly. That way the caller decides where output
/// goes: the terminal in normal use, or an in-memory buffer in tests.
///
/// # Examples
///
/// Use the defaults to report progress plainly to stdout and write
/// diagnostics to stderr:
///
/// ```
/// use std::io::Write;
/// use dev::{Context, report::Status};
///
/// let mut ctx = Context::default();
/// ctx.reporter.step(0, "Install tools", Status::Applying)?;
/// writeln!(ctx.err, "something went sideways")?;
/// # Ok::<(), color_eyre::eyre::Report>(())
/// ```
///
/// Swap out a single stream and keep the defaults for the rest:
///
/// ```
/// use std::io::{self, Write};
/// use dev::Context;
///
/// let mut ctx = Context {
///     log: Box::new(io::sink()),
///     ..Context::default()
/// };
/// writeln!(ctx.log, "this goes nowhere")?;
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct Context {
    /// Shows the progress of dependencies and tasks. Defaults to
    /// [`Plain`] on stdout.
    pub reporter: Box<dyn Reporter>,

    /// Where errors and diagnostics go. Defaults to stderr.
    pub err: Box<dyn Write + Send>,

    /// Where the output of commands dev runs goes (see
    /// [`process::run`](crate::process::run)), so the terminal only has
    /// dev's own progress. `dev up` and `dev down` point it at a file in
    /// `.dev/log`. Defaults to stderr.
    pub log: Box<dyn Write + Send>,

    /// Set on every process `dev` starts, e.g. a project's `env`. Defaults to
    /// none.
    pub env: Vec<(String, String)>,
}

impl Context {
    /// Builds a command for `program` with [`Context::env`] set. Every process
    /// `dev` starts must come from here, so none miss the project's env;
    /// clippy rejects `Command::new` anywhere else.
    #[allow(clippy::disallowed_methods)]
    pub fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut cmd = Command::new(program);
        cmd.envs(self.env.iter().map(|(k, v)| (k, v)));
        cmd
    }
}

impl Default for Context {
    /// Creates a context that reports progress plainly on stdout, with
    /// diagnostics and command output on stderr.
    fn default() -> Self {
        Self {
            reporter: Box::new(Plain::new(Box::new(io::stdout()))),
            err: Box::new(io::stderr()),
            log: Box::new(io::stderr()),
            env: Vec::new(),
        }
    }
}
