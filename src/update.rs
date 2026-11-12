//! Keeps `dev` up to date with its GitHub releases.
//!
//! `dev self-update` installs the latest release over the running binary,
//! using the same `install.sh` as the curl installer. The prompt hook
//! (`dev hook-env`) also starts it in the background, detached and silent, at
//! most once every [`CHECK_EVERY`]. The next `dev` command then says what
//! changed.
//!
//! State lives in `$XDG_STATE_HOME/dev` (`~/.local/state/dev`):
//!
//! - `update-check`: its mtime is when the hook last started a check.
//! - `update.log`: the output of the last background run, for when one fails.
//! - `updated`: a notice waiting to be printed by the next `dev` command.

use std::{
    fmt,
    fs::{self, File},
    io::{ErrorKind, Write},
    os::unix::process::CommandExt,
    path::PathBuf,
    process::Stdio,
    str::FromStr,
    time::Duration,
};

use color_eyre::eyre::{WrapErr, bail, eyre};

use crate::{
    Context,
    Result,
    process,
    shell::{home, state_home},
};

/// `install.sh`, run by `dev self-update` to install a release.
pub const INSTALL_SCRIPT: &str = include_str!("../install.sh");

/// Where releases are downloaded from, unless `DEV_DOWNLOAD_URL` says
/// otherwise. Must match `install.sh`.
pub const DOWNLOAD_URL: &str = "https://github.com/pseudomuto/dev/releases";

/// How often the prompt hook checks for a new release.
pub const CHECK_EVERY: Duration = Duration::from_secs(4 * 60 * 60);

/// A release version, e.g. `0.1.0`. Pre-releases aren't published, so
/// there's no `-rc1` part.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(u64, u64, u64);

impl Version {
    /// The version of this `dev`.
    pub fn current() -> Self {
        env!("CARGO_PKG_VERSION")
            .parse()
            .expect("Cargo.toml has a plain X.Y.Z version")
    }
}

impl FromStr for Version {
    type Err = color_eyre::Report;

    /// Parses `X.Y.Z`, with or without a leading `v`.
    fn from_str(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.strip_prefix('v').unwrap_or(s).split('.').collect();
        let [major, minor, patch] = parts.as_slice() else {
            bail!("expected a version like 1.2.3, got `{s}`");
        };
        let num = |part: &str| {
            part.parse()
                .map_err(|_| eyre!("expected a version like 1.2.3, got `{s}`"))
        };
        Ok(Self(num(major)?, num(minor)?, num(patch)?))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// The version in a release's `SHA256SUMS`, read from its asset names, e.g.
/// `dev-v0.2.0-aarch64-apple-darwin.tar.gz`. `None` if there isn't one.
pub fn latest_from_sums(sums: &str) -> Option<Version> {
    sums.lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .filter_map(|name| name.trim_start_matches('*').strip_prefix("dev-v"))
        .find_map(|rest| rest.split_once('-')?.0.parse().ok())
}

/// What [`Updater::update`] did.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Already on the latest release.
    UpToDate(Version),
    /// Installed `to` over `from`.
    Updated { from: Version, to: Version },
}

/// Updates the `dev` at `exe` from the releases at `download_url`.
pub struct Updater {
    pub current: Version,
    pub exe: PathBuf,
    pub download_url: String,
    /// Where the files in the [module docs](self) go.
    pub state_dir: PathBuf,
}

impl Updater {
    /// Updates the running `dev`, from `DEV_DOWNLOAD_URL` if it's set.
    pub fn for_current_exe() -> Result<Self> {
        let exe = std::env::current_exe().wrap_err("failed to find the running dev")?;
        let download_url = std::env::var("DEV_DOWNLOAD_URL")
            .ok()
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| DOWNLOAD_URL.to_string());
        Ok(Self {
            current: Version::current(),
            exe,
            download_url,
            state_dir: state_home()?.join("dev"),
        })
    }

    /// The latest release's version.
    pub fn latest(&self, ctx: &Context) -> Result<Version> {
        // GitHub serves the latest release's assets under latest/download/.
        let url = format!("{}/latest/download/SHA256SUMS", self.download_url);
        let sums = process::output(
            ctx.command("curl")
                .args(["-fsSL", "--retry", "2", "--max-time", "30", &url]),
            &format!("curl {url}"),
        )?;
        latest_from_sums(&sums).ok_or_else(|| eyre!("{url} doesn't list any dev releases"))
    }

