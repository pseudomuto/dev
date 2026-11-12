mod common;

use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

use common::capturing_ctx;
use dev::{
    Context,
    Result,
    deps::{
        Task,
        mise::{Binary, Config, Tool},
    },
};
use tempfile::{TempDir, tempdir};

/// Writes an executable shell script to `dir/name` and returns its path.
fn script(dir: &TempDir, name: &str, body: &str) -> Result<PathBuf> {
    let path = dir.path().join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, format!("#!/bin/sh\n{body}\n"))?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
    Ok(path)
}

/// A fake `mise` at `dir/<name>` that appends each call's arguments to
/// `dir/calls`, prints `version_json` for `version --json`, and succeeds at
/// everything else.
struct FakeMise {
    path: PathBuf,
    calls: PathBuf,
}

impl FakeMise {
    fn new(dir: &TempDir, name: &str, version_json: &str) -> Result<Self> {
        let calls = dir.path().join("calls");
        let body = format!(
            r#"echo "$*" >> '{calls}'
[ "$1" = version ] && printf '%s' '{version_json}'
exit 0"#,
            calls = calls.display(),
        );
        Ok(Self {
            path: script(dir, name, &body)?,
            calls,
        })
    }

    fn calls(&self) -> Result<Vec<String>> {
        // No file just means mise was never called.
        let calls = match fs::read_to_string(&self.calls) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            res => res?,
        };
        Ok(calls.lines().map(str::to_owned).collect())
    }
}

/// The shell setup expected for the mise at `path`.
fn setup_for(path: &std::path::Path) -> String {
    format!(
        r#"if [ -n "${{ZSH_VERSION:-}}" ]; then
  eval "$('{p}' activate zsh)"
elif [ -n "${{BASH_VERSION:-}}" ]; then
  eval "$('{p}' activate bash)"
fi
"#,
        p = path.display()
    )
}

#[test]
fn binary_uses_an_external_mise_as_is() -> Result<()> {
    let dir = tempdir()?;
    let mise = FakeMise::new(&dir, "external/mise", "not json")?;
    let task = Binary::default()
        .with_search_path(dir.path().join("external"))
        .with_install_path(dir.path().join("managed/mise"))
        .with_install_url(format!("file://{}", dir.path().join("missing").display()))
        .with_shell_dir(dir.path().join("shell"));
    let mut ctx = capturing_ctx().0;

    assert!(!task.is_applied(&mut ctx)?, "no shell setup yet");
    task.apply(&mut ctx)?;
    assert_eq!(
        setup_for(&mise.path),
        fs::read_to_string(dir.path().join("shell/mise"))?
    );
    assert!(task.is_applied(&mut ctx)?);
    assert_eq!(
        Vec::<String>::new(),
        mise.calls()?,
        "an external mise is never checked or updated"
    );
    assert!(!dir.path().join("managed/mise").exists());
    Ok(())
}

/// An installer that writes a mise to `$MISE_INSTALL_PATH`, printing to both
/// streams.
fn installer(dir: &TempDir) -> Result<String> {
    let body = r#"mkdir -p "$(dirname "$MISE_INSTALL_PATH")"
printf '#!/bin/sh\n' > "$MISE_INSTALL_PATH"
chmod +x "$MISE_INSTALL_PATH"
echo "installed to $MISE_INSTALL_PATH"
echo "progress" >&2"#;
    let path = script(dir, "install", body)?;
    Ok(format!("file://{}", path.display()))
}

#[test]
fn binary_installs_mise_when_missing() -> Result<()> {
    let dir = tempdir()?;
    let install_path = dir.path().join("managed/mise");
    let task = Binary::default()
        .with_search_path(dir.path().join("empty"))
        .with_install_path(&install_path)
        .with_install_url(installer(&dir)?)
        .with_shell_dir(dir.path().join("shell"));
    let (mut ctx, out, err) = capturing_ctx();

    assert!(!task.is_applied(&mut ctx)?);
    task.apply(&mut ctx)?;
    assert_eq!(format!("installed to {}\n", install_path.display()), out.to_string());
    assert_eq!("progress\n", err.to_string());
    assert_eq!(
        setup_for(&install_path),
        fs::read_to_string(dir.path().join("shell/mise"))?
    );
    Ok(())
}

