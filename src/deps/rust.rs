//! Installs rustup and Rust toolchains.
//!
//! ```yaml
//! deps:
//!   rust:
//!     - rust-toolchain.toml   # what that file names: channel, components, targets
//!     - nightly: [rustfmt]    # a toolchain with components
//!     - 1.99.0                # a toolchain on its own
//! ```

use std::{
    env,
    ffi::{OsStr, OsString},
    fmt,
    fs,
    io::ErrorKind,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use color_eyre::eyre::{WrapErr, bail, eyre};
use serde::{
    Deserialize,
    Deserializer,
    de::{self, MapAccess, Visitor},
};

use crate::{
    Context,
    deps::{Dependency, Example, Task},
    process,
    shell::{ShellDir, home},
};

const INSTALL_URL: &str = "https://sh.rustup.rs";
/// rustup's file in the [`ShellDir`]. Must match `share/shell`.
const SHELL_FILE: &str = "rustup";
/// The entry that means "whatever rust-toolchain.toml names".
const TOOLCHAIN_FILE: &str = "rust-toolchain.toml";

/// One item under `rust` in dev.yaml.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    /// `rust-toolchain.toml`: the toolchain that file (in the project dir)
    /// names, with its components and targets.
    File,
    /// A toolchain like `nightly` or `1.99.0`, and the components it needs.
    Named { name: String, components: Vec<String> },
}

impl<'de> Deserialize<'de> for Entry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        deserializer.deserialize_any(EntryVisitor)
    }
}

struct EntryVisitor;

impl<'de> Visitor<'de> for EntryVisitor {
    type Value = Entry;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // Numbers end up here too: YAML reads `1.80` as 1.8, so versions
        // like that have to be quoted to mean what they say.
        f.write_str(
            "`rust-toolchain.toml`, a toolchain like `nightly` or `\"1.80\"` (quoted, so YAML keeps it as text), \
             or a toolchain and its components like `nightly: [rustfmt]`",
        )
    }

    fn visit_str<E: de::Error>(self, s: &str) -> std::result::Result<Entry, E> {
        match s {
            TOOLCHAIN_FILE => Ok(Entry::File),
            "" => Err(E::custom("a toolchain name can't be empty")),
            name => Ok(Entry::Named {
                name: name.to_string(),
                components: Vec::new(),
            }),
        }
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Entry, A::Error> {
        let Some(name) = map.next_key::<String>()? else {
            return Err(de::Error::custom(
                "expected a toolchain and its components, e.g. `nightly: [rustfmt]`",
            ));
        };
        if name == TOOLCHAIN_FILE {
            return Err(de::Error::custom(
                "`rust-toolchain.toml` takes no components; list them in the file instead",
            ));
        }
        let components: Vec<String> = map.next_value()?;
        if let Some(other) = map.next_key::<String>()? {
            return Err(de::Error::custom(format!(
                "each entry is one toolchain; put `{other}` in its own entry"
            )));
        }
        Ok(Entry::Named { name, components })
    }
}

/// Installs rustup, then each toolchain in order.
pub struct Rust {
    tasks: Vec<Box<dyn Task>>,
    cargo_home: Option<PathBuf>,
}

impl Rust {
    /// Creates a dependency that installs rustup, then `entries` in order.
    pub fn new(entries: Vec<Entry>) -> Self {
        let mut tasks: Vec<Box<dyn Task>> = vec![Box::new(Rustup::default())];
        tasks.extend(
            entries
                .into_iter()
                .map(|entry| Box::new(Toolchain::new(entry)) as Box<dyn Task>),
        );
        Self::with_tasks(tasks)
    }

    /// Like [`Rust::new`], but with an explicit list of tasks to run, in
    /// order.
    pub fn with_tasks(tasks: Vec<Box<dyn Task>>) -> Self {
        Self {
            tasks,
            cargo_home: None,
        }
    }

    /// Uses `dir` as `$CARGO_HOME` when putting cargo's bin dir on `PATH`.
    pub fn with_cargo_home(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cargo_home = Some(dir.into());
        self
    }

    /// Puts `$CARGO_HOME/bin` on `PATH` for every command run after this, so
    /// later dependencies in the same `dev up` (e.g. a mise `cargo:` tool)
    /// find cargo, even when the shell running dev doesn't have it yet.
    ///
    /// Done when checking as well as meeting: a dependency that's already met
    /// is never met again, but what comes after it still needs cargo.
    fn use_cargo_bin(&self, ctx: &mut Context) -> crate::Result<()> {
        let cargo_home = match &self.cargo_home {
            Some(dir) => dir.clone(),
            None => default_cargo_home()?,
        };
        prepend_path(ctx, &cargo_home.join("bin"))
    }
}

