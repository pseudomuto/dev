mod common;

use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

use common::capturing_ctx;
use dev::{
    Context,
    Result,
    deps::{
        Task,
        brew::{Package, ZeroBrew},
    },
    shell::ShellDir,
};
use tempfile::{TempDir, tempdir};

/// Writes an executable shell script to `dir/name` and returns its path.
fn script(dir: &TempDir, name: &str, body: &str) -> Result<PathBuf> {
    let path = dir.path().join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n"))?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
    Ok(path)
}

#[test]
fn zerobrew_is_not_applied_when_zb_is_missing() -> Result<()> {
    let dir = tempdir()?;
    let task = ZeroBrew::default()
        .with_zb(dir.path().join("zb"))
        .with_shell_dir(shell_dir(&dir)?)
        .with_latest_version(|_| panic!("latest version shouldn't be checked when zb is missing"));

    assert!(!task.is_applied(&mut Context::default())?);
    Ok(())
}

#[test]
fn zerobrew_is_not_applied_when_shell_file_is_missing() -> Result<()> {
    let dir = tempdir()?;
    let zb = script(&dir, "zb", r#"echo "zb 0.4.2""#)?;
    let task = ZeroBrew::default()
        .with_zb(zb)
        .with_shell_dir(dir.path().join("missing"))
        .with_latest_version(|_| panic!("latest version shouldn't be checked without a shell file"));

    assert!(!task.is_applied(&mut Context::default())?);
    Ok(())
}

/// Makes a shell dir in `dir` with an empty stand-in for zerobrew's
/// generated file, and returns the shell dir.
fn shell_dir(dir: &TempDir) -> Result<PathBuf> {
    let shell = dir.path().join("shell");
    fs::create_dir_all(&shell)?;
    fs::write(shell.join("zerobrew"), "")?;
    Ok(shell)
}

#[test]
fn zerobrew_is_applied_when_installed_version_is_latest() -> Result<()> {
    let dir = tempdir()?;
    let zb = script(&dir, "zb", r#"echo "zb 0.4.2""#)?;
    let task = ZeroBrew::default()
        .with_zb(zb)
        .with_shell_dir(shell_dir(&dir)?)
        .with_latest_version(|_| Ok("0.4.2".into()));

    assert!(task.is_applied(&mut Context::default())?);
    Ok(())
}

#[test]
fn zerobrew_is_not_applied_when_installed_version_is_old() -> Result<()> {
    let dir = tempdir()?;
    let zb = script(&dir, "zb", r#"echo "zb 0.4.1""#)?;
    let task = ZeroBrew::default()
        .with_zb(zb)
        .with_shell_dir(shell_dir(&dir)?)
        .with_latest_version(|_| Ok("0.4.2".into()));

    assert!(!task.is_applied(&mut Context::default())?);
    Ok(())
}

/// A fake `zb` at `version` whose `init` writes a zerobrew block to
/// `$HOME/.profile`, the way the real one does for an unknown `$SHELL`. The
/// block records the env `init` saw, and has junk around it that must not be
/// copied.
fn zb_with_init(dir: &TempDir, version: &str) -> Result<PathBuf> {
    let body = format!(
        r#"[ "$1" = --version ] && {{ echo "zb {version}"; exit; }}
[ "$1" = init ] || exit 1
cat > "$HOME/.profile" <<EOT
# junk before
# >>> zerobrew >>>
shell=$SHELL
zerobrew_bin=$ZEROBREW_BIN
# <<< zerobrew <<<
# junk after
EOT
echo "Restart your terminal or run: . '$HOME/.profile'""#
    );
    script(dir, "zb", &body)
}

/// An installer that just prints its arguments, and some stderr.
fn installer(dir: &TempDir) -> Result<String> {
    let path = script(dir, "install", r#"echo "args: $*"; echo "progress" >&2"#)?;
    Ok(format!("file://{}", path.display()))
}

#[test]
fn zerobrew_apply_runs_installer_with_no_modify_path() -> Result<()> {
    let dir = tempdir()?;
    let task = ZeroBrew::default()
        .with_install_url(installer(&dir)?)
        .with_zb(zb_with_init(&dir, "0.4.1")?)
        .with_latest_version(|_| Ok("0.4.2".into()))
        .with_shell_dir(dir.path().join("shell"));
    let (mut ctx, out, err) = capturing_ctx();

    task.apply(&mut ctx)?;
    assert_eq!("args: --no-modify-path\n", out.to_string());
    assert_eq!("progress\n", err.to_string());
    Ok(())
}

#[test]
fn zerobrew_apply_skips_installer_when_zb_is_current() -> Result<()> {
    let dir = tempdir()?;
    let shell_file = dir.path().join("shell/zerobrew");
    let task = ZeroBrew::default()
        .with_install_url(format!("file://{}", dir.path().join("missing").display()))
        .with_zb(zb_with_init(&dir, "0.4.2")?)
        .with_latest_version(|_| Ok("0.4.2".into()))
        .with_shell_dir(dir.path().join("shell"));
    let (mut ctx, out, err) = capturing_ctx();

    task.apply(&mut ctx)?;
    assert!(shell_file.exists(), "the shell file should still be written");
    assert_eq!("", out.to_string());
    assert_eq!("", err.to_string());
    Ok(())
}

#[test]
fn zerobrew_apply_writes_only_the_zerobrew_block_from_zb_init() -> Result<()> {
    let dir = tempdir()?;
    let shell_file = dir.path().join("shell/zerobrew");
    let task = ZeroBrew::default()
        .with_install_url(installer(&dir)?)
        .with_zb(zb_with_init(&dir, "0.4.1")?)
        .with_latest_version(|_| Ok("0.4.2".into()))
        .with_shell_dir(dir.path().join("shell"));

    task.apply(&mut capturing_ctx().0)?;
    assert_eq!(
        format!(
            "# >>> zerobrew >>>\nshell=/bin/sh\nzerobrew_bin={}\n# <<< zerobrew <<<\n",
            dir.path().display()
        ),
        fs::read_to_string(&shell_file)?
    );
    assert!(
        !ShellDir::new(dir.path().join("shell")).stamp()?.is_empty(),
        "open shells find out about the new setup from the stamp"
    );
    Ok(())
}

#[test]
fn zerobrew_apply_fails_when_zb_init_writes_no_block() -> Result<()> {
    let dir = tempdir()?;
    let zb = script(
        &dir,
        "zb",
        r##"[ "$1" = --version ] && { echo "zb 0.4.1"; exit; }
echo "# nothing to see" > "$HOME/.profile""##,
    )?;
    let shell_file = dir.path().join("shell/zerobrew");
    let task = ZeroBrew::default()
        .with_install_url(installer(&dir)?)
        .with_zb(zb)
        .with_latest_version(|_| Ok("0.4.2".into()))
        .with_shell_dir(dir.path().join("shell"));

    let msg = task
        .apply(&mut capturing_ctx().0)
        .expect_err("a missing block should fail")
        .to_string();
    assert_eq!(
        "`zb init` wrote no `# >>> zerobrew >>>` block; zerobrew may have changed how it sets up shells",
        msg
    );
    assert!(!shell_file.exists(), "nothing should be written");
    Ok(())
}

#[test]
fn zerobrew_apply_fails_when_installer_cannot_be_fetched() -> Result<()> {
    // Without `set -o pipefail`, curl's failure would be swallowed and bash
    // would exit 0 after reading an empty script.
    let dir = tempdir()?;
    let url = format!("file://{}", dir.path().join("missing").display());
    let task = ZeroBrew::default()
        .with_install_url(&url)
        .with_zb(dir.path().join("zb"))
        .with_latest_version(|_| panic!("latest version shouldn't be checked when zb is missing"));
    let (mut ctx, _, err) = capturing_ctx();

    let msg = task
        .apply(&mut ctx)
        .expect_err("a missing installer should fail")
        .to_string();
    assert_eq!(format!("zerobrew installer ({url}) failed"), msg);
    assert!(
        err.to_string().contains("curl: (37)"),
        "expected curl's error on stderr: {err}"
    );
    Ok(())
}

/// A fake `zb` that prints `list` and `outdated` lines and appends each call's
/// arguments to `<dir>/calls`.
struct FakeZb {
    path: PathBuf,
    calls: PathBuf,
}

impl FakeZb {
    fn new(dir: &TempDir, list: &str, outdated: &str) -> Result<Self> {
        let calls = dir.path().join("calls");
        let body = format!(
            r#"echo "$*" >> '{calls}'
case "$1" in
  list) printf '{list}' ;;
  outdated) printf '{outdated}' ;;
  install|upgrade) echo "$1 $2 done" ;;
esac"#,
            calls = calls.display(),
        );
        Ok(Self {
            path: script(dir, "zb", &body)?,
            calls,
        })
    }

    fn calls(&self) -> Result<Vec<String>> {
        // No file just means zb was never called.
        let calls = match fs::read_to_string(&self.calls) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            res => res?,
        };
        Ok(calls.lines().map(str::to_owned).collect())
    }
}

