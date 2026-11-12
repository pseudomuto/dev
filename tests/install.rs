//! Runs `install.sh` against a fake release on disk (`file://` URLs), so
//! nothing is downloaded and nothing outside a temp dir is touched.

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use dev::Result;
use tempfile::TempDir;

/// Every target `install.sh` can pick, so the fake release works on any host.
const TARGETS: &[&str] = &[
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-gnu",
];

/// Writes a release of `version` under `<dir>/releases`, laid out like
/// GitHub's, with a `dev` that prints `fake dev`.
#[allow(clippy::disallowed_methods)]
fn fake_release(dir: &TempDir, version: &str) -> Result<()> {
    let staging = dir.path().join("staging");
    fs::create_dir_all(&staging)?;
    fs::write(staging.join("dev"), "#!/bin/sh\necho fake dev\n")?;

    let release = dir.path().join(format!("releases/download/v{version}"));
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
    Ok(())
}

/// Runs `install.sh` with releases from `dir` and `$HOME` at `dir/home`,
/// installing to `dir/bin`, plus `vars`.
#[allow(clippy::disallowed_methods)]
fn install(dir: &TempDir, vars: &[(&str, &str)]) -> Result<Output> {
    let home = dir.path().join("home");
    fs::create_dir_all(&home)?;
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");

    Ok(Command::new("sh")
        .arg(script)
        .env_clear()
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .env("DEV_DOWNLOAD_URL", format!("file://{}/releases", dir.path().display()))
        .env("DEV_INSTALL_DIR", dir.path().join("bin"))
        .envs(vars.iter().copied())
        .output()?)
}

#[test]
#[allow(clippy::disallowed_methods)]
fn installs_the_release_for_this_machine() -> Result<()> {
    let dir = tempfile::tempdir()?;
    fake_release(&dir, "9.9.9")?;

    let output = install(&dir, &[("DEV_VERSION", "v9.9.9")])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        Some(0),
        output.status.code(),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bin = dir.path().join("bin");
    let installed = Command::new(bin.join("dev")).output()?;
    assert_eq!("fake dev\n", String::from_utf8_lossy(&installed.stdout));

    // bin isn't on the PATH it ran with, so it says to add it.
    assert!(
        stdout.contains(&format!("export PATH=\"{}:$PATH\"", bin.display())),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("eval \"$(dev shell-init)\""), "stdout: {stdout}");

    let names: Vec<_> = fs::read_dir(&bin)?.map(|e| Ok(e?.file_name())).collect::<Result<_>>()?;
    assert_eq!(vec!["dev"], names, "no temp files left behind");
    assert_eq!(0, fs::read_dir(dir.path().join("home"))?.count(), "HOME untouched");
    Ok(())
}

#[test]
fn refuses_a_download_that_fails_its_checksum() -> Result<()> {
    let dir = tempfile::tempdir()?;
    fake_release(&dir, "9.9.9")?;
    let sums = dir.path().join("releases/download/v9.9.9/SHA256SUMS");
    let tampered = fs::read_to_string(&sums)?
        .lines()
        .map(|line| {
            format!(
                "{} {}\n",
                "0".repeat(64),
                line.split_once(' ').expect("hash then name").1
            )
        })
        .collect::<String>();
    fs::write(&sums, tampered)?;

    let output = install(&dir, &[("DEV_VERSION", "9.9.9")])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(Some(1), output.status.code(), "stderr: {stderr}");
    assert!(
        stderr.contains("error: checksum mismatch for dev-v9.9.9-"),
        "stderr: {stderr}"
    );
    assert!(!dir.path().join("bin").exists(), "nothing should be installed");
    Ok(())
}

#[test]
fn fails_when_the_latest_release_cant_be_found() -> Result<()> {
    let dir = tempfile::tempdir()?;

    let output = install(&dir, &[])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(Some(1), output.status.code(), "stderr: {stderr}");
    assert!(
        stderr.contains("error: couldn't look up the latest release"),
        "stderr: {stderr}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("Downloading"), "it should stop there: {stdout}");
    Ok(())
}
