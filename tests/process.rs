mod common;

use common::capturing_ctx;
use dev::{Result, process};

#[test]
fn run_copies_stdout_and_stderr_to_ctx() -> Result<()> {
    let (mut ctx, out, err) = capturing_ctx();
    let mut cmd = ctx.command("sh");
    process::run(&mut ctx, cmd.args(["-c", "echo to-out; echo to-err >&2"]))?;

    assert_eq!("to-out\n", out.to_string());
    assert_eq!("to-err\n", err.to_string());
    Ok(())
}

#[test]
fn run_fails_on_non_zero_exit() {
    let (mut ctx, _, err) = capturing_ctx();
    let mut cmd = ctx.command("sh");
    let res = process::run(&mut ctx, cmd.args(["-c", "echo boom >&2; exit 3"]));

    let msg = res.expect_err("exit 3 should be an error").to_string();
    assert!(msg.contains("`sh` failed"), "unexpected error: {msg}");
    assert!(msg.contains('3'), "exit code missing from error: {msg}");
    assert_eq!("boom\n", err.to_string());
}

#[test]
fn run_handles_output_larger_than_a_pipe_buffer() -> Result<()> {
    // 1 MB on stderr before anything on stdout. Reading the streams one
    // at a time would hang here once the stderr pipe filled up.
    let script = "head -c 1048576 /dev/zero | tr '\\0' e >&2; echo done";
    let (mut ctx, out, err) = capturing_ctx();
    let mut cmd = ctx.command("sh");
    process::run(&mut ctx, cmd.args(["-c", script]))?;

    assert_eq!("done\n", out.to_string());
    assert_eq!(1_048_576, err.to_string().len());
    Ok(())
}
