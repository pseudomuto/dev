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
    /// `$GITHUB_PATH`'s contents.
    path: String,
    /// `$GITHUB_ENV`'s contents.
    env: String,
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
        path: fs::read_to_string(&files[1])?,
        env: fs::read_to_string(&files[2])?,
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

/// Vars for `cache-key`: a Linux x64 runner and dev 0.2.0.
const KEY_VARS: &[(&str, &str)] = &[
    ("DEV_ACTION_OS", "Linux"),
    ("DEV_ACTION_ARCH", "X64"),
    ("DEV_VERSION", "0.2.0"),
];

/// Makes `dir/work` a git repo holding `files` (path, contents), all
/// tracked, and returns its path.
fn workspace(dir: &TempDir, files: &[(&str, &str)]) -> Result<std::path::PathBuf> {
    let work = dir.path().join("work");
    fs::create_dir_all(&work)?;
    git(&work, &["init", "-q"])?;
    for (path, contents) in files {
        let file = work.join(path);
        fs::create_dir_all(file.parent().expect("files are in a dir"))?;
        fs::write(&file, contents)?;
        git(&work, &["add", path])?;
    }
    Ok(work)
}

/// The `key=` line of a `cache-key` run.
fn key(r: &Run) -> &str {
    assert!(r.status.success(), "{}", r.stderr);
    r.output.lines().find(|l| l.starts_with("key=")).expect("a key line")
}

#[test]
fn cache_key_is_stable_and_covers_dev_yaml() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let work = workspace(&dir, &[("dev.yaml", "name: demo\n")])?;

    let a = run("cache-key", &dir, KEY_VARS)?;
    assert!(
        a.output
            .starts_with("prefix=dev-linux-x64-0.2.0-\nkey=dev-linux-x64-0.2.0-"),
        "{}{}",
        a.output,
        a.stderr
    );
    assert_eq!(key(&a), key(&run("cache-key", &dir, KEY_VARS)?));

    fs::write(work.join("dev.yaml"), "name: other\n")?;
    assert_ne!(key(&a), key(&run("cache-key", &dir, KEY_VARS)?));
    Ok(())
}

/// Asserts changing `file` in the workspace changes the key (or doesn't, when
/// `changes` is false), with `vars` on top of [`KEY_VARS`].
fn assert_key_change(dir: &TempDir, file: &str, changes: bool, vars: &[(&str, &str)]) -> Result<()> {
    let vars: Vec<(&str, &str)> = KEY_VARS.iter().chain(vars).copied().collect();
    let before = run("cache-key", dir, &vars)?;
    fs::write(dir.path().join("work").join(file), "changed\n")?;
    let after = run("cache-key", dir, &vars)?;
    assert_eq!(changes, key(&before) != key(&after), "changing {file}");
    Ok(())
}

#[test]
fn cache_key_covers_rust_toolchain_toml() -> Result<()> {
    let dir = tempfile::tempdir()?;
    workspace(
        &dir,
        &[("dev.yaml", "name: demo\n"), ("rust-toolchain.toml", "[toolchain]\n")],
    )?;
    assert_key_change(&dir, "rust-toolchain.toml", true, &[])
}

#[test]
fn cache_key_ignores_language_manifests() -> Result<()> {
    let dir = tempfile::tempdir()?;
    workspace(&dir, &[("dev.yaml", "name: demo\n"), ("Cargo.toml", "[package]\n")])?;
    assert_key_change(&dir, "Cargo.toml", false, &[])
}

#[test]
fn cache_key_covers_cache_key_files() -> Result<()> {
    let dir = tempfile::tempdir()?;
    workspace(
        &dir,
        &[
            ("dev.yaml", "name: demo\n"),
            ("a/b/go.sum", "x\n"),
            (".tool-versions", "y\n"),
        ],
    )?;
    let files = [("INPUT_CACHE_KEY_FILES", "  **/go.sum\n.tool-versions\n\n")];
    assert_key_change(&dir, "a/b/go.sum", true, &files)?;
    assert_key_change(&dir, ".tool-versions", true, &files)
}

