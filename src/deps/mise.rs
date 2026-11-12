use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    io::ErrorKind,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::Command,
};

use color_eyre::eyre::{WrapErr, bail, eyre};
use serde::Deserialize;

use crate::{
    deps::{Dependency, Example, Task},
    process,
    shell::{ShellDir, config_home, home},
};

const INSTALL_URL: &str = "https://mise.run";
/// mise's file in the [`ShellDir`]. Must match `share/shell`.
const SHELL_FILE: &str = "mise";

/// Installs mise and a set of tools with it.
///
/// The tools are added to dev's own mise config
/// (`~/.config/dev/mise.toml`), which mise loads through a link in its
/// `conf.d`. The user's own global config is never touched, and wins when
/// both pin the same tool.
pub struct Mise {
    tasks: Vec<Box<dyn Task>>,
}

impl Mise {
    /// Creates a dependency that installs mise, adds `tools` (name and
    /// version, e.g. `("node", "lts")`) to dev's mise config, then installs
    /// each tool in order.
    pub fn new(tools: impl IntoIterator<Item = (String, String)>) -> Self {
        let tools: Vec<(String, String)> = tools.into_iter().collect();
        let mut tasks: Vec<Box<dyn Task>> = vec![Box::new(Binary::default()), Box::new(Config::new(tools.clone()))];
        tasks.extend(
            tools
                .into_iter()
                .map(|(name, version)| Box::new(Tool::new(name, version)) as Box<dyn Task>),
        );
        Self::with_tasks(tasks)
    }

    /// Like [`Mise::new`], but with an explicit list of tasks to run, in
    /// order.
    pub fn with_tasks(tasks: Vec<Box<dyn Task>>) -> Self {
        Self { tasks }
    }
}

impl Dependency for Mise {
    fn name(&self) -> &str {
        "Install tools"
    }

    fn desc(&self) -> &str {
        "Install/update mise and the tools listed under deps"
    }

    fn tasks(&self) -> &[Box<dyn Task>] {
        &self.tasks
    }

    fn examples(&self) -> Vec<Example> {
        vec![Example::new(
            "Require tools",
            r#"deps:
              node: lts
              python: 3.14
              "cargo:cargo-shear": latest
                "#,
        )]
    }

    fn meet(&self, ctx: &mut crate::Context) -> crate::Result<()> {
        super::meet_tasks(&self.tasks, ctx)
    }

    fn is_met(&self, ctx: &mut crate::Context) -> crate::Result<bool> {
        super::tasks_met(&self.tasks, ctx)
    }

    fn down(&self, _ctx: &mut crate::Context) -> crate::Result<()> {
        Ok(())
    }
}

/// Makes sure mise is installed and its shell setup is written.
///
/// A mise already on `PATH` that dev didn't install (e.g. Homebrew's) is used
/// as is: whatever installed it keeps it up to date. Otherwise dev installs
/// mise to `$MISE_INSTALL_PATH` (`~/.local/bin/mise` by default) and keeps it
/// current with `mise self-update`.
///
/// The defaults use the real `PATH`, install location and installer. The
/// `with_*` methods replace those, so tests can run without the network.
pub struct Binary {
    /// Where to look for an existing mise. `None` means `$PATH`.
    search_path: Option<OsString>,
    /// Where dev installs mise. `None` means `$MISE_INSTALL_PATH` or
    /// `~/.local/bin/mise`, resolved when needed.
    install_path: Option<PathBuf>,
    /// Where to write mise's shell setup, as `<dir>/mise`. `None` means
    /// [`ShellDir::user_default`], resolved when needed.
    shell_dir: Option<PathBuf>,
    install_url: String,
}

impl Default for Binary {
    fn default() -> Self {
        Self {
            search_path: None,
            install_path: None,
            shell_dir: None,
            install_url: INSTALL_URL.into(),
        }
    }
}

