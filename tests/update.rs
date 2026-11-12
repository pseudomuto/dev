//! The pure parts directly, then `dev self-update` and the prompt hook's
//! background update end to end, on a copy of `dev` in a temp dir, against a
//! fake release over `file://`. Nothing is downloaded.

use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};

use dev::{
    Result,
    update::{Version, latest_from_sums},
};
use tempfile::TempDir;

mod common;

/// Every target `install.sh` can pick, so the fake release works anywhere.
const TARGETS: &[&str] = &[
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-gnu",
];

/// A temp dir holding a copy of `dev` at `bin/dev`, a `$HOME`, and releases.
struct Sandbox {
    dir: TempDir,
}

impl Sandbox {
    fn new() -> Result<Self> {
        let dir = tempfile::tempdir()?;
        fs::create_dir_all(dir.path().join("bin"))?;
        fs::create_dir_all(dir.path().join("home"))?;
        common::copy_executable(env!("CARGO_BIN_EXE_dev").as_ref(), &dir.path().join("bin/dev"))?;
        Ok(Self { dir })
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    /// Where dev keeps its update state, under the sandbox's `$HOME`.
    fn state(&self, name: &str) -> PathBuf {
        self.path("home/.local/state/dev").join(name)
    }

    /// Publishes `version` as the latest release, with a `dev` that prints
    /// `fake dev <version>`.
    #[allow(clippy::disallowed_methods)]
    fn release(&self, version: &str) -> Result<()> {
        let staging = self.path("staging");
        fs::create_dir_all(&staging)?;
        fs::write(staging.join("dev"), format!("#!/bin/sh\necho fake dev {version}\n"))?;

        let release = self.path(&format!("releases/download/v{version}"));
        fs::create_dir_all(&release)?;
        for target in TARGETS {
            let asset = release.join(format!("dev-v{version}-{target}.tar.gz"));
            let status = Command::new("tar")
                .arg("-czf")
                .arg(&asset)
                .arg("-C")
                .arg(&staging)
                .arg("dev")
                .status()?;
            assert!(status.success(), "tar failed");
        }
        let status = Command::new("sh")
            .arg("-c")
            .arg("(sha256sum *.tar.gz 2>/dev/null || shasum -a 256 *.tar.gz) > SHA256SUMS")
            .current_dir(&release)
            .status()?;
        assert!(status.success(), "checksumming failed");

        // What GitHub serves for the latest release.
        let latest = self.path("releases/latest/download");
        fs::create_dir_all(&latest)?;
        fs::copy(release.join("SHA256SUMS"), latest.join("SHA256SUMS"))?;
        Ok(())
    }

    /// Runs the sandbox's `dev <args>` with only the env it needs, plus
    /// `vars`.
    #[allow(clippy::disallowed_methods)]
    fn dev(&self, args: &[&str], vars: &[(&str, &str)]) -> Result<Output> {
        Ok(Command::new(self.path("bin/dev"))
            .args(args)
            .current_dir(self.path("home"))
            .env_clear()
            .env("HOME", self.path("home"))
            .env("PATH", "/usr/bin:/bin")
            .env(
                "DEV_DOWNLOAD_URL",
                format!("file://{}/releases", self.dir.path().display()),
            )
            .envs(vars.iter().copied())
            .output()?)
    }

    /// What the sandbox's `dev` prints for `--version`, whichever it is now.
    fn installed(&self) -> Result<String> {
        let output = self.dev(&["--version"], &[])?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

#[test]
fn versions_parse_and_compare_numerically() -> Result<()> {
    let v: Version = "0.10.2".parse()?;
    assert_eq!("0.10.2", v.to_string());
    assert!(v > "0.9.9".parse()?, "10 > 9, not as strings");
    assert!(v < "1.0.0".parse()?);
    assert_eq!(v, "v0.10.2".parse()?, "a leading v is fine");

    for bad in ["", "1.2", "1.2.3.4", "1.x.3", "1.2.3-rc1"] {
        assert!(bad.parse::<Version>().is_err(), "`{bad}` should not parse");
    }
    Ok(())
}

#[test]
fn latest_comes_from_the_asset_names_in_sha256sums() -> Result<()> {
    let sums = "\
aaaa  dev-v0.2.0-aarch64-apple-darwin.tar.gz
bbbb  dev-v0.2.0-x86_64-unknown-linux-musl.tar.gz
";
    assert_eq!(Some("0.2.0".parse()?), latest_from_sums(sums));
    assert_eq!(None, latest_from_sums("<html>Not Found</html>"));
    Ok(())
}

#[test]
fn self_update_installs_a_newer_release() -> Result<()> {
    let sandbox = Sandbox::new()?;
    sandbox.release("9.9.9")?;

    let output = sandbox.dev(&["self-update"], &[])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        Some(0),
        output.status.code(),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(format!("dev updated {} -> 9.9.9\n", Version::current()), stdout);
    assert_eq!("fake dev 9.9.9", sandbox.installed()?);
    Ok(())
}

#[test]
fn self_update_leaves_the_latest_release_alone() -> Result<()> {
    let sandbox = Sandbox::new()?;
    let current = Version::current().to_string();
    sandbox.release(&current)?;

    let output = sandbox.dev(&["self-update"], &[])?;
    assert_eq!(Some(0), output.status.code());
    assert_eq!(
        format!("dev {current} is the latest release\n"),
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        format!("dev {current}"),
        sandbox.installed()?,
        "the real dev, not the fake"
    );
    Ok(())
}

#[test]
fn the_prompt_hook_updates_in_the_background() -> Result<()> {
    let sandbox = Sandbox::new()?;
    sandbox.release("9.9.9")?;

    let output = sandbox.dev(&["hook-env"], &[("DEV_AUTO_UPDATE", "1")])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(Some(0), output.status.code(), "stderr: {stderr}");
    assert!(stderr.is_empty(), "the prompt stays quiet: {stderr}");
    assert!(sandbox.state("update-check").is_file(), "the check is stamped");

    // It's detached, so wait for it.
    let notice = sandbox.state("updated");
    let deadline = Instant::now() + Duration::from_secs(15);
    while !notice.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    let log = fs::read_to_string(sandbox.state("update.log")).unwrap_or_default();
    assert_eq!(
        format!("dev updated {} -> 9.9.9\n", Version::current()),
        fs::read_to_string(&notice).unwrap_or_else(|_| panic!("no notice; update.log:\n{log}"))
    );
    assert_eq!("fake dev 9.9.9", sandbox.installed()?);
    Ok(())
}

#[test]
fn the_prompt_hook_skips_when_disabled_or_checked_recently() -> Result<()> {
    let sandbox = Sandbox::new()?;
    sandbox.release("9.9.9")?;

    let output = sandbox.dev(&["hook-env"], &[("DEV_AUTO_UPDATE", "0")])?;
    assert_eq!(Some(0), output.status.code());
    assert!(!sandbox.state("update-check").exists(), "disabled: nothing stamped");

    fs::create_dir_all(sandbox.path("home/.local/state/dev"))?;
    fs::write(sandbox.state("update-check"), "")?;
    let output = sandbox.dev(&["hook-env"], &[("DEV_AUTO_UPDATE", "1")])?;
    assert_eq!(Some(0), output.status.code());

    // The hook creates the log just before starting an update.
    assert!(
        !sandbox.state("update.log").exists(),
        "checked just now: nothing started"
    );
    assert_eq!(format!("dev {}", Version::current()), sandbox.installed()?);
    Ok(())
}
