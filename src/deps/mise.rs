use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    io::{ErrorKind, Write},
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::Command,
};

use color_eyre::eyre::{WrapErr, bail, eyre};
use serde::Deserialize;

use crate::{
    deps::{Dependency, Example, Task},
    layout,
    process,
    shell::{ShellDir, config_home},
};

const INSTALL_URL: &str = "https://mise.run";
/// mise's file in the [`ShellDir`], listed in
/// [`SETUP_FILES`](crate::shell::SETUP_FILES).
pub const SHELL_FILE: &str = "mise";

/// Where a project's tools go, relative to its root. `.dev` is git-ignored
/// (see [`RunLog`](crate::run_log::RunLog)).
pub const PROJECT_CONFIG: &str = ".dev/mise.toml";

/// Set to [`PROJECT_CONFIG`] in the env of a project with tools. mise adds
/// it to the config filenames it looks for in each dir, so it loads the
/// project's tools there and nowhere else. Must match
/// `share/action/dev-action.sh`.
pub const CONFIG_FILENAME_VAR: &str = "MISE_DEFAULT_CONFIG_FILENAME";

/// A tool for mise: its name, version, and options, e.g. `aws` at `2.37`
/// with `symlink_bins = "true"`.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub version: String,
    /// Passed to mise as is, in order. mise reads option values as strings.
    pub options: Vec<(String, String)>,
}

impl ToolSpec {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            options: Vec::new(),
        }
    }

    /// Adds option `key` with `value`.
    pub fn with_option(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.options.push((key.into(), value.into()));
        self
    }
}

/// Installs mise and a set of tools with it.
///
/// The tools are written to one of dev's own mise configs (see [`Config`]).
/// The user's own mise configs are never touched.
pub struct Mise {
    tasks: Vec<Box<dyn Task>>,
    data_dir: Option<PathBuf>,
}

impl Mise {
    /// Creates a dependency that installs mise, writes `tools` to dev's
    /// global mise config, then installs each tool in order. See
    /// [`Config::global`].
    pub fn global(tools: Vec<ToolSpec>) -> Self {
        Self::with_config(Config::global(tools.clone()), tools)
    }

    /// Like [`Mise::global`], but for the project in `root`. See
    /// [`Config::project`].
    pub fn project(root: impl Into<PathBuf>, tools: Vec<ToolSpec>) -> Self {
        Self::with_config(Config::project(root, tools.clone()), tools)
    }

    fn with_config(config: Config, tools: Vec<ToolSpec>) -> Self {
        let mut tasks: Vec<Box<dyn Task>> = vec![Box::new(Binary::default()), Box::new(config)];
        tasks.extend(
            tools
                .into_iter()
                .map(|t| Box::new(Tool::new(t.name, t.version).with_options(t.options)) as Box<dyn Task>),
        );
        Self::with_tasks(tasks)
    }

    /// Like [`Mise::global`], but with an explicit list of tasks to run, in
    /// order.
    pub fn with_tasks(tasks: Vec<Box<dyn Task>>) -> Self {
        Self { tasks, data_dir: None }
    }

    /// Uses `dir` as mise's data dir (`$MISE_DATA_DIR`) when putting its
    /// shims on `PATH`.
    pub fn with_data_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(dir.into());
        self
    }

    /// Puts mise's shims on `PATH` for every command run after this, so
    /// later dependencies in the same `dev up` (e.g. installing node
    /// packages) find the tools, even when the shell running dev hasn't
    /// activated mise yet.
    ///
    /// Done when checking as well as meeting, like
    /// [`Rust`](super::rust::Rust) does with cargo.
    fn use_shims(&self, ctx: &mut crate::Context) -> crate::Result<()> {
        let data_dir = match &self.data_dir {
            Some(dir) => dir.clone(),
            None => default_data_dir()?,
        };
        super::prepend_path(ctx, &data_dir.join("shims"))
    }
}