/// `mise version --json` output for `version`, with `latest` as given.
fn version_json(version: &str, latest: &str) -> String {
    format!(r#"{{"version": "{version} macos-arm64 (2026-11-10)", "latest": {latest}}}"#)
}

/// A [`Binary`] whose mise is a [`FakeMise`] at the install path, found on
/// the search path, with its shell setup already written.
fn managed(dir: &TempDir, version_json: &str) -> Result<(Binary, FakeMise)> {
    let mise = FakeMise::new(dir, "managed/mise", version_json)?;
    fs::create_dir_all(dir.path().join("shell"))?;
    fs::write(dir.path().join("shell/mise"), setup_for(&mise.path))?;
    let task = Binary::default()
        .with_search_path(dir.path().join("managed"))
        .with_install_path(&mise.path)
        .with_install_url(format!("file://{}", dir.path().join("missing").display()))
        .with_shell_dir(dir.path().join("shell"));
    Ok((task, mise))
}

#[test]
fn binary_is_applied_when_managed_mise_is_latest() -> Result<()> {
    let dir = tempdir()?;
    let (task, mise) = managed(&dir, &version_json("2026.11.6", r#""2026.11.6""#))?;

    assert!(task.is_applied(&mut capturing_ctx().0)?);
    assert_eq!(vec!["version --json"], mise.calls()?);
    Ok(())
}

#[test]
fn binary_self_updates_managed_mise_when_outdated() -> Result<()> {
    let dir = tempdir()?;
    let (task, mise) = managed(&dir, &version_json("2026.11.5", r#""2026.11.6""#))?;
    let mut ctx = capturing_ctx().0;

    assert!(!task.is_applied(&mut ctx)?);
    task.apply(&mut ctx)?;
    assert_eq!(
        vec!["version --json", "version --json", "self-update --yes"],
        mise.calls()?
    );
    Ok(())
}

#[test]
fn binary_fails_when_mise_cannot_report_the_latest_release() -> Result<()> {
    let dir = tempdir()?;
    let (task, mise) = managed(&dir, &version_json("2026.11.6", "null"))?;

    let msg = task
        .is_applied(&mut capturing_ctx().0)
        .expect_err("an unknown latest release should fail")
        .to_string();
    assert_eq!(
        format!(
            "`{} version --json` didn't report the latest release, so mise can't be checked for updates; is the network up?",
            mise.path.display()
        ),
        msg
    );
    Ok(())
}

#[test]
fn binary_fails_when_installer_cannot_be_fetched() -> Result<()> {
    // Without `set -o pipefail`, curl's failure would be swallowed and sh
    // would exit 0 after reading an empty script.
    let dir = tempdir()?;
    let url = format!("file://{}", dir.path().join("missing").display());
    let task = Binary::default()
        .with_search_path(dir.path().join("empty"))
        .with_install_path(dir.path().join("managed/mise"))
        .with_install_url(&url)
        .with_shell_dir(dir.path().join("shell"));
    let (mut ctx, _, err) = capturing_ctx();

    let msg = task
        .apply(&mut ctx)
        .expect_err("a missing installer should fail")
        .to_string();
    assert_eq!(format!("mise installer ({url}) failed"), msg);
    assert!(
        err.to_string().contains("curl: (37)"),
        "expected curl's error on stderr: {err}"
    );
    assert!(!dir.path().join("shell/mise").exists(), "nothing should be written");
    Ok(())
}

#[test]
fn binary_is_not_applied_when_setup_points_at_another_mise() -> Result<()> {
    let dir = tempdir()?;
    let (task, _) = managed(&dir, &version_json("2026.11.6", r#""2026.11.6""#))?;
    fs::write(dir.path().join("shell/mise"), setup_for(&dir.path().join("old/mise")))?;

    assert!(!task.is_applied(&mut capturing_ctx().0)?);
    Ok(())
}

/// A fake `mise` whose `config set --file F K V` stores `K=V` in `F` and
/// whose `config get --file F K` prints the last value stored for `K`, or
/// exits 1. Each call's working dir and arguments go to `dir/calls`.
fn config_mise(dir: &TempDir) -> Result<FakeMise> {
    let calls = dir.path().join("calls");
    let body = format!(
        r#"echo "$(pwd): $*" >> '{calls}'
[ "$1" = config ] || exit 2
case "$2" in
  set) echo "$5=$6" >> "$4" ;;
  get) line=$(grep -F "$5=" "$4" | tail -n 1); [ -n "$line" ] || exit 1; echo "${{line#*=}}" ;;
esac"#,
        calls = calls.display(),
    );
    Ok(FakeMise {
        path: script(dir, "mise", &body)?,
        calls,
    })
}

fn tools(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(n, v)| ((*n).into(), (*v).into())).collect()
}

#[test]
fn config_sets_tools_and_links_the_file_into_conf_d() -> Result<()> {
    let dir = tempdir()?;
    let mise = config_mise(&dir)?;
    let (file, link) = (
        dir.path().join("dev/mise.toml"),
        dir.path().join("config/mise/conf.d/dev.toml"),
    );
    let task = Config::new(tools(&[("node", "lts"), ("go:golang.org/x/tools/gopls", "latest")]))
        .with_mise(&mise.path)
        .with_file(&file)
        .with_link(&link);
    let mut ctx = capturing_ctx().0;

    assert!(!task.is_applied(&mut ctx)?);
    task.apply(&mut ctx)?;
    assert_eq!(file, fs::read_link(&link)?);
    assert_eq!(
        vec![
            format!(r#"/: config set --file {} tools."node" lts"#, file.display()),
            format!(
                r#"/: config set --file {} tools."go:golang.org/x/tools/gopls" latest"#,
                file.display()
            ),
        ],
        mise.calls()?
    );
    assert!(task.is_applied(&mut ctx)?);
    Ok(())
}

#[test]
fn config_will_not_replace_a_file_it_did_not_link() -> Result<()> {
    let dir = tempdir()?;
    let mise = config_mise(&dir)?;
    let (file, link) = (
        dir.path().join("dev/mise.toml"),
        dir.path().join("config/mise/conf.d/dev.toml"),
    );
    fs::create_dir_all(link.parent().unwrap())?;
    fs::write(&link, "[tools]\njq = \"1.7\"\n")?;
    let task = Config::new(tools(&[("node", "lts")]))
        .with_mise(&mise.path)
        .with_file(&file)
        .with_link(&link);
    let mut ctx = capturing_ctx().0;

    assert!(!task.is_applied(&mut ctx)?);
    let msg = task
        .apply(&mut ctx)
        .expect_err("an existing file should fail")
        .to_string();
    assert_eq!(
        format!(
            "{} already exists and isn't a link to {}; move it aside so dev can link its mise config there",
            link.display(),
            file.display()
        ),
        msg
    );
    assert_eq!("[tools]\njq = \"1.7\"\n", fs::read_to_string(&link)?);
    assert_eq!(Vec::<String>::new(), mise.calls()?, "no tools should be set");
    Ok(())
}

#[test]
fn config_is_not_applied_when_a_version_changed() -> Result<()> {
    let dir = tempdir()?;
    let mise = config_mise(&dir)?;
    let (file, link) = (
        dir.path().join("dev/mise.toml"),
        dir.path().join("config/mise/conf.d/dev.toml"),
    );
    let configure = |pairs: &[(&str, &str)]| {
        Config::new(tools(pairs))
            .with_mise(&mise.path)
            .with_file(&file)
            .with_link(&link)
    };
    let mut ctx = capturing_ctx().0;
    configure(&[("node", "22"), ("python", "3.14")]).apply(&mut ctx)?;

    let task = configure(&[("node", "24"), ("python", "3.14")]);
    assert!(!task.is_applied(&mut ctx)?);
    task.apply(&mut ctx)?;
    assert!(task.is_applied(&mut ctx)?);
    Ok(())
}

/// A fake `mise` whose `install --dry-run-code` exits `dry_run_code` and
/// whose `install` prints what it installed and the ctx env it saw. Each
/// call's working dir and arguments go to `dir/calls`.
fn install_mise(dir: &TempDir, dry_run_code: i32) -> Result<FakeMise> {
    let calls = dir.path().join("calls");
    let body = format!(
        r#"echo "$(pwd): $*" >> '{calls}'
[ "$2" = --dry-run-code ] && exit {dry_run_code}
echo "installed $2 (mark: $DEV_MARK)""#,
        calls = calls.display(),
    );
    Ok(FakeMise {
        path: script(dir, "mise", &body)?,
        calls,
    })
}

#[test]
fn tool_is_applied_when_mise_has_nothing_to_install() -> Result<()> {
    let dir = tempdir()?;
    let mise = install_mise(&dir, 0)?;
    let tool = Tool::new("node", "lts").with_mise(&mise.path);

    assert_eq!("Installing node@lts", tool.name());
    assert!(tool.is_applied(&mut capturing_ctx().0)?);
    assert_eq!(vec!["/: install --dry-run-code node@lts"], mise.calls()?);
    Ok(())
}

#[test]
fn tool_installs_with_ctx_env_when_mise_has_work_to_do() -> Result<()> {
    let dir = tempdir()?;
    let mise = install_mise(&dir, 1)?;
    let tool = Tool::new("python", "3.14").with_mise(&mise.path);
    let (ctx, out, _) = capturing_ctx();
    let mut ctx = Context {
        env: vec![("DEV_MARK".into(), "from dev.yaml".into())],
        ..ctx
    };

    assert!(!tool.is_applied(&mut ctx)?);
    tool.apply(&mut ctx)?;
    assert_eq!("installed python@3.14 (mark: from dev.yaml)\n", out.to_string());
    assert_eq!(
        vec!["/: install --dry-run-code python@3.14", "/: install python@3.14"],
        mise.calls()?
    );
    Ok(())
}