/// Which mise to use, and who looks after it.
enum Found {
    /// Not installed. dev will install it here.
    Missing(PathBuf),
    /// Installed where dev installs it, so dev keeps it current.
    Managed(PathBuf),
    /// Installed by something else, which keeps it current.
    External(PathBuf),
}

impl Found {
    fn path(&self) -> &Path {
        match self {
            Found::Missing(p) | Found::Managed(p) | Found::External(p) => p,
        }
    }
}

impl Binary {
    /// Looks for an existing mise in `path` (`PATH` syntax) instead of
    /// `$PATH`.
    pub fn with_search_path(mut self, path: impl Into<OsString>) -> Self {
        self.search_path = Some(path.into());
        self
    }

    /// Installs mise to `path` instead of the default.
    pub fn with_install_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.install_path = Some(path.into());
        self
    }

    /// Writes mise's shell setup into `dir` instead of the default.
    pub fn with_shell_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.shell_dir = Some(dir.into());
        self
    }

    /// Fetches the installer script from `url` instead of mise.run. Any URL
    /// curl understands works, including `file://`.
    pub fn with_install_url(mut self, url: impl Into<String>) -> Self {
        self.install_url = url.into();
        self
    }

    fn locate(&self) -> crate::Result<Found> {
        let install = match &self.install_path {
            Some(path) => path.clone(),
            None => default_install_path()?,
        };
        let search = match &self.search_path {
            Some(path) => path.clone(),
            None => env::var_os("PATH").unwrap_or_default(),
        };

        Ok(match find_on_path(&search, "mise") {
            Some(path) if path == install => Found::Managed(path),
            Some(path) => Found::External(path),
            None if is_executable(&install) => Found::Managed(install),
            None => Found::Missing(install),
        })
    }

    fn shell_dir(&self) -> crate::Result<ShellDir> {
        match &self.shell_dir {
            Some(dir) => Ok(ShellDir::new(dir)),
            None => ShellDir::user_default(),
        }
    }

    /// Runs the mise installer, which installs mise at `path`.
    fn install(&self, ctx: &mut crate::Context, path: &Path) -> crate::Result<()> {
        // pipefail matters: without it, a failed curl pipes nothing into sh,
        // which then exits 0 and the install looks like it succeeded.
        let mut cmd = ctx.command("bash");
        cmd.args([
            "-c",
            r#"set -o pipefail; curl -fsSL "$1" | sh"#,
            "bash",
            &self.install_url,
        ])
        .env("MISE_INSTALL_PATH", path);

        process::run(ctx, &mut cmd).wrap_err_with(|| format!("mise installer ({}) failed", self.install_url))?;

        if !is_executable(path) {
            bail!(
                "the mise installer ({}) finished, but there's no mise at {}",
                self.install_url,
                path.display()
            );
        }
        Ok(())
    }
}

impl Task for Binary {
    fn name(&self) -> &str {
        "Install mise"
    }

    /// Installs mise when it's missing, or updates the one dev installed when
    /// it isn't current, then writes the shell setup for it.
    ///
    /// Output from the installer and `mise self-update` is copied to `ctx.out`
    /// and `ctx.err` as they run.
    fn apply(&self, ctx: &mut crate::Context) -> crate::Result<()> {
        let mise = match self.locate()? {
            Found::Missing(path) => {
                self.install(ctx, &path)?;
                path
            }
            Found::Managed(path) => {
                if !is_current(ctx, &path)? {
                    let mut cmd = mise_command(ctx, &path);
                    process::run(ctx, cmd.args(["self-update", "--yes"]))
                        .wrap_err_with(|| format!("`{} self-update --yes` failed", path.display()))?;
                }
                path
            }
            Found::External(path) => path,
        };

        self.shell_dir()?.write(SHELL_FILE, &shell_setup(&mise))
    }