/// `$MISE_DATA_DIR`, or `mise` in the XDG data dir, as mise itself has it.
fn default_data_dir() -> crate::Result<PathBuf> {
    layout::mise_data(&layout::process)
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
        self.use_shims(ctx)?;
        super::meet_tasks(&self.tasks, ctx)
    }

    fn is_met(&self, ctx: &mut crate::Context) -> crate::Result<bool> {
        self.use_shims(ctx)?;
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
    /// Output from the installer and `mise self-update` goes to `ctx.log` as
    /// they run.
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

/// The header of a global [`Config`].
const GLOBAL_HEADER: &str = "# Written by `dev up --global` from your global dev.yaml, and rewritten on every run.
# Change dev.yaml instead of editing this file.
";

/// The header of a project's [`Config`].
const PROJECT_HEADER: &str = "# Written by `dev up` from this project's dev.yaml, and rewritten on every run.
# Change dev.yaml instead of editing this file.
";

/// Writes one of dev's mise configs, holding exactly the tools given. dev
/// owns the file, so anything else in it is replaced.
///
/// - [`Config::global`]: `~/.config/dev/mise.toml`, linked into mise's `conf.d`
///   so mise loads it everywhere. The user's own global config wins when both
///   pin the same tool.
/// - [`Config::project`]: `<project>/.dev/mise.toml`, which mise only loads in
///   that project, through [`CONFIG_FILENAME_VAR`] in the project's env. mise
///   only loads a project file with tool options once it's trusted, so dev
///   trusts it then: `dev up` installs with those options anyway.
pub struct Config {
    tools: Vec<ToolSpec>,
    /// `None` means the mise [`Binary`] finds, resolved when needed.
    mise: Option<PathBuf>,
    /// Whether to `mise trust` the file when a tool has options.
    trust_options: bool,
    /// `None` means `$XDG_CONFIG_HOME/dev/mise.toml` (`~/.config/dev/...`).
    file: Option<PathBuf>,
    link: Link,
    header: &'static str,
}

/// Where a [`Config`] is linked from.
enum Link {
    /// Not linked: a project's config.
    None,
    /// `$MISE_CONFIG_DIR/conf.d/dev.toml`, where `MISE_CONFIG_DIR` defaults
    /// to `$XDG_CONFIG_HOME/mise`, as in mise.
    Default,
    At(PathBuf),
}

impl Config {
    /// dev's global config, linked into mise's `conf.d`.
    pub fn global(tools: Vec<ToolSpec>) -> Self {
        Self {
            tools,
            mise: None,
            // mise trusts its own conf.d.
            trust_options: false,
            file: None,
            link: Link::Default,
            header: GLOBAL_HEADER,
        }
    }

    /// The config for the project in `root`, at `<root>/.dev/mise.toml`.
    pub fn project(root: impl Into<PathBuf>, tools: Vec<ToolSpec>) -> Self {
        Self {
            tools,
            mise: None,
            trust_options: true,
            file: Some(root.into().join(PROJECT_CONFIG)),
            link: Link::None,
            header: PROJECT_HEADER,
        }
    }

    /// Writes the tools to `path` instead of the default.
    pub fn with_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.file = Some(path.into());
        self
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

    /// Links the config from `path` instead of the default.
    pub fn with_link(mut self, path: impl Into<PathBuf>) -> Self {
        self.link = Link::At(path.into());
        self
    }

    fn file(&self) -> crate::Result<PathBuf> {
        match &self.file {
            Some(path) => Ok(path.clone()),
            None => Ok(config_home()?.join("dev/mise.toml")),
        }
    }

    fn link(&self) -> crate::Result<Option<PathBuf>> {
        match &self.link {
            Link::None => Ok(None),
            Link::Default => Ok(Some(mise_config_dir()?.join("conf.d/dev.toml"))),
            Link::At(path) => Ok(Some(path.clone())),
        }
    }

    /// What the file should hold.
    fn contents(&self) -> String {
        let mut out = format!("{}[tools]\n", self.header);
        for tool in &self.tools {
            let value = if tool.options.is_empty() {
                toml_string(&tool.version)
            } else {
                let options: String = tool
                    .options
                    .iter()
                    .map(|(key, value)| format!(", {} = {}", toml_string(key), toml_string(value)))
                    .collect();
                format!("{{ version = {}{options} }}", toml_string(&tool.version))
            };
            out.push_str(&format!("{} = {value}\n", toml_string(&tool.name)));
        }
        out
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

    /// Links the config into mise's `conf.d` (global only), then replaces it
    /// with one holding exactly the tools. The new file is written next to
    /// the old one and renamed into place, so mise never reads half of it.
    ///
    /// A project's file with tool options is then trusted with `mise trust`,
    /// whose output goes to `ctx.log`.
    fn apply(&self, ctx: &mut crate::Context) -> crate::Result<()> {
        let file = self.file()?;
        let dir = file
            .parent()
            .ok_or_else(|| eyre!("expected {} to be a file in a directory", file.display()))?;
        // Link first, so a link that can't be made leaves the config as it was.
        if let Some(link) = self.link()? {
            Self::ensure_link(&link, &file)?;
        }

        fs::create_dir_all(dir).wrap_err_with(|| format!("failed to create {}", dir.display()))?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir)
            .wrap_err_with(|| format!("failed to create a temp file in {}", dir.display()))?;
        tmp.write_all(self.contents().as_bytes())
            .wrap_err_with(|| format!("failed to write {}", tmp.path().display()))?;
        tmp.persist(&file)
            .wrap_err_with(|| format!("failed to replace {}", file.display()))?;

        if self.trust_options && self.tools.iter().any(|t| !t.options.is_empty()) {
            // mise runs from `/`, where a relative path means something else.
            let file = fs::canonicalize(&file).wrap_err_with(|| format!("failed to resolve {}", file.display()))?;
            let mise = self.mise()?;
            let mut cmd = mise_command(ctx, &mise);
            process::run(ctx, cmd.arg("trust").arg(&file))
                .wrap_err_with(|| format!("`{} trust {}` failed", mise.display(), file.display()))?;
        }
        Ok(())
    }

    /// Returns true if the config holds exactly the tools, and (global only)
    /// is linked into mise's `conf.d`.
    fn is_applied(&self, _ctx: &mut crate::Context) -> crate::Result<bool> {
        let file = self.file()?;
        if let Some(link) = self.link()?
            && fs::read_link(&link).ok().as_deref() != Some(file.as_path())
        {
            return Ok(false);
        }

        match fs::read_to_string(&file) {
            Ok(contents) => Ok(contents == self.contents()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e).wrap_err_with(|| format!("failed to read {}", file.display())),
        }
    }
}

/// Installs one tool version with mise, e.g. `node@lts`.
pub struct Tool {
    /// The tool's name, e.g. `node`.
    tool: String,
    version: String,
    /// Shown as the task name, e.g. `Installing node@lts`.
    label: String,
    /// `<name>@<version>`, or `<name>[k=v,...]@<version>` with options, as
    /// mise takes it.
    spec: String,
    /// `None` means the mise [`Binary`] finds, resolved when needed.
    mise: Option<PathBuf>,
}

impl Tool {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        let (tool, version) = (name.into(), version.into());
        let spec = format!("{tool}@{version}");
        Self {
            tool,
            version,
            label: format!("Installing {spec}"),
            spec,
            mise: None,
        }
    }

    /// Installs with `options` (key and value), which mise needs when the
    /// tool is installed, not only when it's used: dev installs from `/`,
    /// where a project's own config isn't loaded. The label stays as is.
    pub fn with_options(mut self, options: Vec<(String, String)>) -> Self {
        if options.is_empty() {
            return self;
        }
        let options: Vec<String> = options.iter().map(|(k, v)| format!("{k}={v}")).collect();
        self.spec = format!("{}[{}]@{}", self.tool, options.join(","), self.version);
        self
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

    /// Runs `mise install <name>@<version>`, with its output going to
    /// `ctx.log` as it runs.
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

/// `s` as a TOML basic string, e.g. `go:golang.org/x/tools/gopls` ->
/// `"go:golang.org/x/tools/gopls"`. Quoting every key keeps names with dots
/// one key.
fn toml_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The mise [`Binary`] would use, with the default `PATH` and install
/// location.
fn default_mise() -> crate::Result<PathBuf> {
    Ok(Binary::default().locate()?.path().to_owned())
}

/// Where the installer puts mise: `$MISE_INSTALL_PATH`, or
/// `~/.local/bin/mise`.
fn default_install_path() -> crate::Result<PathBuf> {
    layout::mise_bin(&layout::process)
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
