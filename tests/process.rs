mod common;

use common::capturing_ctx;
use dev::{Result, process};

#[test]
fn run_writes_the_command_then_its_output_in_order_to_the_log() -> Result<()> {
    let (mut ctx, out, log) = capturing_ctx();
    let mut cmd = ctx.command("sh");
    process::run(&mut ctx, cmd.args(["-c", "echo one; echo two >&2; echo three"]))?;

    assert_eq!(
        "$ sh -c 'echo one; echo two >&2; echo three'\none\ntwo\nthree\n",
        log.to_string()
    );
    assert_eq!("", out.to_string(), "out is only for dev's own status lines");
    Ok(())
}

#[test]
fn run_failures_include_the_last_lines_of_output() {
    let (mut ctx, _, log) = capturing_ctx();
    let mut cmd = ctx.command("sh");
    let res = process::run(&mut ctx, cmd.args(["-c", "seq 1 25; echo boom >&2; exit 3"]));

    let msg = res.expect_err("exit 3 should be an error").to_string();
    let tail: Vec<String> = (7..=25)
        .map(|n| format!("    {n}"))
        .chain(["    boom".into()])
        .collect();
    assert_eq!(
        format!("`sh` failed with exit status: 3. Last output:\n{}", tail.join("\n")),
        msg
    );
    assert!(log.to_string().ends_with("24\n25\nboom\n"), "the log has it all");
}

#[test]
fn run_failures_show_only_the_last_redraw_of_a_progress_line() {
    let (mut ctx, ..) = capturing_ctx();
    let mut cmd = ctx.command("sh");
    let res = process::run(&mut ctx, cmd.args(["-c", r"printf '10%%\r50%%\r100%%\n'; exit 1"]));

    let msg = res.expect_err("exit 1 should be an error").to_string();
    assert_eq!("`sh` failed with exit status: 1. Last output:\n    100%", msg);
}

#[test]
fn run_handles_output_larger_than_a_pipe_buffer() -> Result<()> {
    // 1 MB in one line, then more. A reader that stopped before the end
    // would leave the command blocked on a full pipe.
    let script = "head -c 1048576 /dev/zero | tr '\\0' e >&2; echo; echo done";
    let (mut ctx, _, log) = capturing_ctx();
    let mut cmd = ctx.command("sh");
    process::run(&mut ctx, cmd.args(["-c", script]))?;

    let log = log.to_string();
    let (header, output) = log.split_once('\n').expect("a header line");
    assert!(header.starts_with("$ sh -c "), "header: {header}");
    assert_eq!(format!("{}\ndone\n", "e".repeat(1_048_576)), output);
    Ok(())
}

#[test]
fn run_gives_commands_no_stdin() -> Result<()> {
    let (mut ctx, _, log) = capturing_ctx();
    let mut cmd = ctx.command("sh");
    process::run(&mut ctx, cmd.args(["-c", "read -r line; echo \"read exited $?\""]))?;

    assert!(log.to_string().ends_with("read exited 1\n"), "log: {log}");
    Ok(())
}

#[test]
fn run_ends_output_with_a_newline_so_the_next_log_line_starts_fresh() -> Result<()> {
    let (mut ctx, _, log) = capturing_ctx();
    let mut cmd = ctx.command("sh");
    process::run(&mut ctx, cmd.args(["-c", "printf 'no newline'"]))?;

    assert_eq!("$ sh -c 'printf '\\''no newline'\\'''\nno newline\n", log.to_string());
    Ok(())
}