impl Dependency for Rust {
    fn name(&self) -> &str {
        "Install Rust"
    }

    fn desc(&self) -> &str {
        "Install rustup and Rust toolchains"
    }

    fn examples(&self) -> Vec<Example> {
        vec![Example::new(
            "Toolchains",
            "deps:
  rust:
    - rust-toolchain.toml
    - nightly: [rustfmt]",
        )]
    }

    fn tasks(&self) -> &[Box<dyn Task>] {
        &self.tasks
    }

    fn meet(&self, ctx: &mut Context) -> crate::Result<()> {
        self.use_cargo_bin(ctx)?;
        super::meet_tasks(&self.tasks, ctx)
    }

    fn is_met(&self, ctx: &mut Context) -> crate::Result<bool> {
        self.use_cargo_bin(ctx)?;
        super::tasks_met(&self.tasks, ctx)
    }

    fn down(&self, _ctx: &mut Context) -> crate::Result<()> {
        Ok(())
    }
}

/// Puts `dir` first on the `PATH` that `ctx` gives commands, unless it's
/// already on it. Starts from a `PATH` the project env set, or else dev's
/// own.
fn prepend_path(ctx: &mut Context, dir: &Path) -> crate::Result<()> {
    let current = match ctx.env.iter().find(|(name, _)| name == "PATH") {
        Some((_, value)) => OsString::from(value),
        None => env::var_os("PATH").unwrap_or_default(),
    };
    if env::split_paths(&current).any(|p| p == dir) {
        return Ok(());
    }

    let joined = env::join_paths(std::iter::once(dir.to_path_buf()).chain(env::split_paths(&current)))
        .wrap_err_with(|| format!("can't add {} to PATH", dir.display()))?;
    let joined = joined
        .into_string()
        .map_err(|p| eyre!("PATH isn't valid UTF-8: {p:?}"))?;

    ctx.env.retain(|(name, _)| name != "PATH");
    ctx.env.push(("PATH".to_string(), joined));
    Ok(())
}

/// Makes sure rustup is installed and its shell setup is written.
///
/// A rustup already on `PATH` outside `$CARGO_HOME` (e.g. Homebrew's) is used
/// as is, with no shell setup: whatever installed it set that up. Otherwise
/// dev installs rustup into `$CARGO_HOME` (`~/.cargo`) with no default
/// toolchain, and copies rustup's own `$CARGO_HOME/env` into the shell setup.
/// rustup keeps itself current when it installs toolchains.
///
/// The `with_*` methods replace the real `PATH`, `$CARGO_HOME` and
/// installer, so tests can run without the network.
pub struct Rustup {
    search_path: Option<OsString>,
    cargo_home: Option<PathBuf>,
    shell_dir: Option<PathBuf>,
    install_url: String,
}

impl Default for Rustup {
    fn default() -> Self {
        Self {
            search_path: None,
            cargo_home: None,
            shell_dir: None,
            install_url: INSTALL_URL.into(),
        }
    }
}

/// Which rustup to use, and who set it up.
enum Found {
    /// Not installed. dev will install it into `$CARGO_HOME`.
    Missing,
    /// In `$CARGO_HOME/bin`, where the installer puts it.
    Managed,
    /// Installed by something else, which set it up.
    External,
}

impl Rustup {
    /// Looks for an existing rustup in `path` (`PATH` syntax) instead of
    /// `$PATH`.
    pub fn with_search_path(mut self, path: impl Into<OsString>) -> Self {
        self.search_path = Some(path.into());
        self
    }

    /// Uses `dir` as `$CARGO_HOME` instead of the default.
    pub fn with_cargo_home(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cargo_home = Some(dir.into());
        self
    }

