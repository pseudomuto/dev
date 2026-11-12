use std::{
    io,
    process::{Command, Stdio},
    thread,
};

use color_eyre::eyre::{WrapErr, bail};

use crate::{Context, Result};

/// Runs `cmd` to completion, copying its stdout to `ctx.out` and its stderr
/// to `ctx.err` as it runs.
///
/// stdin is left as configured on `cmd` (inherited by default), so commands
/// that prompt, like sudo, still work.
///
/// Returns an error if the command can't be started, its output can't be
/// copied, or it exits with a non-zero status.
pub fn run(ctx: &mut Context, cmd: &mut Command) -> Result<()> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .wrap_err_with(|| format!("failed to start `{program}`"))?;

    let mut child_out = child.stdout.take().expect("stdout was piped");
    let mut child_err = child.stderr.take().expect("stderr was piped");
    let Context { out, err, .. } = ctx;

    // Copy both streams at once; reading them one after the other can
    // deadlock if the command fills the pipe we aren't reading.
    let (out_res, err_res) = thread::scope(|s| {
        let out_thread = s.spawn(|| io::copy(&mut child_out, out));
        let err_res = io::copy(&mut child_err, err);
        (out_thread.join().expect("stdout copy thread panicked"), err_res)
    });

    let status = child
        .wait()
        .wrap_err_with(|| format!("failed to wait for `{program}`"))?;
    out_res.wrap_err_with(|| format!("failed to copy stdout from `{program}`"))?;
    err_res.wrap_err_with(|| format!("failed to copy stderr from `{program}`"))?;

    if !status.success() {
        bail!("`{program}` failed with {status}");
    }
    Ok(())
}

/// Runs `cmd` and returns its stdout, failing with its stderr on a non-zero
/// exit. `display` is how the command is named in errors.
pub fn output(cmd: &mut Command, display: &str) -> Result<String> {
    let output = cmd.output().wrap_err_with(|| format!("failed to run `{display}`"))?;

    if !output.status.success() {
        bail!(
            "`{display}` failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
