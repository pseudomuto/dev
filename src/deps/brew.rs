#[cfg(test)]
mod tests;

use std::{
    env,
    fs,
    path::{Path, PathBuf},
};

use color_eyre::eyre::{WrapErr, bail, eyre};

use crate::{
    deps::{Dependency, Example, Task},
    process,
    shell::{ShellDir, data_home, home},
};

const INSTALL_URL: &str = "https://zerobrew.rs/install";
/// zerobrew's file in the [`ShellDir`]. Must match `share/shell`.
const SHELL_FILE: &str = "zerobrew";
const LATEST_RELEASE_URL: &str = "https://github.com/zerobrewhq/zerobrew/releases/latest";

/// Installs zerobrew and a set of brew packages.
pub struct Brew {
    tasks: Vec<Box<dyn Task>>,
}

impl Brew {
    /// Creates a dependency that installs zerobrew, then each of `packages`
    /// (e.g. `["jq", "yq"]`) in order.
    pub fn new(packages: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let mut tasks: Vec<Box<dyn Task>> = vec![Box::new(ZeroBrew::default())];
        tasks.extend(packages.into_iter().map(|p| Box::new(Package::new(p)) as Box<dyn Task>));
        Self::with_tasks(tasks)
    }

    /// Like [`Brew::new`], but with an explicit list of tasks to run, in order,
    /// instead of the default (zerobrew).
    pub fn with_tasks(tasks: Vec<Box<dyn Task>>) -> Self {
        Self { tasks }
    }
}

impl Dependency for Brew {
    fn name(&self) -> &str {
        "Install brew dependencies"
    }

    fn desc(&self) -> &str {
        "Install/update brew and brew packages"
    }

    fn tasks(&self) -> &[Box<dyn Task>] {
        &self.tasks
    }

