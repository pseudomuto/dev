use std::{
    io::{self, Write},
    process::Command,
};

/// Shared state passed to every [`Dependency`](crate::deps::Dependency) and
/// [`Task`](crate::deps::Task) operation.
///
/// Dependencies and tasks should write through the context instead of calling
/// `println!` or `eprintln!` directly. That way the caller decides where output
/// goes: the terminal in normal use, or an in-memory buffer in tests.
///
/// # Examples
///
/// Use the defaults to write to stdout and stderr:
///
/// ```
/// use std::io::Write;
/// use dev::Context;
///
/// let mut ctx = Context::default();
/// writeln!(ctx.out, "installing...")?;
/// writeln!(ctx.err, "something went sideways")?;
/// # Ok::<(), std::io::Error>(())
/// ```
///
/// Swap out a single stream and keep the default for the other:
///
/// ```
/// use std::io::{self, Write};
/// use dev::Context;
///
/// let mut ctx = Context {
///     out: Box::new(io::sink()),
///     ..Context::default()
/// };
/// writeln!(ctx.out, "this goes nowhere")?;
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct Context {
    /// Where normal output goes. Defaults to stdout.
    pub out: Box<dyn Write + Send>,

    /// Where errors and diagnostics go. Defaults to stderr.
    pub err: Box<dyn Write + Send>,

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
    /// Creates a context that writes to the process's stdout and stderr.
    fn default() -> Self {
        Self {
            out: Box::new(io::stdout()),
            err: Box::new(io::stderr()),
            env: Vec::new(),
        }
    }
}
