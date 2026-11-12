//! Runs the real `dev` binary.
//!
//! Only for cases that fail before anything is applied: these must never use
//! a `dev.yaml` with a supported dependency. Nearly any key under `deps` is
//! one (a mise tool), so invalid ones must use a reserved key like `custom`.
//!
//! [`dev`] also keeps `dev` away from the real home and the real mise and zb,
//! as a backstop for when that rule gets broken.

use std::process::{Command, Output};

use dev::Result;
use tempfile::TempDir;

/// Env vars that point dev, mise or zerobrew somewhere other than `$HOME`.
/// Removed so everything they'd write lands under the test's `$HOME`.
const HOME_OVERRIDES: &[&str] = &[
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "MISE_CONFIG_DIR",
    "MISE_DATA_DIR",
    "MISE_INSTALL_PATH",
    "ZEROBREW_BIN",
    "ZEROBREW_DIR",
];

/// Runs `dev <args>` in `dir`, with `$HOME` at `dir/home` and a `PATH` with
/// only system tools, so the machine's own mise and zb can't be found.
///
/// A plain `Command` on purpose: this starts `dev` itself, which must not
/// inherit any project env from a `Context`.
#[allow(clippy::disallowed_methods)]
fn dev(dir: &TempDir, args: &[&str]) -> Result<Output> {
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home)?;

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dev"));
    cmd.args(args)
        .current_dir(dir.path())
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin");
    for var in HOME_OVERRIDES {
        cmd.env_remove(var);
    }
    Ok(cmd.output()?)
}

#[test]
fn dev_runs_away_from_the_real_home_and_tools() -> Result<()> {
    let dir = project_dir("name: demo\ncommands:\n  where: 'echo \"$HOME|$PATH|${MISE_CONFIG_DIR:-unset}\"'\n")?;
    let output = dev(&dir, &["where"])?;

    assert_eq!(
        format!("{}|/usr/bin:/bin|unset\n", dir.path().join("home").display()),
        String::from_utf8_lossy(&output.stdout)
    );
    Ok(())
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
    std::fs::write(dir.path().join("dev.yaml"), "name: demo\ndeps:\n  custom: []\n")?;

    let output = dev(&dir, &["up"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(Some(1), output.status.code(), "stderr: {stderr}");
    assert!(stderr.contains("invalid project file dev.yaml"), "stderr: {stderr}");
    assert!(stderr.contains("unsupported dependency `custom`"), "stderr: {stderr}");
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

/// Runs `dev hook-env` in `$HOME/<rel>`, with `home` as `$HOME` so the real
/// user's global dev.yaml is never read.
#[allow(clippy::disallowed_methods)]
fn hook_env(home: &TempDir, rel: &str) -> Result<Output> {
    let pwd = home.path().join(rel);
    Ok(Command::new(env!("CARGO_BIN_EXE_dev"))
        .arg("hook-env")
        .current_dir(&pwd)
        .env("HOME", home.path())
        .env("PWD", &pwd)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("__DEV_ENV")
        .output()?)
}

#[test]
fn hook_env_prints_nothing_without_dev_yaml() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let output = hook_env(&dir, "")?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(Some(0), output.status.code(), "stderr: {stderr}");
    assert!(
        output.stdout.is_empty(),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    Ok(())
}

#[test]
fn hook_env_fails_on_invalid_dev_yaml_without_printing_a_script() -> Result<()> {
    let home = tempfile::tempdir()?;
    std::fs::create_dir(home.path().join("proj"))?;
    std::fs::write(home.path().join("proj/dev.yaml"), "name: Not Valid\n")?;

    let output = hook_env(&home, "proj")?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(Some(1), output.status.code(), "stderr: {stderr}");
    assert!(stderr.contains("must be lowercase words"), "stderr: {stderr}");
    assert!(output.stdout.is_empty(), "the shell must eval nothing");
    Ok(())
}