    /// Writes rustup's shell setup into `dir` instead of the default.
    pub fn with_shell_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.shell_dir = Some(dir.into());
        self
    }

    /// Fetches the installer script from `url` instead of sh.rustup.rs. Any
    /// URL curl understands works, including `file://`.
    pub fn with_install_url(mut self, url: impl Into<String>) -> Self {
        self.install_url = url.into();
        self
    }

    fn cargo_home(&self) -> crate::Result<PathBuf> {
        match &self.cargo_home {
            Some(dir) => Ok(dir.clone()),
            None => default_cargo_home(),
        }
    }

    fn locate(&self) -> crate::Result<Found> {
        let managed = self.cargo_home()?.join("bin/rustup");
        let search = match &self.search_path {
            Some(path) => path.clone(),
            None => env::var_os("PATH").unwrap_or_default(),
        };

        Ok(match find_on_path(&search, "rustup") {
            Some(path) if path == managed => Found::Managed,
            Some(_) => Found::External,
            None if is_executable(&managed) => Found::Managed,
            None => Found::Missing,
        })
    }

    fn shell_dir(&self) -> crate::Result<ShellDir> {
        match &self.shell_dir {
            Some(dir) => Ok(ShellDir::new(dir)),
            None => ShellDir::user_default(),
        }
    }

    /// rustup's own env script, which puts `$CARGO_HOME/bin` on `PATH`.
    fn env_script(&self) -> crate::Result<String> {
        let path = self.cargo_home()?.join("env");
        fs::read_to_string(&path).wrap_err_with(|| format!("failed to read rustup's env script at {}", path.display()))
    }

    /// Runs the rustup installer into `$CARGO_HOME`, with no default
    /// toolchain (the entries say which) and without touching rc files.
    fn install(&self, ctx: &mut Context) -> crate::Result<()> {
        let cargo_home = self.cargo_home()?;
        // pipefail matters: without it, a failed curl pipes nothing into sh,
        // which then exits 0 and the install looks like it succeeded.
        let mut cmd = ctx.command("bash");
        cmd.args([
            "-c",
            r#"set -o pipefail; curl -fsSL "$1" | sh -s -- -y --no-modify-path --default-toolchain none"#,
            "bash",
            &self.install_url,
        ])
        .env("CARGO_HOME", &cargo_home);

        process::run(ctx, &mut cmd).wrap_err_with(|| format!("rustup installer ({}) failed", self.install_url))?;

        let rustup = cargo_home.join("bin/rustup");
        if !is_executable(&rustup) {
            bail!(
                "the rustup installer ({}) finished, but there's no rustup at {}",
                self.install_url,
                rustup.display()
            );
        }
        Ok(())
    }
}

impl Task for Rustup {
    fn name(&self) -> &str {
        "Install rustup"
    }

    /// Installs rustup when it's missing, then writes the shell setup for a
    /// rustup in `$CARGO_HOME`.
    fn apply(&self, ctx: &mut Context) -> crate::Result<()> {
        match self.locate()? {
            Found::External => return Ok(()),
            Found::Missing => self.install(ctx)?,
            Found::Managed => {}
        }
        self.shell_dir()?.write(SHELL_FILE, &self.env_script()?)
    }

    /// Returns true if rustup is installed and, when it's in `$CARGO_HOME`,
    /// the shell setup matches its env script.
    fn is_applied(&self, _ctx: &mut Context) -> crate::Result<bool> {
        match self.locate()? {
            Found::Missing => Ok(false),
            Found::External => Ok(true),
            Found::Managed => {
                let setup = self.shell_dir()?.file(SHELL_FILE);
                match fs::read_to_string(&setup) {
                    Ok(contents) => Ok(contents == self.env_script()?),
                    Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
                    Err(e) => Err(e).wrap_err_with(|| format!("failed to read {}", setup.display())),
                }
            }
        }
    }
}

/// Installs one [`Entry`] with rustup.
pub struct Toolchain {
    entry: Entry,
    name: String,
    rustup: PathBuf,
    dir: Option<PathBuf>,
}

impl Toolchain {
    pub fn new(entry: Entry) -> Self {
        let name = match &entry {
            Entry::File => format!("Installing the toolchain in {TOOLCHAIN_FILE}"),
            Entry::Named { name, .. } => format!("Installing Rust {name}"),
        };
        Self {
            entry,
            name,
            rustup: PathBuf::from("rustup"),
            dir: None,
        }
    }

    /// Runs `rustup` from `path` instead of `PATH`.
    pub fn with_rustup(mut self, path: impl Into<PathBuf>) -> Self {
        self.rustup = path.into();
        self
    }

    /// Reads `rust-toolchain.toml` from, and runs rustup in, `dir` instead
    /// of the current dir.
    pub fn with_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.dir = Some(dir.into());
        self
    }

    fn dir(&self) -> crate::Result<PathBuf> {
        match &self.dir {
            Some(dir) => Ok(dir.clone()),
            None => env::current_dir().wrap_err("failed to read the current directory"),
        }
    }

    fn rustup(&self, ctx: &Context, args: &[&str]) -> crate::Result<String> {
        let mut cmd = ctx.command(&self.rustup);
        cmd.args(args).current_dir(self.dir()?);
        process::output(&mut cmd, &format!("rustup {}", args.join(" ")))
    }

    /// What has to be installed: the toolchain, its components, its targets.
    fn wanted(&self) -> crate::Result<Spec> {
        match &self.entry {
            Entry::Named { name, components } => Ok(Spec {
                channel: name.clone(),
                components: components.clone(),
                targets: Vec::new(),
            }),
            Entry::File => {
                let path = self.dir()?.join(TOOLCHAIN_FILE);
                let toml = fs::read_to_string(&path).wrap_err_with(|| format!("failed to read {}", path.display()))?;
                Spec::parse(&toml).wrap_err_with(|| format!("can't tell which toolchain {} names", path.display()))
            }
        }
    }
}

