use std::{
    collections::VecDeque,
    io::{self, BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
};

use color_eyre::eyre::{WrapErr, bail};

use crate::{Context, Result, report::Reporter};

/// How many lines of a failed command's output its error includes.
const TAIL_LINES: usize = 20;

/// Runs `cmd` to completion, writing a `$ <command>` line and then its
/// stdout and stderr, in the order it wrote them, to `ctx.log`.
///
/// stdin is `/dev/null`, so a command that prompts fails instead of waiting
/// for an answer nobody can see. Use [`run_interactive`] for one that has to
/// prompt, like sudo.
///
/// Returns an error if the command can't be started, its output can't be
/// copied, or it exits with a non-zero status. The error includes the last
/// lines of its output, since `ctx.log` usually isn't on screen.
pub fn run(ctx: &mut Context, cmd: &mut Command) -> Result<()> {
    run_logged(ctx, cmd.stdin(Stdio::null()))
}

/// Like [`run`], but stdin stays attached to dev's, so the command can
/// prompt, e.g. sudo asking for a password. The reporter is paused
/// meanwhile, so it doesn't draw over the prompt.
pub fn run_interactive(ctx: &mut Context, cmd: &mut Command) -> Result<()> {
    ctx.reporter.pause()?;
    let res = run_logged(ctx, cmd.stdin(Stdio::inherit()));
    let resumed = ctx.reporter.resume();
    // The command's failure is the one worth reporting.
    res?;
    resumed
}

fn run_logged(ctx: &mut Context, cmd: &mut Command) -> Result<()> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    writeln!(ctx.log, "$ {}", display(cmd)).wrap_err("failed to write to the log")?;

    // One pipe for both streams keeps the command's own ordering, and with a
    // single reader there's no pipe left unread to fill up and block it.
    let (reader, writer) = io::pipe().wrap_err("failed to create a pipe for command output")?;
    let writer_err = writer
        .try_clone()
        .wrap_err("failed to create a pipe for command output")?;
    let spawned = cmd.stdout(writer).stderr(writer_err).spawn();
    // `cmd` holds its copies of the write end until they're replaced. Drop
    // them now, or the reader below never sees the end of the output.
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = spawned.wrap_err_with(|| format!("failed to start `{program}`"))?;

    let tail = match copy_output(reader, &mut ctx.log, ctx.reporter.as_mut()) {
        Ok(tail) => tail,
        Err(e) => {
            // Don't leave it running, or blocked on a pipe nobody reads.
            stop(&mut child);
            return Err(e).wrap_err_with(|| format!("failed to copy output from `{program}` to the log"));
        }
    };

    let status = child
        .wait()
        .wrap_err_with(|| format!("failed to wait for `{program}`"))?;
    if !status.success() {
        if tail.is_empty() {
            bail!("`{program}` failed with {status}");
        }
        bail!("`{program}` failed with {status}. Last output:\n{}", tail.join("\n"));
    }
    Ok(())
}

/// Copies `reader` to `log` line by line, ending it with a newline if it
/// didn't, and returns the last [`TAIL_LINES`] lines, indented, for an error
/// message. Each line also goes to `reporter`, as a terminal would show it.
fn copy_output(reader: impl io::Read, log: &mut dyn Write, reporter: &mut dyn Reporter) -> Result<Vec<String>> {
    let mut reader = BufReader::new(reader);
    let mut tail = VecDeque::with_capacity(TAIL_LINES);
    let mut line = Vec::new();
    let mut ends_in_newline = true;

    while reader.read_until(b'\n', &mut line)? > 0 {
        log.write_all(&line)?;
        ends_in_newline = line.ends_with(b"\n");

        let text = String::from_utf8_lossy(&line);
        let text = text.trim_end_matches(['\n', '\r']);
        // A progress bar redraws itself after each `\r`; only the last
        // drawing is what a terminal would show.
        let shown = text.rsplit('\r').next().unwrap_or_default();
        reporter.output(shown)?;
        if tail.len() == TAIL_LINES {
            tail.pop_front();
        }
        tail.push_back(format!("    {shown}"));
        line.clear();
    }

    // So whatever dev logs next starts on its own line.
    if !ends_in_newline {
        log.write_all(b"\n")?;
    }
    log.flush()?;
    Ok(tail.into())
}

/// Kills `child` and reaps it, ignoring failures: this only runs while
/// already handling an error, which is the one worth reporting.
fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// `cmd` as a shell-like line for the log, e.g. `zb install jq`. Arguments
/// with spaces or quotes are shown quoted.
fn display(cmd: &Command) -> String {
    std::iter::once(cmd.get_program())
        .chain(cmd.get_args())
        .map(|arg| {
            let arg = arg.to_string_lossy();
            if arg.is_empty() || arg.contains(|c: char| c.is_whitespace() || c == '\'' || c == '"') {
                format!("'{}'", arg.replace('\'', r"'\''"))
            } else {
                arg.into_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
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