    /// Returns true if mise is installed, its shell setup points at it, and
    /// (when dev installed it) it's the latest release.
    fn is_applied(&self, ctx: &mut crate::Context) -> crate::Result<bool> {
        let found = self.locate()?;
        let setup = self.shell_dir()?.file(SHELL_FILE);
        let current_setup = match fs::read_to_string(&setup) {
            Ok(contents) => contents == shell_setup(found.path()),
            Err(e) if e.kind() == ErrorKind::NotFound => false,
            Err(e) => return Err(e).wrap_err_with(|| format!("failed to read {}", setup.display())),
        };
        if !current_setup {
            return Ok(false);
        }

        match found {
            Found::Missing(_) => Ok(false),
            Found::External(_) => Ok(true),
            Found::Managed(path) => is_current(ctx, &path),
        }
    }
}

/// Adds tools to dev's mise config, and links that config into mise's
/// `conf.d` so mise loads it.
pub struct Config {
    /// Name and version, e.g. `("node", "lts")`.
    tools: Vec<(String, String)>,
    /// `None` means the mise [`Binary`] finds, resolved when needed.
    mise: Option<PathBuf>,
    /// `None` means `$XDG_CONFIG_HOME/dev/mise.toml` (`~/.config/dev/...`).
    file: Option<PathBuf>,
    /// `None` means `$MISE_CONFIG_DIR/conf.d/dev.toml`, where
    /// `MISE_CONFIG_DIR` defaults to `$XDG_CONFIG_HOME/mise`, as in mise.
    link: Option<PathBuf>,
}

impl Config {
    pub fn new(tools: Vec<(String, String)>) -> Self {
        Self {
            tools,
            mise: None,
            file: None,
            link: None,
        }
    }

    /// Uses the mise at `path` instead of the one [`Binary`] finds.
    pub fn with_mise(mut self, path: impl Into<PathBuf>) -> Self {
        self.mise = Some(path.into());
        self
    }

    /// Writes the tools to `path` instead of the default.
    pub fn with_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.file = Some(path.into());
        self
    }

    /// Links the config from `path` instead of the default.
    pub fn with_link(mut self, path: impl Into<PathBuf>) -> Self {
        self.link = Some(path.into());
        self
    }

    fn mise(&self) -> crate::Result<PathBuf> {
        match &self.mise {
            Some(path) => Ok(path.clone()),
            None => default_mise(),
        }
    }

    fn file(&self) -> crate::Result<PathBuf> {
        match &self.file {
            Some(path) => Ok(path.clone()),
            None => Ok(config_home()?.join("dev/mise.toml")),
        }
    }

    fn link(&self) -> crate::Result<PathBuf> {
        match &self.link {
            Some(path) => Ok(path.clone()),
            None => Ok(mise_config_dir()?.join("conf.d/dev.toml")),
        }
    }

    /// Links `link` to `file`, unless it already is. Fails rather than
    /// replace anything else that's there, since it isn't dev's.
    fn ensure_link(link: &Path, file: &Path) -> crate::Result<()> {
        match fs::symlink_metadata(link) {
            Ok(_) => match fs::read_link(link) {
                Ok(target) if target == file => Ok(()),
                _ => bail!(
                    "{} already exists and isn't a link to {}; move it aside so dev can link its mise config there",
                    link.display(),
                    file.display()
                ),
            },
            Err(e) if e.kind() == ErrorKind::NotFound => {
                let dir = link
                    .parent()
                    .ok_or_else(|| eyre!("expected {} to be a file in a directory", link.display()))?;
                fs::create_dir_all(dir).wrap_err_with(|| format!("failed to create {}", dir.display()))?;
                symlink(file, link).wrap_err_with(|| format!("failed to link {} to {}", link.display(), file.display()))
            }
            Err(e) => Err(e).wrap_err_with(|| format!("failed to check {}", link.display())),
        }
    }
}

impl Task for Config {
    fn name(&self) -> &str {
        "Configure tools"
    }

