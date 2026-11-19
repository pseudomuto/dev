mod common;

use std::{fs, path::PathBuf};

use common::capturing_ctx;
use dev::{
    Context,
    Result,
    deps::{
        Dependency,
        Task,
        mise::{Binary, Config, Mise, Tool, ToolSpec},
    },
};
use tempfile::{TempDir, tempdir};

/// Writes an executable shell script to `dir/name` and returns its path.
fn script(dir: &TempDir, name: &str, body: &str) -> Result<PathBuf> {
    let path = dir.path().join(name);
    common::write_executable(
        &path,
        format!(
            "#!/bin/sh
{body}
"
        )
        .as_bytes(),
    )?;
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
    let (mut ctx, out, log) = capturing_ctx();

    assert!(!task.is_applied(&mut ctx)?);
    task.apply(&mut ctx)?;
    assert!(
        log.to_string()
            .ends_with(&format!("\ninstalled to {}\nprogress\n", install_path.display())),
        "log: {log}"
    );
    assert_eq!("", out.to_string());
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
    let (mut ctx, _, log) = capturing_ctx();

    let msg = task
        .apply(&mut ctx)
        .expect_err("a missing installer should fail")
        .to_string();
    assert_eq!(format!("mise installer ({url}) failed"), msg);
    assert!(
        log.to_string().contains("curl: (37)"),
        "expected curl's error in the log: {log}"
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

fn tools(pairs: &[(&str, &str)]) -> Vec<ToolSpec> {
    pairs.iter().map(|(n, v)| ToolSpec::new(*n, *v)).collect()
}

const GLOBAL_HEADER: &str = "# Written by `dev up --global` from your global dev.yaml, and rewritten on every run.\n# Change dev.yaml instead of editing this file.\n";
const PROJECT_HEADER: &str = "# Written by `dev up` from this project's dev.yaml, and rewritten on every run.\n# Change dev.yaml instead of editing this file.\n";

#[test]
fn config_writes_the_tools_and_links_the_file_into_conf_d() -> Result<()> {
    let dir = tempdir()?;
    let (file, link) = (
        dir.path().join("dev/mise.toml"),
        dir.path().join("config/mise/conf.d/dev.toml"),
    );
    let task = Config::global(tools(&[("node", "lts"), ("go:golang.org/x/tools/gopls", "latest")]))
        .with_file(&file)
        .with_link(&link);
    let mut ctx = capturing_ctx().0;

    assert!(!task.is_applied(&mut ctx)?);
    task.apply(&mut ctx)?;
    assert_eq!(file, fs::read_link(&link)?);
    assert_eq!(
        format!("{GLOBAL_HEADER}[tools]\n\"node\" = \"lts\"\n\"go:golang.org/x/tools/gopls\" = \"latest\"\n"),
        fs::read_to_string(&file)?
    );
    assert!(task.is_applied(&mut ctx)?);
    Ok(())
}

#[test]
fn config_drops_tools_that_are_no_longer_listed() -> Result<()> {
    let dir = tempdir()?;
    let (file, link) = (
        dir.path().join("dev/mise.toml"),
        dir.path().join("config/mise/conf.d/dev.toml"),
    );
    fs::create_dir_all(file.parent().unwrap())?;
    fs::write(&file, "[tools]\nshellcheck = \"0.11.0\"\nnode = \"26\"\n")?;
    let task = Config::global(tools(&[("node", "24")]))
        .with_file(&file)
        .with_link(&link);
    let mut ctx = capturing_ctx().0;

    assert!(!task.is_applied(&mut ctx)?);
    task.apply(&mut ctx)?;
    assert_eq!(
        format!("{GLOBAL_HEADER}[tools]\n\"node\" = \"24\"\n"),
        fs::read_to_string(&file)?
    );
    assert!(task.is_applied(&mut ctx)?);
    Ok(())
}

#[test]
fn config_for_a_project_writes_dev_mise_toml_without_a_link() -> Result<()> {
    let dir = tempdir()?;
    let task = Config::project(dir.path(), tools(&[("jq", "1.7")]));
    let mut ctx = capturing_ctx().0;

    assert!(!task.is_applied(&mut ctx)?);
    task.apply(&mut ctx)?;
    let file = dir.path().join(".dev/mise.toml");
    assert_eq!(
        format!("{PROJECT_HEADER}[tools]\n\"jq\" = \"1.7\"\n"),
        fs::read_to_string(&file)?
    );
    assert!(
        fs::symlink_metadata(&file)?.is_file(),
        "a project's file is never a link"
    );
    assert!(task.is_applied(&mut ctx)?);
    Ok(())
}

#[test]
fn config_quotes_keys_and_values_for_toml() -> Result<()> {
    let dir = tempdir()?;
    let task = Config::project(dir.path(), tools(&[("a\"b\\c", "1\t2")]));
    task.apply(&mut capturing_ctx().0)?;
    assert_eq!(
        format!("{PROJECT_HEADER}[tools]\n\"a\\\"b\\\\c\" = \"1\\u00092\"\n"),
        fs::read_to_string(dir.path().join(".dev/mise.toml"))?
    );
    Ok(())
}

#[test]
fn config_writes_options_as_an_inline_table() -> Result<()> {
    let dir = tempdir()?;
    let aws = ToolSpec::new("aws", "2.37")
        .with_option("symlink_bins", "true")
        .with_option("exe", "aws");
    // A fake mise: tools with options get `mise trust`, which would write
    // to the real mise's trust store.
    let mise = install_mise(&dir, 0)?;
    let task = Config::project(dir.path(), vec![aws, ToolSpec::new("jq", "1.7")]).with_mise(&mise.path);
    task.apply(&mut capturing_ctx().0)?;
    assert_eq!(
        format!(
            "{PROJECT_HEADER}[tools]\n\"aws\" = {{ version = \"2.37\", \"symlink_bins\" = \"true\", \"exe\" = \"aws\" }}\n\"jq\" = \"1.7\"\n"
        ),
        fs::read_to_string(dir.path().join(".dev/mise.toml"))?
    );
    Ok(())
}

#[test]
fn config_trusts_a_project_file_with_options_so_mise_loads_it() -> Result<()> {
    let dir = tempdir()?;
    let mise = install_mise(&dir, 0)?;
    let aws = ToolSpec::new("aws", "2.37").with_option("symlink_bins", "true");
    let task = Config::project(dir.path(), vec![aws]).with_mise(&mise.path);
    task.apply(&mut capturing_ctx().0)?;

    // By its real path: on macOS, temp dirs are under a /var symlink.
    let file = fs::canonicalize(dir.path())?.join(".dev/mise.toml");
    assert_eq!(vec![format!("/: trust {}", file.display())], mise.calls()?);
    Ok(())
}

#[test]
fn config_trusts_a_relative_project_file_by_its_absolute_path() -> Result<()> {
    // mise runs from `/`, so a relative path would point somewhere else. A
    // project loaded from ./dev.yaml has the root `.`; this is the same
    // thing, relative to the test's working dir, without changing it.
    let dir = tempdir()?;
    let cwd = std::env::current_dir()?;
    let up = "../".repeat(cwd.components().count() - 1);
    let abs = fs::canonicalize(dir.path())?;
    let root = PathBuf::from(up).join(abs.strip_prefix("/")?);
    assert!(root.is_relative());

    let mise = install_mise(&dir, 0)?;
    let aws = ToolSpec::new("aws", "2.37").with_option("symlink_bins", "true");
    Config::project(&root, vec![aws])
        .with_mise(&mise.path)
        .apply(&mut capturing_ctx().0)?;

    assert_eq!(
        vec![format!("/: trust {}", abs.join(".dev/mise.toml").display())],
        mise.calls()?
    );
    Ok(())
}

#[test]
fn config_trusts_nothing_without_options_or_for_the_global_file() -> Result<()> {
    let dir = tempdir()?;
    let mise = install_mise(&dir, 0)?;
    let mut ctx = capturing_ctx().0;
    Config::project(dir.path(), tools(&[("jq", "1.7")]))
        .with_mise(&mise.path)
        .apply(&mut ctx)?;
    let aws = ToolSpec::new("aws", "2.37").with_option("symlink_bins", "true");
    Config::global(vec![aws])
        .with_mise(&mise.path)
        .with_file(dir.path().join("dev/mise.toml"))
        .with_link(dir.path().join("config/mise/conf.d/dev.toml"))
        .apply(&mut ctx)?;

    assert_eq!(Vec::<String>::new(), mise.calls()?);
    Ok(())
}

#[test]
fn config_will_not_replace_a_file_it_did_not_link() -> Result<()> {
    let dir = tempdir()?;
    let (file, link) = (
        dir.path().join("dev/mise.toml"),
        dir.path().join("config/mise/conf.d/dev.toml"),
    );
    fs::create_dir_all(link.parent().unwrap())?;
    fs::write(&link, "[tools]\njq = \"1.7\"\n")?;
    let task = Config::global(tools(&[("node", "lts")]))
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
    assert!(!file.exists(), "dev's config should be left as it was");
    Ok(())
}

#[test]
fn config_is_not_applied_when_a_version_changed() -> Result<()> {
    let dir = tempdir()?;
    let configure = |pairs: &[(&str, &str)]| Config::project(dir.path(), tools(pairs));
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
fn tool_installs_with_its_options() -> Result<()> {
    let dir = tempdir()?;
    let mise = install_mise(&dir, 1)?;
    let tool = Tool::new("aws", "2.37")
        .with_options(vec![
            ("symlink_bins".to_string(), "true".to_string()),
            ("exe".to_string(), "aws".to_string()),
        ])
        .with_mise(&mise.path);
    let mut ctx = capturing_ctx().0;

    assert_eq!("Installing aws@2.37", tool.name(), "options would just be noise");
    assert!(!tool.is_applied(&mut ctx)?);
    tool.apply(&mut ctx)?;
    assert_eq!(
        vec![
            "/: install --dry-run-code aws[symlink_bins=true,exe=aws]@2.37",
            "/: install aws[symlink_bins=true,exe=aws]@2.37",
        ],
        mise.calls()?
    );
    Ok(())
}

#[test]
fn tool_installs_with_ctx_env_when_mise_has_work_to_do() -> Result<()> {
    let dir = tempdir()?;
    let mise = install_mise(&dir, 1)?;
    let tool = Tool::new("python", "3.14").with_mise(&mise.path);
    let (ctx, _, log) = capturing_ctx();
    let mut ctx = Context {
        env: vec![("DEV_MARK".into(), "from dev.yaml".into())],
        ..ctx
    };

    assert!(!tool.is_applied(&mut ctx)?);
    tool.apply(&mut ctx)?;
    assert_eq!(
        format!(
            "$ {} install python@3.14\ninstalled python@3.14 (mark: from dev.yaml)\n",
            mise.path.display()
        ),
        log.to_string()
    );
    assert_eq!(
        vec!["/: install --dry-run-code python@3.14", "/: install python@3.14"],
        mise.calls()?
    );
    Ok(())
}

#[test]
fn mise_puts_its_shims_on_path_for_what_comes_after() -> Result<()> {
    let dir = tempdir()?;
    let data_dir = dir.path().join("mise");
    let dep = Mise::with_tasks(Vec::new()).with_data_dir(&data_dir);
    let (mut ctx, _, _) = capturing_ctx();
    ctx.env = vec![("PATH".to_string(), "/proj/bin:/usr/bin".to_string())];

    assert!(dep.is_met(&mut ctx)?, "no tasks, so met");
    dep.is_met(&mut ctx)?;

    let want = format!("{}/shims:/proj/bin:/usr/bin", data_dir.display());
    assert_eq!(
        vec![("PATH".to_string(), want)],
        ctx.env,
        "first, once, and keeping the project's"
    );
    Ok(())
}

#[test]
fn binary_uses_an_external_mise_on_the_ctx_path() -> Result<()> {
    let dir = tempdir()?;
    let mise = FakeMise::new(&dir, "external/mise", "not json")?;
    let task = Binary::default()
        .with_install_path(dir.path().join("managed/mise"))
        .with_install_url(format!("file://{}", dir.path().join("missing").display()))
        .with_shell_dir(dir.path().join("shell"));
    let mut ctx = capturing_ctx().0;
    ctx.env = vec![("PATH".into(), dir.path().join("external").display().to_string())];

    task.apply(&mut ctx)?;
    assert_eq!(
        setup_for(&mise.path),
        fs::read_to_string(dir.path().join("shell/mise"))?
    );
    assert!(!dir.path().join("managed/mise").exists());
    Ok(())
}
