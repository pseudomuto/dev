//! Installs the project's own packages (node_modules, gems, Go modules, and
//! so on) with each ecosystem's package manager.
//!
//! The ecosystems are found from known files in the project root, and run
//! last, after the tools that install them. `packages` in dev.yaml can turn
//! this off or pick the ecosystems and where they run:
//!
//! ```yaml
//! deps:
//!   packages: false          # don't install packages
//!   packages: [node, ruby]   # only these, at this spot
//! ```
//!
//! An install is current when its fingerprint matches: a hash of the
//! manifest, lockfile, and runtime version, kept in
//! `<root>/.dev/packages/<ecosystem>`.
//!
//! In CI, installs use the lockfile as is (e.g. `npm ci`), failing instead of
//! updating it.

use std::{
    env,
    fmt,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use color_eyre::eyre::{WrapErr, bail};
use serde::Deserialize;

use crate::{
    Context,
    deps::{Dependency, Example, Task},
    process,
};

/// Where fingerprints go, under the project root.
const STATE_DIR: &str = ".dev/packages";

/// A kind of project that has packages to install.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Eco {
    Node,
    Ruby,
    Go,
    Rust,
    Python,
}

impl Eco {
    /// Every ecosystem, in the order they're installed when found.
    pub const ALL: [Eco; 5] = [Eco::Node, Eco::Ruby, Eco::Go, Eco::Rust, Eco::Python];

    /// The name used in dev.yaml and for its fingerprint file.
    pub fn key(self) -> &'static str {
        match self {
            Eco::Node => "node",
            Eco::Ruby => "ruby",
            Eco::Go => "go",
            Eco::Rust => "rust",
            Eco::Python => "python",
        }
    }
}

impl fmt::Display for Eco {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

/// Whether installs may update lockfiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Plain installs, which update the lockfile when the manifest changed.
    Local,
    /// Installs that use the lockfile as is, and fail if it's out of date.
    Ci,
}

impl Mode {
    /// [`Mode::Ci`] when `CI` is set to anything but empty, `0`, or `false`,
    /// as CI services do.
    pub fn from_env() -> Self {
        match env::var("CI") {
            Ok(v) if !matches!(v.as_str(), "" | "0" | "false") => Mode::Ci,
            _ => Mode::Local,
        }
    }
}

/// Installs the packages of each ecosystem found in the project.
pub struct Packages {
    tasks: Vec<Box<dyn Task>>,
}

impl Packages {
    /// One install for each ecosystem in [`Eco::ALL`] whose files are in
    /// `root`.
    pub fn detect(root: impl AsRef<Path>, mode: Mode) -> Self {
        let root = root.as_ref();
        let tasks = Eco::ALL
            .into_iter()
            .filter_map(|eco| Install::detect(root, eco, mode))
            .map(|install| Box::new(install) as Box<dyn Task>)
            .collect();
        Self::with_tasks(tasks)
    }

    /// One install for each of `ecos`, in that order. Fails naming the first
    /// one whose files aren't in `root`, since asking for it means its files
    /// are expected.
    pub fn only(root: impl AsRef<Path>, ecos: &[Eco], mode: Mode) -> crate::Result<Self> {
        let root = root.as_ref();
        let mut tasks: Vec<Box<dyn Task>> = Vec::new();
        for &eco in ecos {
            let Some(install) = Install::detect(root, eco, mode) else {
                color_eyre::eyre::bail!(
                    "`packages` lists `{eco}`, but {} has none of its files ({})",
                    root.display(),
                    manifests(eco).join(", ")
                );
            };
            tasks.push(Box::new(install));
        }
        Ok(Self::with_tasks(tasks))
    }

    /// Runs exactly `tasks`, in order.
    pub fn with_tasks(tasks: Vec<Box<dyn Task>>) -> Self {
        Self { tasks }
    }
}

impl Dependency for Packages {
    fn name(&self) -> &str {
        "Install project packages"
    }

    fn desc(&self) -> &str {
        "Install the project's packages (npm, bundler, go, cargo, uv, poetry) when they're out of date"
    }

    fn examples(&self) -> Vec<Example> {
        vec![
            Example::new("Only some ecosystems", "deps:\n  packages: [node, ruby]"),
            Example::new("Turn it off", "deps:\n  packages: false"),
        ]
    }

    fn tasks(&self) -> &[Box<dyn Task>] {
        &self.tasks
    }

    fn meet(&self, ctx: &mut Context) -> crate::Result<()> {
        super::meet_tasks(&self.tasks, ctx)
    }

    fn is_met(&self, ctx: &mut Context) -> crate::Result<bool> {
        super::tasks_met(&self.tasks, ctx)
    }

