//! Runs `share/action/dev-action.sh`, the logic behind `action.yaml`, with a
//! temp `$HOME` and workspace, and temp files standing in for the ones GitHub
//! gives each step (`$GITHUB_OUTPUT`, `$GITHUB_PATH`, `$GITHUB_ENV`). Nothing
//! is downloaded and nothing outside the temp dir is touched.

use std::{
    fs,
    path::Path,
    process::{Command, ExitStatus},
};

use dev::Result;
use tempfile::TempDir;

/// What a run of the script did.
struct Run {
    status: ExitStatus,
    stderr: String,
    /// `$GITHUB_OUTPUT`'s contents.
    output: String,
}

/// Runs `dev-action.sh <sub>` with `$HOME` at `dir/home`, cwd and
/// `$GITHUB_WORKSPACE` at `dir/work`, `PATH=/usr/bin:/bin`, plus `vars`.
#[allow(clippy::disallowed_methods)]
fn run(sub: &str, dir: &TempDir, vars: &[(&str, &str)]) -> Result<Run> {
    let (home, work, gh) = (dir.path().join("home"), dir.path().join("work"), dir.path().join("gh"));
    for d in [&home, &work, &gh] {
        fs::create_dir_all(d)?;
    }
    let files = ["output", "path", "env"].map(|name| gh.join(name));
    for file in &files {
        fs::write(file, "")?;
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("share/action/dev-action.sh");

    let out = Command::new("sh")
        .arg(script)
        .arg(sub)
        .current_dir(&work)
        .env_clear()
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .env("GITHUB_WORKSPACE", &work)
        .env("GITHUB_OUTPUT", &files[0])
        .env("GITHUB_PATH", &files[1])
        .env("GITHUB_ENV", &files[2])
        .envs(vars.iter().copied())
        .output()?;

    Ok(Run {
        status: out.status,
        stderr: String::from_utf8(out.stderr)?,
        output: fs::read_to_string(&files[0])?,
    })
}

#[test]
fn resolve_version_uses_the_input() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let r = run("resolve-version", &dir, &[("INPUT_VERSION", "v0.2.0")])?;
    assert!(r.status.success(), "{}", r.stderr);
    assert_eq!("version=0.2.0\ninstalled=false\n", r.output);
    Ok(())
}

#[test]
fn resolve_version_uses_a_version_tag_ref() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let r = run("resolve-version", &dir, &[("DEV_ACTION_REF", "v0.3.1")])?;
    assert!(r.status.success(), "{}", r.stderr);
    assert_eq!("version=0.3.1\ninstalled=false\n", r.output);
    Ok(())
}

#[test]
fn resolve_version_reads_the_ref_from_the_action_path() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("_actions/pseudomuto/dev/v0.3.1");
    let r = run(
        "resolve-version",
        &dir,
        &[("GITHUB_ACTION_PATH", path.to_str().unwrap())],
    )?;
    assert!(r.status.success(), "{}", r.stderr);
    assert_eq!("version=0.3.1\ninstalled=false\n", r.output);
    Ok(())
}

/// Runs `git` in `dir`, isolated from the host's git config.
#[allow(clippy::disallowed_methods)]
fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()?;
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(String::from_utf8(out.stdout)?.trim().to_string())
}

/// Makes a repo at `dir/repo` with one commit, tagged `tag` the way the
/// Release workflow tags (annotated). Returns the commit's SHA.
fn tagged_repo(dir: &TempDir, tag: &str) -> Result<String> {
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo)?;
    git(&repo, &["init", "-q"])?;
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "release"])?;
    git(&repo, &["tag", "-a", tag, "-m", tag])?;
    git(&repo, &["rev-parse", "HEAD"])
}

#[test]
fn resolve_version_finds_the_tag_on_a_pinned_sha() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let sha = tagged_repo(&dir, "v0.4.0")?;
    let repo = dir.path().join("repo");
    let vars = [
        ("DEV_ACTION_REF", sha.as_str()),
        ("DEV_ACTION_REPO", repo.to_str().unwrap()),
    ];
    let r = run("resolve-version", &dir, &vars)?;
    assert!(r.status.success(), "{}", r.stderr);
    assert_eq!("version=0.4.0\ninstalled=false\n", r.output);
    Ok(())
}

/// Asserts `r` failed with `msg` somewhere in its stderr, and output nothing.
fn assert_failed(r: &Run, msg: &str) {
    assert!(!r.status.success(), "expected a failure, got output: {}", r.output);
    assert!(r.stderr.contains(msg), "expected {msg:?} in stderr: {}", r.stderr);
    assert_eq!("", r.output);
}

#[test]
fn resolve_version_fails_on_an_untagged_sha() -> Result<()> {
    let dir = tempfile::tempdir()?;
    tagged_repo(&dir, "v0.4.0")?;
    let repo = dir.path().join("repo");
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let r = run(
        "resolve-version",
        &dir,
        &[("DEV_ACTION_REF", sha), ("DEV_ACTION_REPO", repo.to_str().unwrap())],
    )?;
    assert_failed(&r, &format!("commit {sha} has no release tag; set the version input"));
    Ok(())
}

#[test]
fn resolve_version_fails_on_a_branch_ref() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let r = run("resolve-version", &dir, &[("DEV_ACTION_REF", "main")])?;
    assert_failed(
        &r,
        "can't tell which dev to install from ref 'main'; set the version input",
    );
    Ok(())
}

#[test]
fn resolve_version_rejects_a_bad_version() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let r = run("resolve-version", &dir, &[("INPUT_VERSION", "latest")])?;
    assert_failed(&r, "version 'latest' isn't a version like 0.2.0");
    Ok(())
}

/// `PATH` with the dev these tests were built with first.
fn path_with_dev() -> String {
    let bin = Path::new(env!("CARGO_BIN_EXE_dev")).parent().expect("dev is in a dir");
    format!("{}:/usr/bin:/bin", bin.display())
}

#[test]
fn resolve_version_uses_an_installed_dev() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = path_with_dev();
    let r = run(
        "resolve-version",
        &dir,
        &[("INPUT_VERSION", "installed"), ("PATH", &path)],
    )?;
    assert!(r.status.success(), "{}", r.stderr);
    assert_eq!(
        format!("version={}\ninstalled=true\n", env!("CARGO_PKG_VERSION")),
        r.output
    );
    Ok(())
}

#[test]
fn resolve_version_fails_when_installed_dev_is_missing() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let r = run("resolve-version", &dir, &[("INPUT_VERSION", "installed")])?;
    assert_failed(&r, "version is 'installed' but there's no dev on PATH");
    Ok(())
}