    /// Creates dev's mise config if needed, links it into mise's `conf.d`,
    /// and sets each tool's version in it, leaving other tools alone.
    fn apply(&self, ctx: &mut crate::Context) -> crate::Result<()> {
        let (mise, file) = (self.mise()?, self.file()?);
        let dir = file
            .parent()
            .ok_or_else(|| eyre!("expected {} to be a file in a directory", file.display()))?;
        fs::create_dir_all(dir).wrap_err_with(|| format!("failed to create {}", dir.display()))?;
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file)
            .wrap_err_with(|| format!("failed to create {}", file.display()))?;
        // Link first, so a link that can't be made leaves the config as it was.
        Self::ensure_link(&self.link()?, &file)?;

        for (name, version) in &self.tools {
            let key = tool_key(name);
            let display = format!(
                "{} config set --file {} {key} {version}",
                mise.display(),
                file.display()
            );
            let mut cmd = mise_command(ctx, &mise);
            cmd.args(["config", "set", "--file"]).arg(&file).args([&key, version]);
            process::output(&mut cmd, &display)?;
        }

        Ok(())
    }

    /// Returns true if the config is linked into mise's `conf.d` and sets
    /// every tool to its version.
    fn is_applied(&self, ctx: &mut crate::Context) -> crate::Result<bool> {
        let (file, link) = (self.file()?, self.link()?);
        if !file.exists() || fs::read_link(&link).ok().as_deref() != Some(file.as_path()) {
            return Ok(false);
        }

        let mise = self.mise()?;
        for (name, version) in &self.tools {
            // A non-zero exit means the key isn't set. Anything worse fails
            // loudly when `apply` runs `config set`.
            let output = mise_command(ctx, &mise)
                .args(["config", "get", "--file"])
                .arg(&file)
                .arg(tool_key(name))
                .output()
                .wrap_err_with(|| format!("failed to run `{} config get`", mise.display()))?;

            if !output.status.success() || String::from_utf8_lossy(&output.stdout).trim() != version {
                return Ok(false);
            }
        }

        Ok(true)
    }
}

/// Installs one tool version with mise, e.g. `node@lts`.
pub struct Tool {
    /// `<name>@<version>`, as mise takes it.
    spec: String,
    /// Shown as the task name, e.g. `Installing node@lts`.
    label: String,
    /// `None` means the mise [`Binary`] finds, resolved when needed.
    mise: Option<PathBuf>,
}

impl Tool {
    pub fn new(name: impl AsRef<str>, version: impl AsRef<str>) -> Self {
        let spec = format!("{}@{}", name.as_ref(), version.as_ref());
        Self {
            label: format!("Installing {spec}"),
            spec,
            mise: None,
        }
    }

    /// Uses the mise at `path` instead of the one [`Binary`] finds.
    pub fn with_mise(mut self, path: impl Into<PathBuf>) -> Self {
        self.mise = Some(path.into());
        self
    }

    fn mise(&self) -> crate::Result<PathBuf> {
        match &self.mise {
            Some(path) => Ok(path.clone()),
            None => default_mise(),
        }
    }
}

impl Task for Tool {
    fn name(&self) -> &str {
        &self.label
    }

    /// Runs `mise install <name>@<version>`, copying its output to `ctx.out`
    /// and `ctx.err` as it runs.
    fn apply(&self, ctx: &mut crate::Context) -> crate::Result<()> {
        let mise = self.mise()?;
        let mut cmd = mise_command(ctx, &mise);
        process::run(ctx, cmd.args(["install", &self.spec]))
            .wrap_err_with(|| format!("`{} install {}` failed", mise.display(), self.spec))
    }

    /// Returns true if mise has nothing to install for this tool.
    ///
    /// mise exits 1 both when the tool needs installing and when it can't
    /// find it, so an unknown tool only fails, with mise's error, in `apply`.
    fn is_applied(&self, ctx: &mut crate::Context) -> crate::Result<bool> {
        let mise = self.mise()?;
        let output = mise_command(ctx, &mise)
            .args(["install", "--dry-run-code", &self.spec])
            .output()
            .wrap_err_with(|| format!("failed to run `{} install --dry-run-code`", mise.display()))?;

        Ok(output.status.success())
    }
}

