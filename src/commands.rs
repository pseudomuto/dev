use std::os::unix::process::ExitStatusExt;

use color_eyre::eyre::{WrapErr, bail};

use crate::{Context, Result, project::Project};

/// Runs the project command `name` from `dev.yaml` and returns its exit code.
///
/// The snippet runs as `sh -c '<snippet>' <name> <args...>`, so `$0` is the
/// command's name and `$@` its args. It's attached to the terminal directly,
/// not through `ctx.out`/`ctx.err`, so colors, prompts, and TTY checks work.
pub fn run_command(ctx: &Context, project: &Project, name: &str, args: &[String]) -> Result<i32> {
    let Some(snippet) = project.commands.get(name) else {
        if project.commands.is_empty() {
            bail!("unknown command `{name}`; dev.yaml has no commands");
        }
        let names: Vec<&str> = project.commands.keys().map(String::as_str).collect();
        bail!("unknown command `{name}`; dev.yaml has: {}", names.join(", "));
    };

    let status = ctx
        .command("sh")
        .arg("-c")
        .arg(snippet)
        .arg(name)
        .args(args)
        .status()
        .wrap_err_with(|| format!("failed to start command `{name}`"))?;

    // A command killed by a signal has no code; report it the way shells do.
    Ok(status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(0)))
}
