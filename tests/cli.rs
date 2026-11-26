//! Runs the real `dev` binary.
//!
//! Only for cases that fail before anything is applied: these must never use
//! a `dev.yaml` with a supported dependency. Nearly any key under `deps` is
//! one (a mise tool), so invalid ones must use a built-in key with a value
//! of the wrong shape, like `custom: nope`.
//!
//! [`dev`] also keeps `dev` away from the real home and the real mise and zb,
//! as a backstop for when that rule gets broken.

use std::{
    path::Path,
    process::{Command, Output},
};

use dev::Result;
use tempfile::TempDir;

mod common;

/// Env vars that point dev, mise or zerobrew somewhere other than `$HOME`.
/// Removed so everything they'd write lands under the test's `$HOME`.
const HOME_OVERRIDES: &[&str] = &[
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "DEV_DOWNLOAD_URL",
    "MISE_CONFIG_DIR",
    "MISE_DATA_DIR",
    "MISE_INSTALL_PATH",
    "ZEROBREW_BIN",
    "ZEROBREW_DIR",
    // Not a location, but it changes what `dev up` prints.
    "DEV_UI",
];

/// Runs `dev <args>` in `dir`, with `$HOME` at `dir/home` and a `PATH` with
/// only system tools, so the machine's own mise and zb can't be found.
///
/// A plain `Command` on purpose: this starts `dev` itself, which must not
/// inherit any project env from a `Context`.
fn dev(dir: &TempDir, args: &[&str]) -> Result<Output> {
    dev_with(dir, args, &[])
}