#[test]
fn cache_key_fails_on_a_glob_that_matches_nothing() -> Result<()> {
    let dir = tempfile::tempdir()?;
    workspace(&dir, &[("dev.yaml", "name: demo\n")])?;
    let vars: Vec<(&str, &str)> = KEY_VARS
        .iter()
        .chain(&[("INPUT_CACHE_KEY_FILES", "nope/*.lock")])
        .copied()
        .collect();
    let r = run("cache-key", &dir, &vars)?;
    assert_failed(&r, "cache-key-files pattern 'nope/*.lock' matched no tracked files");
    Ok(())
}

#[test]
fn cache_key_fails_without_a_dev_yaml() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let work = workspace(&dir, &[])?;
    let r = run("cache-key", &dir, KEY_VARS)?;
    // The script prints the real path; on macOS the temp dir is behind a
    // symlink (/var -> /private/var).
    assert_failed(&r, &format!("no dev.yaml at {}", fs::canonicalize(&work)?.display()));
    Ok(())
}

/// Writes `contents` to `dir/home/<rel>`, making its dirs.
fn home_file(dir: &TempDir, rel: &str, contents: &str) -> Result<()> {
    let file = dir.path().join("home").join(rel);
    fs::create_dir_all(file.parent().expect("files are in a dir"))?;
    fs::write(file, contents)?;
    Ok(())
}

#[test]
fn path_puts_dev_tools_first_in_shell_init_order() -> Result<()> {
    let dir = tempfile::tempdir()?;
    home_file(
        &dir,
        ".local/share/dev/shell/rustup",
        "export PATH=\"$HOME/.cargo/bin:$PATH\"\n",
    )?;
    home_file(
        &dir,
        ".local/share/dev/shell/zerobrew",
        "export PATH=\"$HOME/zb/bin:$HOME/zb/sbin:$PATH\"\n",
    )?;
    // Never sourced: mise's setup only works from a prompt hook.
    home_file(&dir, ".local/share/dev/shell/mise", "exit 1\n")?;
    fs::create_dir_all(dir.path().join("home/.local/share/mise/shims"))?;

    let r = run("path", &dir, &[])?;
    assert!(r.status.success(), "{}", r.stderr);
    let home = dir.path().join("home");
    let home = home.display();
    // GitHub puts the last line first, so this is lowest priority first.
    let want = [
        format!("{home}/.local/bin"),
        format!("{home}/zb/sbin"),
        format!("{home}/zb/bin"),
        format!("{home}/.cargo/bin"),
        format!("{home}/.local/share/mise/shims"),
    ];
    assert_eq!(want.join("\n") + "\n", r.path);
    Ok(())
}

#[test]
fn path_skips_tools_that_arent_set_up() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let r = run("path", &dir, &[])?;
    assert!(r.status.success(), "{}", r.stderr);
    assert_eq!(format!("{}/.local/bin\n", dir.path().join("home").display()), r.path);
    Ok(())
}

#[test]
fn path_follows_xdg_data_home() -> Result<()> {
    let dir = tempfile::tempdir()?;
    home_file(
        &dir,
        "data/dev/shell/rustup",
        "export PATH=\"$HOME/.cargo/bin:$PATH\"\n",
    )?;
    fs::create_dir_all(dir.path().join("home/data/mise/shims"))?;
    let data = dir.path().join("home/data");

    let r = run("path", &dir, &[("XDG_DATA_HOME", data.to_str().unwrap())])?;
    assert!(r.status.success(), "{}", r.stderr);
    let home = dir.path().join("home");
    let home = home.display();
    assert_eq!(
        format!("{home}/.local/bin\n{home}/.cargo/bin\n{home}/data/mise/shims\n"),
        r.path
    );
    Ok(())
}