    /// Installs the latest release over [`Updater::exe`] if it's newer.
    pub fn update(&self, ctx: &Context) -> Result<Outcome> {
        let latest = self.latest(ctx)?;
        if latest <= self.current {
            return Ok(Outcome::UpToDate(self.current));
        }

        // install.sh always installs `<dir>/dev`.
        if self.exe.file_name().is_none_or(|name| name != "dev") {
            bail!(
                "can't update {}: install.sh only replaces a binary named `dev`",
                self.exe.display()
            );
        }
        let dir = self
            .exe
            .parent()
            .ok_or_else(|| eyre!("{} has no parent dir", self.exe.display()))?;

        let mut child = ctx
            .command("sh")
            .arg("-s")
            .env("DEV_VERSION", latest.to_string())
            .env("DEV_INSTALL_DIR", dir)
            .env("DEV_DOWNLOAD_URL", &self.download_url)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .wrap_err("failed to start install.sh")?;
        child
            .stdin
            .take()
            .expect("stdin is piped")
            .write_all(INSTALL_SCRIPT.as_bytes())
            .wrap_err("failed to pass install.sh to sh")?;
        let output = child.wait_with_output().wrap_err("failed to wait for install.sh")?;
        if !output.status.success() {
            bail!(
                "installing dev {latest} failed with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }

        Ok(Outcome::Updated {
            from: self.current,
            to: latest,
        })
    }

    /// Whether the prompt hook should update this `dev` on its own, per
    /// `DEV_AUTO_UPDATE` (`auto_update`): `0` never, `1` always, otherwise
    /// only release builds that aren't managed by cargo (`~/.cargo/bin`).
    pub fn auto_update_enabled(&self, auto_update: Option<&str>) -> bool {
        match auto_update {
            Some("0") => false,
            Some("1") => true,
            _ => !cfg!(debug_assertions) && !self.in_cargo_bin(),
        }
    }

    fn in_cargo_bin(&self) -> bool {
        let cargo_home = std::env::var_os("CARGO_HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
            .or_else(|| home().ok().map(|h| h.join(".cargo")));
        cargo_home.is_some_and(|h| self.exe.starts_with(h.join("bin")))
    }

    /// True when the hook hasn't started a check in the last
    /// [`CHECK_EVERY`].
    pub fn check_due(&self) -> Result<bool> {
        let stamp = self.state_dir.join("update-check");
        let modified = match fs::metadata(&stamp) {
            Ok(meta) => meta
                .modified()
                .wrap_err_with(|| format!("failed to read the mtime of {}", stamp.display()))?,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(true),
            Err(e) => return Err(e).wrap_err_with(|| format!("failed to read {}", stamp.display())),
        };
        // A stamp from the future (clock changes) counts as stale.
        Ok(modified.elapsed().map_or(true, |age| age >= CHECK_EVERY))
    }

    /// Starts `dev self-update --background` if a check is due, detached so
    /// the prompt never waits for it. The stamp is updated first, so other
    /// shells don't start one too.
    pub fn spawn_if_due(&self, ctx: &Context) -> Result<()> {
        if !self.check_due()? {
            return Ok(());
        }

        fs::create_dir_all(&self.state_dir)
            .wrap_err_with(|| format!("failed to create {}", self.state_dir.display()))?;
        let stamp = self.state_dir.join("update-check");
        File::create(&stamp).wrap_err_with(|| format!("failed to write {}", stamp.display()))?;

        let log_path = self.state_dir.join("update.log");
        let log = File::create(&log_path).wrap_err_with(|| format!("failed to create {}", log_path.display()))?;
        ctx.command(&self.exe)
            .args(["self-update", "--background"])
            .stdin(Stdio::null())
            .stdout(log.try_clone().wrap_err("failed to share the update log")?)
            .stderr(log)
            // Its own process group, so Ctrl-C at the prompt doesn't stop it.
            .process_group(0)
            .spawn()
            .wrap_err("failed to start dev self-update")?;
        Ok(())
    }

    /// Saves `notice` for the next `dev` command to print.
    pub fn save_notice(&self, notice: &str) -> Result<()> {
        fs::create_dir_all(&self.state_dir)
            .wrap_err_with(|| format!("failed to create {}", self.state_dir.display()))?;
        let path = self.notice_path();
        fs::write(&path, format!("{notice}\n")).wrap_err_with(|| format!("failed to write {}", path.display()))
    }

    /// The saved notice, if there is one. It's removed, so it's only shown
    /// once.
    pub fn take_notice(&self) -> Result<Option<String>> {
        let path = self.notice_path();
        let notice = match fs::read_to_string(&path) {
            Ok(notice) => notice,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).wrap_err_with(|| format!("failed to read {}", path.display())),
        };
        fs::remove_file(&path).wrap_err_with(|| format!("failed to remove {}", path.display()))?;
        Ok(Some(notice.trim().to_string()))
    }

    fn notice_path(&self) -> PathBuf {
        self.state_dir.join("updated")
    }
}

/// The notice for `outcome`, e.g. `dev updated 0.1.0 -> 0.2.0`.
pub fn describe(outcome: &Outcome) -> String {
    match outcome {
        Outcome::UpToDate(v) => format!("dev {v} is the latest release"),
        Outcome::Updated { from, to } => format!("dev updated {from} -> {to}"),
    }
}