/// A command for `mise`, run from `/`.
///
/// mise reads the config files for its working directory. From a project,
/// that would pull in its (possibly untrusted) `mise.toml`, and from dev's
/// config dir, dev's own config as a project file. `/` has neither.
fn mise_command(ctx: &crate::Context, mise: &Path) -> Command {
    let mut cmd = ctx.command(mise);
    cmd.current_dir("/");
    cmd
}

/// What `mise version --json` reports.
#[derive(Deserialize)]
struct VersionInfo {
    /// e.g. `2026.11.6 macos-arm64 (2026-11-10)`.
    version: String,
    /// e.g. `2026.11.6`. Missing when mise couldn't check.
    latest: Option<String>,
}

/// Whether the mise at `path` is the latest release, as mise reports it.
fn is_current(ctx: &crate::Context, mise: &Path) -> crate::Result<bool> {
    let display = format!("{} version --json", mise.display());
    let output = process::output(mise_command(ctx, mise).args(["version", "--json"]), &display)?;
    let info: VersionInfo = serde_json::from_str(&output)
        .wrap_err_with(|| format!("expected version JSON from `{display}`, got {output:?}"))?;

    let installed = info
        .version
        .split_whitespace()
        .next()
        .ok_or_else(|| eyre!("`{display}` reported an empty version"))?;
    let latest = info.latest.ok_or_else(|| {
        eyre!("`{display}` didn't report the latest release, so mise can't be checked for updates; is the network up?")
    })?;

    Ok(installed == latest)
}

/// The shell setup for `mise`: runs `mise activate` for the running shell.
///
/// `mise activate` prints the `PATH` it was run with, so its output can't be
/// saved and replayed later. Running it when the setup loads keeps the shell's
/// own `PATH`.
fn shell_setup(mise: &Path) -> String {
    let mise = sh_quote(&mise.to_string_lossy());
    format!(
        r#"if [ -n "${{ZSH_VERSION:-}}" ]; then
  eval "$({mise} activate zsh)"
elif [ -n "${{BASH_VERSION:-}}" ]; then
  eval "$({mise} activate bash)"
fi
"#
    )
}

/// Quotes `s` for sh, e.g. `it's` -> `'it'\''s'`.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The `mise config` key for tool `name`, quoted so names with dots (e.g.
/// `go:golang.org/x/tools/gopls`) stay one key.
fn tool_key(name: &str) -> String {
    format!(r#"tools."{}""#, name.replace('\\', r"\\").replace('"', r#"\""#))
}

/// The mise [`Binary`] would use, with the default `PATH` and install
/// location.
fn default_mise() -> crate::Result<PathBuf> {
    Ok(Binary::default().locate()?.path().to_owned())
}

/// Where the installer puts mise: `$MISE_INSTALL_PATH`, or
/// `~/.local/bin/mise`.
fn default_install_path() -> crate::Result<PathBuf> {
    match env::var_os("MISE_INSTALL_PATH").filter(|p| !p.is_empty()) {
        Some(path) => Ok(PathBuf::from(path)),
        None => Ok(home()?.join(".local/bin/mise")),
    }
}

/// mise's config dir: `$MISE_CONFIG_DIR`, or `$XDG_CONFIG_HOME/mise`.
fn mise_config_dir() -> crate::Result<PathBuf> {
    match env::var_os("MISE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => Ok(config_home()?.join("mise")),
    }
}

/// The first executable `name` in `path` (`PATH` syntax). Relative entries
/// are skipped, so where dev runs from can't change which mise it finds.
fn find_on_path(path: &OsStr, name: &str) -> Option<PathBuf> {
    env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