/// Reads `$GITHUB_ENV` written as `NAME<<DELIM`, value lines, `DELIM`.
fn parse_github_env(s: &str) -> Vec<(String, String)> {
    let mut vars = Vec::new();
    let mut lines = s.lines();
    while let Some(line) = lines.next() {
        let (name, delim) = line
            .split_once("<<")
            .unwrap_or_else(|| panic!("not a NAME<<DELIM line: {line:?}"));
        let value: Vec<&str> = lines.by_ref().take_while(|l| *l != delim).collect();
        vars.push((name.to_string(), value.join("\n")));
    }
    vars
}

/// Runs `export-env` with the real dev on `PATH` and `yaml` as the
/// workspace's dev.yaml, plus `vars`.
fn export_env(dir: &TempDir, yaml: &str, vars: &[(&str, &str)]) -> Result<Run> {
    fs::create_dir_all(dir.path().join("work"))?;
    fs::write(dir.path().join("work/dev.yaml"), yaml)?;
    let path = path_with_dev();
    let vars: Vec<(&str, &str)> = [("PATH", path.as_str())].iter().chain(vars).copied().collect();
    let r = run("export-env", dir, &vars)?;
    assert!(r.status.success(), "{}", r.stderr);
    Ok(r)
}

#[test]
fn export_env_exports_dev_yaml_env() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let r = export_env(&dir, "name: demo\nenv:\n  GREETING: hi\n  DB: \"${DB:-local}\"\n", &[])?;
    let want = vec![
        ("DB".to_string(), "local".to_string()),
        ("GREETING".to_string(), "hi".to_string()),
    ];
    assert_eq!(want, parse_github_env(&r.env));
    assert!(!r.env.contains("__DEV_ENV"), "{}", r.env);
    Ok(())
}

#[test]
fn export_env_honours_overrides() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let r = export_env(&dir, "name: demo\nenv:\n  DB: \"${DB:-local}\"\n", &[("DB", "ci")])?;
    assert_eq!(vec![("DB".to_string(), "ci".to_string())], parse_github_env(&r.env));
    Ok(())
}

#[test]
fn export_env_keeps_values_exactly() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let value = "a\nb 'c' \"d\" $X =e \\n\n";
    // A YAML double-quoted scalar, with its escapes, so the value is exact.
    let yaml = format!("name: demo\nenv:\n  MULTI: {value:?}\n");
    let r = export_env(&dir, &yaml, &[])?;
    assert_eq!(vec![("MULTI".to_string(), value.to_string())], parse_github_env(&r.env));
    Ok(())
}

#[test]
fn export_env_sends_path_entries_to_github_path() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let r = export_env(&dir, "name: demo\nenv:\n  PATH: \"${PWD}/bin:${PATH}\"\n", &[])?;
    // `${PWD}` is the workspace as GitHub gives it, not with symlinks
    // resolved (on macOS the temp dir is behind /var -> /private/var).
    let work = dir.path().join("work");
    assert_eq!(format!("{}/bin\n", work.display()), r.path);
    assert_eq!("", r.env, "PATH must never go to $GITHUB_ENV");
    Ok(())
}

#[test]
fn path_exports_the_vars_a_tool_setup_sets() -> Result<()> {
    let dir = tempfile::tempdir()?;
    home_file(
        &dir,
        ".local/share/dev/shell/zerobrew",
        concat!(
            "export ZEROBREW_PREFIX=\"$HOME/zb\"\n",
            "export PKG_CONFIG_PATH=\"$ZEROBREW_PREFIX/lib/pkgconfig:${PKG_CONFIG_PATH:-}\"\n",
            "export PATH=\"$ZEROBREW_PREFIX/bin:$PATH\"\n",
        ),
    )?;

    let r = run("path", &dir, &[])?;
    assert!(r.status.success(), "{}", r.stderr);
    let home = dir.path().join("home");
    let home = home.display();
    let want = vec![
        ("PKG_CONFIG_PATH".to_string(), format!("{home}/zb/lib/pkgconfig:")),
        ("ZEROBREW_PREFIX".to_string(), format!("{home}/zb")),
    ];
    assert_eq!(want, parse_github_env(&r.env));
    assert_eq!(format!("{home}/.local/bin\n{home}/zb/bin\n"), r.path);
    Ok(())
}