impl Task for Toolchain {
    fn name(&self) -> &str {
        &self.name
    }

    fn apply(&self, ctx: &mut Context) -> crate::Result<()> {
        let mut cmd = ctx.command(&self.rustup);
        cmd.current_dir(self.dir()?).args(["toolchain", "install"]);
        if let Entry::Named { name, components } = &self.entry {
            cmd.args([name, "--profile", "minimal"]);
            for component in components {
                cmd.args(["--component", component]);
            }
        }
        // With no name, rustup installs what rust-toolchain.toml says.
        process::run(ctx, &mut cmd)
    }

    /// Returns true if the toolchain, its components, and its targets are
    /// installed. A moving channel like `nightly` isn't checked for updates.
    fn is_applied(&self, ctx: &mut Context) -> crate::Result<bool> {
        let spec = self.wanted()?;

        let toolchains = self.rustup(ctx, &["toolchain", "list"])?;
        if !toolchains.lines().any(|line| names(line, &spec.channel)) {
            return Ok(false);
        }

        if !spec.components.is_empty() {
            let installed = self.rustup(ctx, &["component", "list", "--installed", "--toolchain", &spec.channel])?;
            if !spec
                .components
                .iter()
                .all(|c| installed.lines().any(|line| names(line, c)))
            {
                return Ok(false);
            }
        }

        if !spec.targets.is_empty() {
            let installed = self.rustup(ctx, &["target", "list", "--installed", "--toolchain", &spec.channel])?;
            if !spec
                .targets
                .iter()
                .all(|t| installed.lines().any(|line| line.trim() == t))
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// True when a line of rustup's output (`nightly-aarch64-apple-darwin
/// (default)`, `rustfmt-aarch64-apple-darwin`) is for `name`, ignoring the
/// host triple and any notes after it.
fn names(line: &str, name: &str) -> bool {
    let item = line.split_whitespace().next().unwrap_or_default();
    item == name || item.strip_prefix(name).is_some_and(|rest| rest.starts_with('-'))
}

/// A toolchain and what it needs, as rust-toolchain.toml says.
#[derive(Debug, PartialEq)]
pub struct Spec {
    pub channel: String,
    pub components: Vec<String>,
    pub targets: Vec<String>,
}

impl Spec {
    /// Reads `channel`, `components` and `targets` from the `[toolchain]`
    /// table of a rust-toolchain.toml. Arrays may span lines. Anything else
    /// in the file is ignored.
    pub fn parse(toml: &str) -> crate::Result<Self> {
        let channel = toml_value(toml, "channel")
            .and_then(|v| quoted(&v).into_iter().next())
            .ok_or_else(|| eyre!("it has no `channel = \"...\"`"))?;
        Ok(Self {
            channel,
            components: toml_value(toml, "components").map(|v| quoted(&v)).unwrap_or_default(),
            targets: toml_value(toml, "targets").map(|v| quoted(&v)).unwrap_or_default(),
        })
    }
}

/// The raw value of `key = ...` in `toml`, through the closing `]` when it's
/// an array that spans lines. Comments after `#` are dropped.
fn toml_value(toml: &str, key: &str) -> Option<String> {
    let mut lines = toml.lines().map(|line| line.split('#').next().unwrap_or_default());
    let value = lines.find_map(|line| {
        let (k, v) = line.split_once('=')?;
        (k.trim() == key).then(|| v.trim().to_string())
    })?;

    if !value.starts_with('[') || value.contains(']') {
        return Some(value);
    }
    let mut value = value;
    for line in lines {
        value.push_str(line);
        if line.contains(']') {
            break;
        }
    }
    Some(value)
}

/// The double-quoted strings in `value`, e.g. `["a", "b"]` -> `a`, `b`.
fn quoted(value: &str) -> Vec<String> {
    value.split('"').skip(1).step_by(2).map(str::to_string).collect()
}

/// `$CARGO_HOME`, or `~/.cargo`.
fn default_cargo_home() -> crate::Result<PathBuf> {
    match env::var_os("CARGO_HOME").filter(|h| !h.is_empty()) {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => Ok(home()?.join(".cargo")),
    }
}

fn find_on_path(path: &OsStr, name: &str) -> Option<PathBuf> {
    env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