#[test]
fn package_is_not_applied_when_not_installed() -> Result<()> {
    let dir = tempdir()?;
    let zb = FakeZb::new(&dir, "yq 4.44.1\n", "")?;
    let pkg = Package::new("jq").with_zb(&zb.path);

    assert_eq!("Installing jq", pkg.name());
    assert!(!pkg.is_applied(&mut Context::default())?);
    assert_eq!(vec!["list"], zb.calls()?);
    Ok(())
}

#[test]
fn package_is_applied_when_installed_and_current() -> Result<()> {
    let dir = tempdir()?;
    let zb = FakeZb::new(&dir, "jq 1.7.1\nyq 4.44.1\n", "yq\n")?;
    let pkg = Package::new("jq").with_zb(&zb.path);

    assert!(pkg.is_applied(&mut Context::default())?);
    assert_eq!(vec!["list", "outdated --quiet"], zb.calls()?);
    Ok(())
}

#[test]
fn package_is_not_applied_when_outdated() -> Result<()> {
    let dir = tempdir()?;
    let zb = FakeZb::new(&dir, "jq 1.7.0\n", "jq\n")?;
    let pkg = Package::new("jq").with_zb(&zb.path);

    assert!(!pkg.is_applied(&mut Context::default())?);
    Ok(())
}