/// Like [`dev`], with `vars` set on top.
#[allow(clippy::disallowed_methods)]
fn dev_with(dir: &TempDir, args: &[&str], vars: &[(&str, &str)]) -> Result<Output> {
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
    // Never a real update, even from a release build. Tests opt in.
    cmd.env("DEV_AUTO_UPDATE", "0");
    cmd.envs(vars.iter().copied());
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
    std::fs::write(dir.path().join("dev.yaml"), "name: demo\ndeps:\n  custom: nope\n")?;

    let output = dev(&dir, &["up"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(Some(1), output.status.code(), "stderr: {stderr}");
    assert!(stderr.contains("invalid project file dev.yaml"), "stderr: {stderr}");
    assert!(
        stderr.contains("line 3 column 11: expected sequence start"),
        "stderr: {stderr}"
    );
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
  deploy:
    desc: Deploy an app
    flags:
      env: { short: e, default: staging, desc: Target environment }
      dry-run: { type: bool }
      tag: {}
    args:
      - name: target
      - name: region
        required: false
    run: 'echo "$TARGET|${REGION-unset}|$ENV|$DRY_RUN|${TAG-unset}|$*"'
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
fn declared_flags_and_args_reach_the_command_as_env() -> Result<()> {
    let dir = project_dir(COMMANDS_YAML)?;
    let cases: [(&[&str], &str); 2] = [
        (
            &["deploy", "web", "-e", "prod", "--dry-run"],
            "web|unset|prod|true|unset|\n",
        ),
        (
            &["deploy", "web", "eu", "--tag", "v1", "--", "a", "b c"],
            "web|eu|staging|false|v1|a b c\n",
        ),
    ];

    for (args, want) in cases {
        let output = dev(&dir, args)?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(Some(0), output.status.code(), "args: {args:?}, stderr: {stderr}");
        assert_eq!(want, String::from_utf8_lossy(&output.stdout), "args: {args:?}");
    }
    Ok(())
}

#[test]
fn declared_commands_get_help_and_usage_errors() -> Result<()> {
    let dir = project_dir(COMMANDS_YAML)?;

    let help = dev(&dir, &["deploy", "--help"])?;
    let stdout = String::from_utf8_lossy(&help.stdout);
    assert_eq!(Some(0), help.status.code(), "stdout: {stdout}");
    for want in ["Deploy an app", "Usage: dev deploy", "Target environment", "--dry-run"] {
        assert!(stdout.contains(want), "want `{want}` in help: {stdout}");
    }

    let missing = dev(&dir, &["deploy"])?;
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert_eq!(Some(2), missing.status.code(), "stderr: {stderr}");
    assert!(stderr.contains("<TARGET>"), "stderr: {stderr}");
    assert!(
        !stderr.contains("Location:"),
        "clap's message, not an eyre report: {stderr}"
    );
    assert!(missing.stdout.is_empty(), "nothing should run");
    Ok(())
}

#[test]
fn file_commands_run_the_script_next_to_dev_yaml() -> Result<()> {
    let dir = project_dir(
        r#"name: demo
commands:
  release:
    file: scripts/release.sh
    flags:
      yes: { short: y, type: bool }
  bare:
    file: bare.sh
"#,
    )?;
    let scripts = [
        ("scripts/release.sh", "#!/bin/sh\necho \"file|$YES|$*\"\n"),
        // No slash, so it must not be looked up on PATH.
        ("bare.sh", "#!/bin/sh\necho \"bare|$*\"\n"),
    ];
    for (path, body) in scripts {
        common::write_executable(&dir.path().join(path), body.as_bytes())?;
    }

    let cases: [(&[&str], &str); 2] = [
        (&["release", "-y", "--", "x"], "file|true|x\n"),
        (&["bare", "a", "b"], "bare|a b\n"),
    ];
    for (args, want) in cases {
        let output = dev(&dir, args)?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(Some(0), output.status.code(), "args: {args:?}, stderr: {stderr}");
        assert_eq!(want, String::from_utf8_lossy(&output.stdout), "args: {args:?}");
    }
    Ok(())
}

#[test]
fn help_lists_project_commands() -> Result<()> {
    let dir = project_dir(COMMANDS_YAML)?;
    let output = dev(&dir, &["--help"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(Some(0), output.status.code(), "stdout: {stdout}");
    assert!(stdout.contains("Commands:\n  up "), "built-ins still listed: {stdout}");
    assert!(
        stdout.contains("Project commands:\n  deploy  Deploy an app\n  fail\n  hello\n"),
        "stdout: {stdout}"
    );
    Ok(())
}

#[test]
fn bare_dev_lists_project_commands_too() -> Result<()> {
    let dir = project_dir(COMMANDS_YAML)?;
    let output = dev(&dir, &[])?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    // clap's usual help-on-no-command: stderr and a usage error's status.
    assert_eq!(Some(2), output.status.code(), "stderr: {stderr}");
    assert!(
        stderr.contains("Project commands:\n  deploy  Deploy an app\n"),
        "stderr: {stderr}"
    );
    Ok(())
}

#[test]
fn help_works_without_a_valid_dev_yaml() -> Result<()> {
    let none = tempfile::tempdir()?;
    let output = dev(&none, &["--help"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(Some(0), output.status.code(), "stdout: {stdout}");
    assert!(
        !stdout.contains("Project commands"),
        "no dev.yaml, no section: {stdout}"
    );

    let invalid = project_dir("name: Not Valid\n")?;
    let output = dev(&invalid, &["--help"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(Some(0), output.status.code(), "stdout: {stdout}");
    assert!(
        stdout.contains("Project commands:\n  (dev.yaml is invalid: project name `Not Valid` must be lowercase"),
        "stdout: {stdout}"
    );
    Ok(())
}

#[test]
fn the_next_command_prints_an_update_notice_once() -> Result<()> {
    let dir = project_dir(COMMANDS_YAML)?;
    let state = dir.path().join("home/.local/state/dev");
    std::fs::create_dir_all(&state)?;
    std::fs::write(state.join("updated"), "dev updated 0.1.0 -> 0.2.0\n")?;

    let first = dev(&dir, &["hello"])?;
    assert_eq!(Some(0), first.status.code());
    assert_eq!("dev updated 0.1.0 -> 0.2.0\n", String::from_utf8_lossy(&first.stderr));
    assert_eq!("hi from hello, args: \n", String::from_utf8_lossy(&first.stdout));

    let second = dev(&dir, &["hello"])?;
    assert!(
        second.stderr.is_empty(),
        "shown once: {}",
        String::from_utf8_lossy(&second.stderr)
    );
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
    // One per line, which the report indents to line up with the first, and
    // colors, ending the last one with a reset.
    let lines: Vec<&str> = stderr.lines().map(|l| l.trim().trim_end_matches("\u{1b}[0m")).collect();
    assert!(
        lines
            .iter()
            .any(|l| l.ends_with("unknown command `nope`; dev.yaml has:")),
        "stderr: {stderr}"
    );
    for entry in ["deploy - Deploy an app", "fail", "hello"] {
        assert!(lines.contains(&entry), "want `{entry}` in stderr: {stderr}");
    }
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
    assert!(dir.path().join(".dev/log/latest").is_file(), "down still gets a log");
    assert_eq!("*\n", std::fs::read_to_string(dir.path().join(".dev/.gitignore"))?);
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

#[test]
fn shell_init_prints_the_shell_setup_anywhere() -> Result<()> {
    // No dev.yaml: it's run from rc files, wherever the shell starts.
    let dir = tempfile::tempdir()?;
    let output = dev(&dir, &["shell-init"])?;

    assert_eq!(
        Some(0),
        output.status.code(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let share = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/share/shell"))?;
    let shell = dir.path().join("home/.local/share/dev/shell");
    let want = format!(
        "__dev_shell_dir='{}'\n__dev_stamp_file='{}/.stamp'\n__dev_setups='{}'\n{share}",
        shell.display(),
        shell.display(),
        dev::shell::SETUP_FILES.join("\n"),
    );
    assert_eq!(want, String::from_utf8_lossy(&output.stdout));
    Ok(())
}

#[test]
fn action_paths_prints_the_layout_for_this_env() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let data = dir.path().join("data");
    let data_str = data.to_str().expect("temp paths are UTF-8");
    let output = dev_with(&dir, &["action-paths"], &[("XDG_DATA_HOME", data_str)])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(Some(0), output.status.code(), "stderr: {stderr}");

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let shims = data.join("mise/shims");
    assert_eq!(Some(shims.to_str().expect("UTF-8")), json["path"][0]["dir"].as_str());
    let exe_dir = Path::new(env!("CARGO_BIN_EXE_dev")).parent().expect("dev is in a dir");
    assert_eq!(
        Some(exe_dir.to_str().expect("UTF-8")),
        json["path"][3]["dir"].as_str(),
        "dev's own dir comes last"
    );
    assert!(json["cache"].is_array() && json["brew_cache"].is_array(), "{json}");
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
        .env_remove("XDG_STATE_HOME")
        .env_remove("__DEV_ENV")
        .env("DEV_AUTO_UPDATE", "0")
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

#[test]
fn up_rejects_an_unknown_dev_ui() -> Result<()> {
    let dir = project_dir("name: demo\n")?;
    let output = dev_with(&dir, &["up"], &[("DEV_UI", "fancy")])?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(Some(1), output.status.code(), "stderr: {stderr}");
    assert!(
        stderr.contains("unknown DEV_UI `fancy`; the only option is `plain`"),
        "stderr: {stderr}"
    );
    Ok(())
}

#[test]
fn new_creates_a_project_from_a_user_template() -> Result<()> {
    let dir = tempfile::tempdir()?;
    // No deps, so the `dev up` it runs applies nothing.
    let template = dir.path().join("home/.config/dev/templates/t");
    std::fs::create_dir_all(&template)?;
    std::fs::write(template.join("dev.yaml"), "name: {{name}}\n")?;
    std::fs::write(template.join(".dev-notes.txt"), "Rename {{name}} first.\n")?;

    let output = dev(&dir, &["new", "demo", "-t", "t"])?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(Some(0), output.status.code(), "stderr: {stderr}");
    let demo = dir.path().join("demo");
    assert_eq!("name: demo\n", std::fs::read_to_string(demo.join("dev.yaml"))?);
    assert!(demo.join(".git").is_dir(), "no git repo");
    assert!(demo.join(".dev/log/latest").exists(), "dev up didn't run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.ends_with("Next: cd demo\n\nRename demo first.\n"),
        "stdout: {stdout}"
    );
    Ok(())
}

#[test]
fn new_fails_when_the_dir_exists() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::create_dir(dir.path().join("demo"))?;

    let output = dev(&dir, &["new", "demo", "-t", "go"])?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(Some(1), output.status.code(), "stderr: {stderr}");
    assert!(stderr.contains("failed to create demo"), "stderr: {stderr}");
    assert!(!dir.path().join("demo/.git").exists(), "git init ran anyway");
    Ok(())
}

#[test]
fn up_global_fails_without_a_global_dev_yaml() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let output = dev(&dir, &["up", "--global"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(Some(1), output.status.code(), "stderr: {stderr}");
    let path = dir.path().join("home/.config/dev.yaml");
    assert!(
        stderr.contains(&format!("failed to read {}", path.display())),
        "stderr: {stderr}"
    );
    assert!(output.stdout.is_empty(), "nothing should be applied");
    Ok(())
}

#[test]
fn up_global_reads_only_the_global_dev_yaml_and_logs_to_the_state_dir() -> Result<()> {
    // Invalid, so reading it would fail.
    let dir = project_dir("name: demo\ndeps:\n  custom: nope\n")?;
    let config = dir.path().join("home/.config");
    std::fs::create_dir_all(&config)?;
    // No deps, so nothing gets installed.
    std::fs::write(config.join("dev.yaml"), "env:\n  A: a\n")?;

    let output = dev(&dir, &["up", "--global"])?;
    assert_eq!(
        Some(0),
        output.status.code(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(dir.path().join("home/.local/state/dev/log/latest").is_file());
    assert!(!dir.path().join(".dev").exists(), "the project dir is left alone");
    Ok(())
}

#[test]
fn project_commands_skip_the_env_the_shell_already_loaded() -> Result<()> {
    let dir = project_dir("name: demo\nenv:\n  X: from-yaml\ncommands:\n  show: echo \"$X\"\n")?;
    let state = common::shell_state(&dir.path().join("dev.yaml"), false)?;

    let output = dev_with(&dir, &["show"], &[("X", "from-shell"), ("__DEV_ENV", &state)])?;

    assert_eq!(
        Some(0),
        output.status.code(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!("from-shell\n", String::from_utf8_lossy(&output.stdout));
    Ok(())
}

#[test]
fn list_commands_run_in_order_and_exit_at_the_first_failure() -> Result<()> {
    let dir = project_dir(
        r#"name: demo
commands:
  steps:
    - X="$0"
    - |
      echo "$X: $*"
      (exit 4)
    - echo unreachable
"#,
    )?;
    let output = dev(&dir, &["steps", "a", "b"])?;

    assert_eq!(
        Some(4),
        output.status.code(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!("steps: a b\n", String::from_utf8_lossy(&output.stdout));
    Ok(())
}

#[test]
fn cmd_items_run_other_commands_with_layered_env() -> Result<()> {
    let dir = project_dir(
        r#"name: demo
env:
  BASE: project
commands:
  child:
    flags:
      mode: { default: child-default }
      dry-run: { type: bool }
    run: echo "$0 [$*] mode=$MODE dry=$DRY_RUN parent=$PARENT_FLAG base=$BASE extra=$EXTRA"
  parent:
    flags:
      parent-flag: { default: from-parent }
      dry-run: { type: bool }
    run:
      - echo "parent [$*]"
      - { cmd: child, args: [--, x], env: { EXTRA: item, MODE: item-mode } }
      - { cmd: child, args: [--dry-run] }
      - { cmd: fail }
      - echo unreachable
  fail: exit 5
"#,
    )?;
    let output = dev(&dir, &["parent", "--dry-run", "--", "p1"])?;

    assert_eq!(
        Some(5),
        output.status.code(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The caller's vars are inherited, then the target's own flags (so
    // dry-run resets), then the item's env. The caller's `$@` isn't.
    let want = "parent [p1]\n\
        child [x] mode=item-mode dry=false parent=from-parent base=project extra=item\n\
        child [] mode=child-default dry=true parent=from-parent base=project extra=\n";
    assert_eq!(want, String::from_utf8_lossy(&output.stdout));
    Ok(())
}

#[test]
fn depends_run_first_in_order_once_with_headers() -> Result<()> {
    let dir = project_dir(
        r#"name: demo
commands:
  a: { depends: [b, c], run: 'echo a "$@"' }
  b: { depends: [d], run: echo b }
  c: { depends: [d], run: echo c }
  d: echo d
"#,
    )?;
    let output = dev(&dir, &["a", "x"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(Some(0), output.status.code(), "stderr: {stderr}");
    assert_eq!("d\nb\nc\na x\n", String::from_utf8_lossy(&output.stdout));
    assert!(stderr.contains("==> d\n==> b\n==> c\n"), "stderr: {stderr}");
    assert!(!stderr.contains("==> a"), "stderr: {stderr}");
    Ok(())
}

#[test]
fn a_failing_dep_stops_the_run_with_its_code() -> Result<()> {
    let dir = project_dir(
        r#"name: demo
commands:
  ci: { depends: [ok, bad, never], run: echo main }
  ok: echo ok
  bad: exit 5
  never: echo never
"#,
    )?;
    let output = dev(&dir, &["ci"])?;

    assert_eq!(
        Some(5),
        output.status.code(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!("ok\n", String::from_utf8_lossy(&output.stdout));
    Ok(())
}

#[test]
fn deps_get_their_flag_defaults() -> Result<()> {
    let dir = project_dir(
        r#"name: demo
commands:
  ci: { depends: [show], run: "true" }
  show:
    flags:
      env: { default: staging }
    run: echo "$ENV"
"#,
    )?;
    let output = dev(&dir, &["ci"])?;

    assert_eq!(
        Some(0),
        output.status.code(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!("staging\n", String::from_utf8_lossy(&output.stdout));
    Ok(())
}

#[test]
fn help_and_usage_errors_run_no_deps() -> Result<()> {
    let dir = project_dir(
        r#"name: demo
commands:
  ci:
    depends: [loud]
    flags:
      yes: { type: bool }
    run: "true"
  loud: echo LOUD
"#,
    )?;

    for (args, code) in [(&["ci", "--help"][..], 0), (&["ci", "--nope"][..], 2)] {
        let output = dev(&dir, args)?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(Some(code), output.status.code(), "dev {args:?}");
        assert!(!stdout.contains("LOUD"), "dev {args:?} ran a dep: {stdout}");
    }
    Ok(())
}

#[test]
fn depends_only_commands_reject_args_with_usage() -> Result<()> {
    let dir = project_dir("name: demo\ncommands:\n  ci: { depends: [ok] }\n  ok: echo ok\n")?;
    let output = dev(&dir, &["ci", "extra"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(Some(2), output.status.code(), "stderr: {stderr}");
    assert!(stderr.contains("Usage: dev ci"), "stderr: {stderr}");
    assert!(
        output.stdout.is_empty(),
        "ran a dep: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    Ok(())
}

#[test]
fn depends_only_commands_get_help() -> Result<()> {
    let dir = project_dir("name: demo\ncommands:\n  ci: { desc: Run CI, depends: [ok] }\n  ok: echo ok\n")?;
    let output = dev(&dir, &["ci", "--help"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(
        Some(0),
        output.status.code(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("Run CI"), "stdout: {stdout}");
    assert!(!stdout.contains("ARGS"), "no script, so no `-- ARGS`: {stdout}");
    assert!(!stdout.contains("ok\n"), "ran a dep: {stdout}");
    Ok(())
}

#[test]
fn cmd_items_run_their_targets_depends() -> Result<()> {
    let dir = project_dir(
        r#"name: demo
commands:
  check:
    - echo check
    - { cmd: test }
  test: { depends: [build], run: echo test }
  build: echo build
"#,
    )?;
    let output = dev(&dir, &["check"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(Some(0), output.status.code(), "stderr: {stderr}");
    assert_eq!("check\nbuild\ntest\n", String::from_utf8_lossy(&output.stdout));
    assert!(stderr.contains("==> build\n"), "stderr: {stderr}");
    Ok(())
}