    fn examples(&self) -> Vec<super::Example> {
        vec![Example::new(
            "Require packages",
            "deps:
              brew:
                - eza
                - jq
                ",
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

/// Looks up the latest released zerobrew version, e.g. `"0.4.2"`.
type LatestVersionFn = Box<dyn Fn(&crate::Context) -> crate::Result<String>>;

/// Installs or upgrades zerobrew (`zb`) itself, not any packages.
///
/// The defaults talk to the real zerobrew installer and GitHub. The `with_*`
/// methods replace those, so tests can run without the network.
pub struct ZeroBrew {
    /// Where `zb` lives. `None` means where the installer puts it:
    /// `$ZEROBREW_BIN/zb` or `~/.local/bin/zb`, resolved when needed.
    zb: Option<PathBuf>,
    /// Where to write zerobrew's shell setup, as `<dir>/zerobrew`. `None`
    /// means [`ShellDir::user_default`], resolved when needed.
    shell_dir: Option<PathBuf>,
    install_url: String,
    latest_version: LatestVersionFn,
}

impl Default for ZeroBrew {
    fn default() -> Self {
        Self {
            zb: None,
            shell_dir: None,
            install_url: INSTALL_URL.into(),
            latest_version: Box::new(latest_version),
        }
    }
}

impl ZeroBrew {
    /// Looks for `zb` at `path` instead of where the installer puts it.
    pub fn with_zb(mut self, path: impl Into<PathBuf>) -> Self {
        self.zb = Some(path.into());
        self
    }

    /// Writes zerobrew's shell setup into `dir` instead of the default.
    pub fn with_shell_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.shell_dir = Some(dir.into());
        self
    }

    /// Fetches the installer script from `url` instead of zerobrew.rs. Any URL
    /// curl understands works, including `file://`.
    pub fn with_install_url(mut self, url: impl Into<String>) -> Self {
        self.install_url = url.into();
        self
    }

    /// Uses `f` to find the latest version instead of asking GitHub.
    pub fn with_latest_version(mut self, f: impl Fn(&crate::Context) -> crate::Result<String> + 'static) -> Self {
        self.latest_version = Box::new(f);
        self
    }

    fn zb_path(&self) -> crate::Result<PathBuf> {
        match &self.zb {
            Some(path) => Ok(path.clone()),
            None => default_zb_path(),
        }
    }

    /// Writes zerobrew's own shell setup (the block `zb init` adds to shell
    /// config) to the shell file, so it can't drift from zerobrew's.
    ///
    /// `zb init` only writes that block into an rc file, so it runs with a
    /// throwaway `$HOME` and `SHELL=/bin/sh`, which makes it write
    /// `$HOME/.profile`. The real rc files are never touched. zb works out its
    /// paths from `$HOME` too, so the env vars it would otherwise derive from
    /// it are set from the real home first.
    fn write_shell_file(&self, ctx: &crate::Context) -> crate::Result<()> {
        let zb = self.zb_path()?;
        let zb_bin = zb
            .parent()
            .ok_or_else(|| eyre!("expected {} to be a file in a directory", zb.display()))?;
        let fake_home = tempfile::tempdir().wrap_err("failed to create a temp dir for `zb init`")?;

        let mut cmd = ctx.command(&zb);
        cmd.arg("init")
            .env("HOME", fake_home.path())
            .env("SHELL", "/bin/sh")
            .env("ZEROBREW_BIN", zb_bin)
            .env("XDG_DATA_HOME", data_home()?)
            .env_remove("ZDOTDIR");
        if env::var_os("ZEROBREW_DIR").is_none_or(|d| d.is_empty()) {
            cmd.env("ZEROBREW_DIR", home()?.join(".zerobrew"));
        }
        // Its output names the throwaway .profile, so only show it on failure.
        process::output(&mut cmd, &format!("{} init", zb.display()))?;

        let profile = fake_home.path().join(".profile");
        let config =
            fs::read_to_string(&profile).wrap_err_with(|| format!("`zb init` didn't write {}", profile.display()))?;
        let block = zerobrew_block(&config).ok_or_else(|| {
            eyre!("`zb init` wrote no `{ZB_BLOCK_START}` block; zerobrew may have changed how it sets up shells")
        })?;

        self.shell_dir()?.write(SHELL_FILE, &block)
    }

    fn shell_dir(&self) -> crate::Result<ShellDir> {
        match &self.shell_dir {
            Some(dir) => Ok(ShellDir::new(dir)),
            None => ShellDir::user_default(),
        }
    }
}

impl Task for ZeroBrew {
    fn name(&self) -> &str {
        "Install zerobrew"
    }

    /// Installs or upgrades `zb` when it isn't current, then writes its shell
    /// setup.
    ///
    /// The installer's output is copied to `ctx.out` and `ctx.err` as it runs.
    /// stdin stays attached to the terminal so sudo can prompt for a password.
    fn apply(&self, ctx: &mut crate::Context) -> crate::Result<()> {
        // When only the shell file is missing, zb itself is fine.
        if !self.is_current(ctx)? {
            self.install(ctx)?;
        }

        self.write_shell_file(ctx)
    }

    /// Returns true if `zb` is installed, its shell setup has been written,
    /// and it matches the latest release.
    fn is_applied(&self, ctx: &mut crate::Context) -> crate::Result<bool> {
        if !self.shell_dir()?.file(SHELL_FILE).exists() {
            return Ok(false);
        }

        self.is_current(ctx)
    }
}

impl ZeroBrew {
    /// Whether `zb` is installed and matches the latest release. Only asks
    /// for the latest release when `zb` is installed.
    fn is_current(&self, ctx: &crate::Context) -> crate::Result<bool> {
        let zb = self.zb_path()?;
        if !zb.exists() {
            return Ok(false);
        }

        Ok(installed_version(ctx, &zb)? == (self.latest_version)(ctx)?)
    }

    /// Runs the zerobrew installer, which installs or upgrades `zb`.
    fn install(&self, ctx: &mut crate::Context) -> crate::Result<()> {
        // pipefail matters: without it, a failed curl pipes nothing into bash,
        // which then exits 0 and the install looks like it succeeded.
        let mut cmd = ctx.command("bash");
        cmd.args([
            "-c",
            r#"set -o pipefail; curl -fsSL "$1" | bash -s -- --no-modify-path"#,
            "bash",
            &self.install_url,
        ]);

        process::run(ctx, &mut cmd).wrap_err_with(|| format!("zerobrew installer ({}) failed", self.install_url))
    }
}

/// Installs a package with zerobrew, or upgrades it when it's outdated.
pub struct Package {
    /// The formula to install, e.g. `jq`.
    formula: String,
    /// Shown as the task name, e.g. `Installing jq`.
    label: String,
    /// Where `zb` lives. `None` means where the installer puts it (see
    /// [`ZeroBrew`]), resolved when needed.
    zb: Option<PathBuf>,
}

impl Package {
    pub fn new(formula: impl Into<String>) -> Self {
        let formula = formula.into();
        Self {
            label: format!("Installing {formula}"),
            formula,
            zb: None,
        }
    }

    /// Uses the `zb` at `path` instead of where the installer puts it.
    pub fn with_zb(mut self, path: impl Into<PathBuf>) -> Self {
        self.zb = Some(path.into());
        self
    }

    fn zb_path(&self) -> crate::Result<PathBuf> {
        match &self.zb {
            Some(path) => Ok(path.clone()),
            None => default_zb_path(),
        }
    }

    /// Whether `zb list` includes this package.
    fn is_installed(&self, ctx: &crate::Context, zb: &Path) -> crate::Result<bool> {
        let listed = zb_output(ctx, zb, &["list"])?;
        Ok(listed_names(&listed).any(|n| n == self.formula))
    }
}

impl Task for Package {
    fn name(&self) -> &str {
        &self.label
    }

    /// Installs the package if it's missing, otherwise upgrades it.
    fn apply(&self, ctx: &mut crate::Context) -> crate::Result<()> {
        let zb = self.zb_path()?;
        let action = if self.is_installed(ctx, &zb)? {
            "upgrade"
        } else {
            "install"
        };

        let mut cmd = ctx.command(&zb);
        process::run(ctx, cmd.args([action, &self.formula]))
            .wrap_err_with(|| format!("`zb {action} {}` failed", self.formula))
    }

    /// Returns true if the package is installed and not outdated.
    fn is_applied(&self, ctx: &mut crate::Context) -> crate::Result<bool> {
        let zb = self.zb_path()?;
        if !self.is_installed(ctx, &zb)? {
            return Ok(false);
        }

        let outdated = zb_output(ctx, &zb, &["outdated", "--quiet"])?;
        Ok(!listed_names(&outdated).any(|n| n == self.formula))
    }
}

/// Runs `zb <args>` and returns its stdout, failing on a non-zero exit.
fn zb_output(ctx: &crate::Context, zb: &Path, args: &[&str]) -> crate::Result<String> {
    let display = format!("{} {}", zb.display(), args.join(" "));
    process::output(ctx.command(zb).args(args), &display)
}

/// Marks the shell setup `zb init` manages in an rc file.
const ZB_BLOCK_START: &str = "# >>> zerobrew >>>";
const ZB_BLOCK_END: &str = "# <<< zerobrew <<<";

/// The zerobrew block in `config`, markers included, ending in a newline.
fn zerobrew_block(config: &str) -> Option<String> {
    let start = config.find(ZB_BLOCK_START)?;
    let len = config[start..].find(ZB_BLOCK_END)? + ZB_BLOCK_END.len();
    Some(format!("{}\n", &config[start..start + len]))
}

/// The package name from each line of `zb list` (`jq 1.7.1`) or
/// `zb outdated --quiet` (`jq`) output.
fn listed_names(output: &str) -> impl Iterator<Item = &str> {
    output.lines().filter_map(|line| line.split_whitespace().next())
}

/// Where the installer puts `zb`: `$ZEROBREW_BIN/zb`, or `~/.local/bin/zb`.
fn default_zb_path() -> crate::Result<PathBuf> {
    // The installer treats an empty ZEROBREW_BIN as unset, so do the same.
    if let Some(bin) = env::var_os("ZEROBREW_BIN").filter(|b| !b.is_empty()) {
        return Ok(PathBuf::from(bin).join("zb"));
    }

    let home = env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .ok_or_else(|| eyre!("HOME is not set, so zb can't be found in ~/.local/bin"))?;
    Ok(PathBuf::from(home).join(".local/bin/zb"))
}

fn installed_version(ctx: &crate::Context, zb: &Path) -> crate::Result<String> {
    let output = ctx
        .command(zb)
        .arg("--version")
        .output()
        .wrap_err_with(|| format!("failed to run {} --version", zb.display()))?;

    if !output.status.success() {
        bail!("{} --version failed with {}", zb.display(), output.status);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_version(&stdout).map(str::to_owned).ok_or_else(|| {
        eyre!(
            "expected `zb <version>` from {} --version, got {stdout:?}",
            zb.display()
        )
    })
}

/// Asks GitHub where `releases/latest` redirects to and reads the tag from it.
fn latest_version(ctx: &crate::Context) -> crate::Result<String> {
    let output = ctx
        .command("curl")
        .args(["-sS", "-o", "/dev/null", "-w", "%{redirect_url}", LATEST_RELEASE_URL])
        .output()
        .wrap_err("failed to run curl to check the latest zerobrew release")?;

    if !output.status.success() {
        bail!(
            "curl {LATEST_RELEASE_URL} failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let url = String::from_utf8_lossy(&output.stdout);
    parse_release_tag(&url)
        .map(str::to_owned)
        .ok_or_else(|| eyre!("expected {LATEST_RELEASE_URL} to redirect to a release tag, got {url:?}"))
}

/// Extracts the version from `zb --version` output, e.g. `zb 0.4.2` -> `0.4.2`.
fn parse_version(output: &str) -> Option<&str> {
    output
        .trim()
        .strip_prefix("zb ")
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

/// Extracts the version from a GitHub release redirect URL,
/// e.g. `.../releases/tag/v0.4.2` -> `0.4.2`.
fn parse_release_tag(url: &str) -> Option<&str> {
    let (_, tag) = url.trim().rsplit_once("/tag/")?;
    let version = tag.strip_prefix('v').unwrap_or(tag);
    (!version.is_empty()).then_some(version)
}