    /// Leaves installed packages alone.
    fn down(&self, _ctx: &mut Context) -> crate::Result<()> {
        Ok(())
    }
}

/// The files whose presence means `eco` is in use, for errors.
fn manifests(eco: Eco) -> &'static [&'static str] {
    match eco {
        Eco::Node => &["package.json"],
        Eco::Ruby => &["Gemfile"],
        Eco::Go => &["go.mod"],
        Eco::Rust => &["Cargo.toml"],
        Eco::Python => &["uv.lock", "poetry.lock"],
    }
}

/// How one package manager installs, and what goes into its fingerprint.
struct Spec {
    tool: &'static str,
    /// The install command.
    command: &'static [&'static str],
    /// The install command in CI, which uses the lockfile as is.
    ci_command: &'static [&'static str],
    /// Env for `ci_command`.
    ci_env: &'static [(&'static str, &'static str)],
    /// Prints the runtime's version, which is part of the fingerprint.
    version: &'static [&'static str],
    /// Files, relative to the root, that are part of the fingerprint. Missing
    /// ones count too, as missing.
    inputs: &'static [&'static str],
    /// What the install creates in the root, when it creates anything there.
    dir: Option<&'static str>,
}

const NPM: Spec = Spec {
    tool: "npm",
    command: &["npm", "install"],
    ci_command: &["npm", "ci"],
    ci_env: &[],
    version: &["node", "--version"],
    inputs: &["package.json", "package-lock.json"],
    dir: Some("node_modules"),
};

const YARN: Spec = Spec {
    tool: "yarn",
    command: &["yarn", "install"],
    // Yarn 2+ takes this as `--immutable`.
    ci_command: &["yarn", "install", "--frozen-lockfile"],
    ci_env: &[],
    version: &["node", "--version"],
    inputs: &["package.json", "yarn.lock"],
    dir: Some("node_modules"),
};

const PNPM: Spec = Spec {
    tool: "pnpm",
    command: &["pnpm", "install"],
    ci_command: &["pnpm", "install", "--frozen-lockfile"],
    ci_env: &[],
    version: &["node", "--version"],
    inputs: &["package.json", "pnpm-lock.yaml"],
    dir: Some("node_modules"),
};

const BUN: Spec = Spec {
    tool: "bun",
    command: &["bun", "install"],
    ci_command: &["bun", "install", "--frozen-lockfile"],
    ci_env: &[],
    version: &["bun", "--version"],
    inputs: &["package.json", "bun.lock", "bun.lockb"],
    dir: Some("node_modules"),
};

const BUNDLER: Spec = Spec {
    tool: "bundler",
    command: &["bundle", "install"],
    // `--frozen` is deprecated in favor of this.
    ci_command: &["bundle", "install"],
    ci_env: &[("BUNDLE_FROZEN", "true")],
    version: &["ruby", "--version"],
    inputs: &["Gemfile", "Gemfile.lock"],
    dir: None,
};

const GO: Spec = Spec {
    tool: "go",
    command: &["go", "mod", "download"],
    ci_command: &["go", "mod", "download"],
    ci_env: &[],
    version: &["go", "version"],
    inputs: &["go.mod", "go.sum"],
    dir: None,
};

const CARGO: Spec = Spec {
    tool: "cargo",
    command: &["cargo", "fetch"],
    ci_command: &["cargo", "fetch", "--locked"],
    ci_env: &[],
    version: &["cargo", "--version"],
    inputs: &["Cargo.toml", "Cargo.lock"],
    dir: None,
};

const UV: Spec = Spec {
    tool: "uv",
    command: &["uv", "sync"],
    ci_command: &["uv", "sync", "--locked"],
    ci_env: &[],
    version: &["uv", "--version"],
    inputs: &["pyproject.toml", "uv.lock", ".python-version"],
    dir: Some(".venv"),
};

// poetry keeps its venv outside the project unless configured otherwise, so
// there's no dir to check. `poetry install` never updates the lockfile.
const POETRY: Spec = Spec {
    tool: "poetry",
    command: &["poetry", "install"],
    ci_command: &["poetry", "install"],
    ci_env: &[],
    version: &["poetry", "--version"],
    inputs: &["pyproject.toml", "poetry.lock", ".python-version"],
    dir: None,
};

/// Installs one ecosystem's packages, when its fingerprint is stale.
pub struct Install {
    eco: Eco,
    name: String,
    root: PathBuf,
    spec: &'static Spec,
    mode: Mode,
}

