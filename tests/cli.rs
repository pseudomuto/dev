//! Runs the real `dev` binary.
//!
//! Only for cases that fail before anything is applied: these must never use
//! a `dev.yaml` with a supported dependency, or the test would change the
//! machine running it.

use std::process::{Command, Output};

use dev::Result;
use tempfile::TempDir;

/// Runs `dev <args>` in `dir`.
///
/// A plain `Command` on purpose: this starts `dev` itself, which must not
/// inherit any project env from a `Context`.
#[allow(clippy::disallowed_methods)]
fn dev(dir: &TempDir, args: &[&str]) -> Result<Output> {
    Ok(Command::new(env!("CARGO_BIN_EXE_dev"))
        .args(args)
        .current_dir(dir.path())
        .output()?)
}

#[test]
fn up_fails_without_dev_yaml() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let output = dev(&dir, &["up"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(Some(1), output.status.code(), "stderr: {stderr}");
    assert!(stderr.contains("failed to read dev.yaml"), "stderr: {stderr}");
    assert!(output.stdout.is_empty(), "nothing should be applied");
    Ok(())
}

#[test]
fn up_fails_on_invalid_dev_yaml() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("dev.yaml"), "name: demo\ndeps:\n  go: 1.27\n")?;

    let output = dev(&dir, &["up"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(Some(1), output.status.code(), "stderr: {stderr}");
    assert!(stderr.contains("invalid project file dev.yaml"), "stderr: {stderr}");
    assert!(stderr.contains("unsupported dependency `go`"), "stderr: {stderr}");
    assert!(output.stdout.is_empty(), "nothing should be applied");
    Ok(())
}

/// A project with commands and env, but no deps, so nothing gets installed.
const COMMANDS_YAML: &str = r#"name: demo
env:
  GREETING: hi
commands:
  hello: 'echo "$GREETING from $0, args: $*"'
  fail: exit 3
"#;

fn project_dir(yaml: &str) -> Result<TempDir> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("dev.yaml"), yaml)?;
    Ok(dir)
}

#[test]
fn runs_project_commands_with_env_and_args() -> Result<()> {
    let dir = project_dir(COMMANDS_YAML)?;
    let output = dev(&dir, &["hello", "a", "b c"])?;

    assert_eq!(
        Some(0),
        output.status.code(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!("hi from hello, args: a b c\n", String::from_utf8_lossy(&output.stdout));
    Ok(())
}

#[test]
fn project_commands_exit_with_their_own_code() -> Result<()> {
    let dir = project_dir(COMMANDS_YAML)?;
    let output = dev(&dir, &["fail"])?;

    assert_eq!(Some(3), output.status.code());
    assert!(
        output.stderr.is_empty(),
        "no error report: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[test]
fn unknown_commands_list_the_available_ones() -> Result<()> {
    let dir = project_dir(COMMANDS_YAML)?;
    let output = dev(&dir, &["nope"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(Some(1), output.status.code(), "stderr: {stderr}");
    assert!(
        stderr.contains("unknown command `nope`; dev.yaml has: fail, hello"),
        "stderr: {stderr}"
    );
    Ok(())
}

#[test]
fn commands_named_after_built_ins_are_rejected() -> Result<()> {
    let dir = project_dir("name: demo\ncommands:\n  up: echo hijacked\n")?;
    let output = dev(&dir, &["up"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(Some(1), output.status.code(), "stderr: {stderr}");
    assert!(
        stderr.contains("command `up` in dev.yaml conflicts with a built-in command"),
        "stderr: {stderr}"
    );
    assert!(output.stdout.is_empty(), "nothing should run");
    Ok(())
}

#[test]
fn down_succeeds_with_no_deps() -> Result<()> {
    let dir = project_dir("name: demo\n")?;
    let output = dev(&dir, &["down"])?;

    assert_eq!(
        Some(0),
        output.status.code(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty(), "no deps, nothing to report");
    Ok(())
}

#[test]
fn commands_named_down_are_rejected() -> Result<()> {
    let dir = project_dir("name: demo\ncommands:\n  down: echo hijacked\n")?;
    let output = dev(&dir, &["down"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(Some(1), output.status.code(), "stderr: {stderr}");
    assert!(
        stderr.contains("command `down` in dev.yaml conflicts with a built-in command"),
        "stderr: {stderr}"
    );
    Ok(())
}