#[test]
fn package_apply_installs_when_missing() -> Result<()> {
    let dir = tempdir()?;
    let zb = FakeZb::new(&dir, "No formulas installed.\n", "")?;
    let pkg = Package::new("jq").with_zb(&zb.path);
    let (mut ctx, out, _) = capturing_ctx();

    pkg.apply(&mut ctx)?;
    assert_eq!(vec!["list", "install jq"], zb.calls()?);
    assert_eq!("install jq done\n", out.to_string());
    Ok(())
}

#[test]
fn package_apply_upgrades_when_installed() -> Result<()> {
    let dir = tempdir()?;
    let zb = FakeZb::new(&dir, "jq 1.7.0\n", "jq\n")?;
    let pkg = Package::new("jq").with_zb(&zb.path);
    let (mut ctx, out, _) = capturing_ctx();

    pkg.apply(&mut ctx)?;
    assert_eq!(vec!["list", "upgrade jq"], zb.calls()?);
    assert_eq!("upgrade jq done\n", out.to_string());
    Ok(())
}

#[test]
fn package_reports_zb_failures() -> Result<()> {
    let dir = tempdir()?;
    let zb = script(&dir, "zb", r#"echo "database is locked" >&2; exit 2"#)?;
    let pkg = Package::new("jq").with_zb(&zb);

    let msg = pkg
        .is_applied(&mut Context::default())
        .expect_err("a failing zb should be an error")
        .to_string();
    assert_eq!(
        format!("`{} list` failed with exit status: 2: database is locked", zb.display()),
        msg
    );
    Ok(())
}

#[test]
fn package_runs_zb_with_ctx_env() -> Result<()> {
    let dir = tempdir()?;
    let seen = dir.path().join("seen");
    let zb = script(&dir, "zb", &format!(r#"echo "$DEV_MARK" > '{}'"#, seen.display()))?;
    let mut ctx = Context {
        env: vec![("DEV_MARK".into(), "from dev.yaml".into())],
        ..Context::default()
    };

    Package::new("jq").with_zb(&zb).is_applied(&mut ctx)?;
    assert_eq!("from dev.yaml\n", fs::read_to_string(&seen)?);
    Ok(())
}