impl Install {
    /// The install for `eco` in `root`, if its files are there.
    pub fn detect(root: &Path, eco: Eco, mode: Mode) -> Option<Self> {
        let has = |file: &str| root.join(file).is_file();
        let spec = match eco {
            Eco::Node if !has("package.json") => return None,
            Eco::Node if has("pnpm-lock.yaml") => &PNPM,
            Eco::Node if has("yarn.lock") => &YARN,
            Eco::Node if has("bun.lock") || has("bun.lockb") => &BUN,
            Eco::Node => &NPM,
            Eco::Ruby if has("Gemfile") => &BUNDLER,
            Eco::Go if has("go.mod") => &GO,
            Eco::Rust if has("Cargo.toml") => &CARGO,
            Eco::Python if has("uv.lock") => &UV,
            Eco::Python if has("poetry.lock") => &POETRY,
            _ => return None,
        };

        Some(Self {
            eco,
            name: format!("Installing {eco} packages ({})", spec.tool),
            root: root.to_path_buf(),
            spec,
            mode,
        })
    }

    fn state_file(&self) -> PathBuf {
        self.root.join(STATE_DIR).join(self.eco.key())
    }

    /// A hash of the inputs and the runtime's version, as hex.
    /// A hash of each input file, in order. Missing files hash as missing.
    fn input_hashes(&self) -> crate::Result<Vec<u64>> {
        self.spec
            .inputs
            .iter()
            .map(|name| {
                let path = self.root.join(name);
                let mut hash = Fnv::new();
                hash.write(name.as_bytes());
                match fs::read(&path) {
                    Ok(contents) => {
                        hash.write(&[1]);
                        hash.write(&contents);
                    }
                    Err(e) if e.kind() == ErrorKind::NotFound => hash.write(&[0]),
                    Err(e) => return Err(e).wrap_err_with(|| format!("failed to read {}", path.display())),
                }
                Ok(hash.finish())
            })
            .collect()
    }

    fn fingerprint(&self, ctx: &Context) -> crate::Result<String> {
        let mut hash = Fnv::new();
        for input in self.input_hashes()? {
            hash.write(&input.to_le_bytes());
        }

        let version = self.spec.version;
        let display = version.join(" ");
        let mut cmd = ctx.command(version[0]);
        cmd.args(&version[1..]).current_dir(&self.root);
        let version = process::output(&mut cmd, &display).wrap_err_with(|| {
            format!(
                "can't tell which {} {} uses; add it to deps, or set `packages: false` to skip installing packages",
                version[0], self.eco
            )
        })?;
        hash.write(version.trim().as_bytes());

        Ok(format!("{:016x}", hash.finish()))
    }
}

impl Task for Install {
    fn name(&self) -> &str {
        &self.name
    }

    fn apply(&self, ctx: &mut Context) -> crate::Result<()> {
        let (command, env) = match self.mode {
            Mode::Local => (self.spec.command, &[][..]),
            Mode::Ci => (self.spec.ci_command, self.spec.ci_env),
        };
        let before = self.input_hashes()?;
        let mut cmd = ctx.command(command[0]);
        cmd.args(&command[1..])
            .envs(env.iter().copied())
            .current_dir(&self.root);
        process::run(ctx, &mut cmd)?;

        // In CI, the committed files are the truth. An install that changes
        // them (e.g. `go mod download` filling in go.sum) means they weren't
        // committed up to date, which a plain install would quietly paper over.
        if self.mode == Mode::Ci {
            let changed: Vec<&str> = self
                .spec
                .inputs
                .iter()
                .zip(before.iter().zip(self.input_hashes()?))
                .filter(|(_, (before, after))| **before != *after)
                .map(|(name, _)| *name)
                .collect();
            if !changed.is_empty() {
                bail!(
                    "`{}` changed {} in CI; run `dev up` locally and commit the result",
                    command.join(" "),
                    changed.join(", ")
                );
            }
        }

        // After the install, since it can rewrite the lockfile.
        let fingerprint = self.fingerprint(ctx)?;
        let path = self.state_file();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).wrap_err_with(|| format!("failed to create {}", dir.display()))?;
        }
        fs::write(&path, fingerprint).wrap_err_with(|| format!("failed to write {}", path.display()))
    }

    /// Returns true if the stored fingerprint matches, and what the install
    /// creates is still there.
    fn is_applied(&self, ctx: &mut Context) -> crate::Result<bool> {
        if let Some(dir) = self.spec.dir
            && !self.root.join(dir).is_dir()
        {
            return Ok(false);
        }

        let path = self.state_file();
        let stored = match fs::read_to_string(&path) {
            Ok(stored) => stored,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e).wrap_err_with(|| format!("failed to read {}", path.display())),
        };
        Ok(stored.trim() == self.fingerprint(ctx)?)
    }
}

/// 64-bit FNV-1a. Unlike std's hasher, its output never changes between
/// Rust versions, so fingerprints written by one build of dev hold for the
/// next. It doesn't need to resist collisions on purpose.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}
