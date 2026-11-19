//! Runs `share/action/dev-action.sh`, the logic behind `action.yaml`, with a
//! temp `$HOME` and workspace, and temp files standing in for the ones GitHub
//! gives each step (`$GITHUB_OUTPUT`, `$GITHUB_PATH`, `$GITHUB_ENV`). Nothing
//! is downloaded and nothing outside the temp dir is touched.

use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    process::{Command, ExitStatus},
};

use dev::{
    Result,
    action,
    deps::{brew, mise, rust},
    layout::{Host, Layout},
};
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
    let r = run("resolve-version", &dir, &[("INPUT_VERSION", "v0.4.2")])?;
    assert!(r.status.success(), "{}", r.stderr);
    assert_eq!("version=0.4.2\ninstalled=false\n", r.output);
    Ok(())
}

#[test]
fn resolve_version_uses_a_version_tag_ref() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let r = run("resolve-version", &dir, &[("DEV_ACTION_REF", "v1.0.1")])?;
    assert!(r.status.success(), "{}", r.stderr);
    assert_eq!("version=1.0.1\ninstalled=false\n", r.output);
    Ok(())
}

#[test]
fn resolve_version_reads_the_ref_from_the_action_path() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("_actions/pseudomuto/dev/v1.0.1");
    let r = run(
        "resolve-version",
        &dir,
        &[("GITHUB_ACTION_PATH", path.to_str().unwrap())],
    )?;
    assert!(r.status.success(), "{}", r.stderr);
    assert_eq!("version=1.0.1\ninstalled=false\n", r.output);
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

/// Vars for `cache-key`: a Linux x64 runner on GitHub's Ubuntu 24.04 image,
/// and dev 0.2.0.
const KEY_VARS: &[(&str, &str)] = &[
    ("DEV_ACTION_OS", "Linux"),
    ("ImageOS", "ubuntu24"),
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

/// The lines of multiline output `name` (`name<<DELIM` ... `DELIM`) in
/// `$GITHUB_OUTPUT`'s contents `s`.
fn output_lines(s: &str, name: &str) -> Vec<String> {
    let mut lines = s.lines();
    let start = format!("{name}<<");
    let delim = lines
        .by_ref()
        .find_map(|l| l.strip_prefix(&start))
        .unwrap_or_else(|| panic!("no {name} output in:\n{s}"));
    lines.take_while(|l| *l != delim).map(str::to_string).collect()
}

#[test]
fn cache_key_lists_the_paths_dev_reports_for_both_caches() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let work = workspace(&dir, &[("dev.yaml", "name: demo\n")])?;
    let data = dir.path().join("home/data");
    let xdg = [("XDG_DATA_HOME", data.to_str().unwrap())];
    let vars: Vec<(&str, &str)> = KEY_VARS.iter().chain(&xdg).copied().collect();

    let r = run_with_dev("cache-key", &dir, &vars)?;
    assert!(r.status.success(), "{}", r.stderr);
    let want = action::paths(&layout(&dir, &xdg)?, Host::current(), Path::new(&dev_dir()), &work);
    assert_eq!(want.cache, output_lines(&r.output, "cache-paths"));
    assert_eq!(want.brew_cache, output_lines(&r.output, "brew-paths"));
    let installs = data.join("mise/installs").display().to_string();
    assert!(
        want.cache.contains(&installs),
        "XDG_DATA_HOME moves the cache: {:?}",
        want.cache
    );
    Ok(())
}

/// Writes an executable `path` that logs its args and the
/// `MISE_DEFAULT_CONFIG_FILENAME` it saw to `log`.
fn fake_logger(path: &Path, log: &Path) -> Result<()> {
    fs::create_dir_all(path.parent().expect("files are in a dir"))?;
    let body = format!(
        "#!/bin/sh\necho \"$* var=${{MISE_DEFAULT_CONFIG_FILENAME:-}}\" >> '{}'\n",
        log.display()
    );
    fs::write(path, body)?;
    fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;
    Ok(())
}

#[test]
fn prune_runs_mise_with_the_project_env() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mise = dir.path().join("home/tools/mise");
    let log = dir.path().join("log");
    fake_logger(&mise, &log)?;
    fs::create_dir_all(dir.path().join("work/.dev"))?;
    fs::write(dir.path().join("work/.dev/mise.toml"), "[tools]\n")?;

    let r = run_with_dev("prune", &dir, &[("MISE_INSTALL_PATH", mise.to_str().unwrap())])?;
    assert!(r.status.success(), "{}", r.stderr);
    assert_eq!("prune --tools var=.dev/mise.toml\n", fs::read_to_string(&log)?);
    Ok(())
}

#[test]
fn prune_does_nothing_before_mise_is_installed() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let r = run_with_dev("prune", &dir, &[])?;
    assert!(r.status.success(), "{}", r.stderr);
    Ok(())
}

#[test]
fn brew_installed_says_whether_theres_a_brew_cache_to_save() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path().join("home/zb-root");
    let vars = [("ZEROBREW_ROOT", root.to_str().unwrap())];

    let r = run_with_dev("brew-installed", &dir, &vars)?;
    assert!(r.status.success(), "{}", r.stderr);
    assert_eq!("installed=false\n", r.output, "nothing installed yet");

    fs::create_dir_all(&root)?;
    let r = run_with_dev("brew-installed", &dir, &vars)?;
    assert!(r.status.success(), "{}", r.stderr);
    // macOS has no brew cache (see `action::Paths::brew_cache`).
    let cached = !action::paths(
        &layout(&dir, &vars)?,
        Host::current(),
        Path::new(&dev_dir()),
        &dir.path().join("work"),
    )
    .brew_cache
    .is_empty();
    assert_eq!(format!("installed={cached}\n"), r.output);
    Ok(())
}

#[test]
fn cache_key_is_stable_and_covers_dev_yaml() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let work = workspace(&dir, &[("dev.yaml", "name: demo\n")])?;

    let a = run_with_dev("cache-key", &dir, KEY_VARS)?;
    assert!(
        a.output
            .starts_with("prefix=dev-linux-x64-ubuntu24-0.2.0-\nkey=dev-linux-x64-ubuntu24-0.2.0-"),
        "{}{}",
        a.output,
        a.stderr
    );
    assert_eq!(key(&a), key(&run_with_dev("cache-key", &dir, KEY_VARS)?));

    fs::write(work.join("dev.yaml"), "name: other\n")?;
    assert_ne!(key(&a), key(&run_with_dev("cache-key", &dir, KEY_VARS)?));
    Ok(())
}

/// Asserts changing `file` in the workspace changes the key (or doesn't, when
/// `changes` is false), with `vars` on top of [`KEY_VARS`].
fn assert_key_change(dir: &TempDir, file: &str, changes: bool, vars: &[(&str, &str)]) -> Result<()> {
    let vars: Vec<(&str, &str)> = KEY_VARS.iter().chain(vars).copied().collect();
    let before = run_with_dev("cache-key", dir, &vars)?;
    fs::write(dir.path().join("work").join(file), "changed\n")?;
    let after = run_with_dev("cache-key", dir, &vars)?;
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
    let r = run_with_dev("cache-key", &dir, &vars)?;
    assert_failed(&r, "cache-key-files pattern 'nope/*.lock' matched no tracked files");
    Ok(())
}

#[test]
fn cache_key_fails_without_a_dev_yaml() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let work = workspace(&dir, &[])?;
    let r = run_with_dev("cache-key", &dir, KEY_VARS)?;
    // The script prints the real path; on macOS the temp dir is behind a
    // symlink (/var -> /private/var).
    assert_failed(&r, &format!("no dev.yaml at {}", fs::canonicalize(&work)?.display()));
    Ok(())
}

/// Runs `dev-action.sh <sub>` like [`run`], with the built dev on `PATH`.
fn run_with_dev(sub: &str, dir: &TempDir, vars: &[(&str, &str)]) -> Result<Run> {
    let path = path_with_dev();
    let vars: Vec<(&str, &str)> = [("PATH", path.as_str())].iter().chain(vars).copied().collect();
    run(sub, dir, &vars)
}

/// The dir the built dev is in, which `path` adds last.
fn dev_dir() -> String {
    let bin = Path::new(env!("CARGO_BIN_EXE_dev")).parent().expect("dev is in a dir");
    bin.display().to_string()
}

/// The layout dev sees with the test's `$HOME` and `vars`.
fn layout(dir: &TempDir, vars: &[(&str, &str)]) -> Result<Layout> {
    let mut env = BTreeMap::from([("HOME".to_string(), dir.path().join("home").display().to_string())]);
    env.extend(vars.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    Layout::from_env(&env, Host::current())
}

/// Writes `contents` as setup `name`, where dev keeps it for `layout`.
fn setup_file(layout: &Layout, name: &str, contents: &str) -> Result<()> {
    fs::create_dir_all(&layout.shell_dir)?;
    fs::write(layout.shell_dir.join(name), contents)?;
    Ok(())
}

#[test]
fn path_puts_dev_tools_first_in_shell_init_order() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let l = layout(&dir, &[])?;
    setup_file(&l, rust::SHELL_FILE, "export PATH=\"$HOME/.cargo/bin:$PATH\"\n")?;
    setup_file(
        &l,
        brew::SHELL_FILE,
        "export PATH=\"$HOME/zb/bin:$HOME/zb/sbin:$PATH\"\n",
    )?;
    // Never sourced: mise's setup only works from a prompt hook.
    setup_file(&l, mise::SHELL_FILE, "exit 1\n")?;
    fs::create_dir_all(l.mise_data.join("shims"))?;

    let r = run_with_dev("path", &dir, &[])?;
    assert!(r.status.success(), "{}", r.stderr);
    let home = dir.path().join("home");
    let home = home.display();
    // GitHub puts the last line first, so this is lowest priority first.
    let want = [
        dev_dir(),
        format!("{home}/zb/sbin"),
        format!("{home}/zb/bin"),
        format!("{home}/.cargo/bin"),
        l.mise_data.join("shims").display().to_string(),
    ];
    assert_eq!(want.join("\n") + "\n", r.path);
    Ok(())
}

#[test]
fn path_skips_tools_that_arent_set_up() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let r = run_with_dev("path", &dir, &[])?;
    assert!(r.status.success(), "{}", r.stderr);
    assert_eq!(format!("{}\n", dev_dir()), r.path);
    Ok(())
}

#[test]
fn path_follows_xdg_data_home_and_mise_data_dir() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let data = dir.path().join("home/data");
    let mise = dir.path().join("home/mise");
    for vars in [
        vec![("XDG_DATA_HOME", data.to_str().unwrap())],
        vec![("MISE_DATA_DIR", mise.to_str().unwrap())],
    ] {
        let dir = tempfile::tempdir()?;
        let l = layout(&dir, &vars)?;
        setup_file(&l, rust::SHELL_FILE, "export PATH=\"$HOME/.cargo/bin:$PATH\"\n")?;
        fs::create_dir_all(l.mise_data.join("shims"))?;

        let r = run_with_dev("path", &dir, &vars)?;
        assert!(r.status.success(), "{vars:?}: {}", r.stderr);
        let home = dir.path().join("home");
        let want = format!(
            "{}\n{}/.cargo/bin\n{}\n",
            dev_dir(),
            home.display(),
            l.mise_data.join("shims").display()
        );
        assert_eq!(want, r.path, "{vars:?}");
    }
    Ok(())
}

#[test]
fn path_fails_without_a_dev_that_has_action_paths() -> Result<()> {
    let dir = tempfile::tempdir()?;
    // Like an older dev, which runs `action-paths` as a dev.yaml command.
    let bin = dir.path().join("old");
    fs::create_dir_all(&bin)?;
    fs::write(bin.join("dev"), "#!/bin/sh\necho 'no such command' >&2\nexit 1\n")?;
    fs::set_permissions(bin.join("dev"), std::os::unix::fs::PermissionsExt::from_mode(0o755))?;
    let path = format!("{}:/usr/bin:/bin", bin.display());

    let r = run("path", &dir, &[("PATH", path.as_str())])?;
    assert_failed(
        &r,
        "doesn't have `dev action-paths`; this action needs dev 0.4.0 or later",
    );
    Ok(())
}

#[test]
fn path_points_mise_at_the_project_config_when_there_is_one() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let work = dir.path().join("work/.dev");
    fs::create_dir_all(&work)?;
    fs::write(work.join("mise.toml"), "[tools]\n")?;

    let r = run_with_dev("path", &dir, &[])?;
    assert!(r.status.success(), "{}", r.stderr);
    assert_eq!(
        vec![("MISE_DEFAULT_CONFIG_FILENAME".to_string(), ".dev/mise.toml".to_string())],
        parse_github_env(&r.env)
    );
    Ok(())
}

#[test]
fn path_leaves_mise_alone_without_a_project_config() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let r = run_with_dev("path", &dir, &[])?;
    assert!(r.status.success(), "{}", r.stderr);
    assert_eq!("", r.env);
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
    setup_file(
        &layout(&dir, &[])?,
        brew::SHELL_FILE,
        concat!(
            "export ZEROBREW_PREFIX=\"$HOME/zb\"\n",
            "export PKG_CONFIG_PATH=\"$ZEROBREW_PREFIX/lib/pkgconfig:${PKG_CONFIG_PATH:-}\"\n",
            "export PATH=\"$ZEROBREW_PREFIX/bin:$PATH\"\n",
        ),
    )?;

    let r = run_with_dev("path", &dir, &[])?;
    assert!(r.status.success(), "{}", r.stderr);
    let home = dir.path().join("home");
    let home = home.display();
    let want = vec![
        ("PKG_CONFIG_PATH".to_string(), format!("{home}/zb/lib/pkgconfig:")),
        ("ZEROBREW_PREFIX".to_string(), format!("{home}/zb")),
    ];
    assert_eq!(want, parse_github_env(&r.env));
    assert_eq!(format!("{}\n{home}/zb/bin\n", dev_dir()), r.path);
    Ok(())
}

#[test]
fn path_fails_when_a_tool_setup_fails() -> Result<()> {
    let dir = tempfile::tempdir()?;
    setup_file(&layout(&dir, &[])?, rust::SHELL_FILE, "exit 3\n")?;
    let r = run_with_dev("path", &dir, &[])?;
    assert!(!r.status.success(), "expected a failure, got PATH: {}", r.path);
    assert!(r.stderr.contains("couldn't load the shell setup in"), "{}", r.stderr);
    Ok(())
}

#[test]
fn cache_key_covers_files_with_non_ascii_names() -> Result<()> {
    let dir = tempfile::tempdir()?;
    workspace(&dir, &[("dev.yaml", "name: demo\n"), ("café.lock", "x\n")])?;
    assert_key_change(&dir, "café.lock", true, &[("INPUT_CACHE_KEY_FILES", "*.lock")])
}

#[test]
fn cache_key_separates_runner_images() -> Result<()> {
    let dir = tempfile::tempdir()?;
    workspace(&dir, &[("dev.yaml", "name: demo\n")])?;
    let older: Vec<(&str, &str)> = KEY_VARS.iter().chain(&[("ImageOS", "ubuntu22")]).copied().collect();
    let r = run_with_dev("cache-key", &dir, &older)?;
    assert!(
        r.output.starts_with("prefix=dev-linux-x64-ubuntu22-0.2.0-\n"),
        "{}{}",
        r.output,
        r.stderr
    );
    Ok(())
}

#[test]
fn cache_key_works_out_the_image_without_image_os() -> Result<()> {
    let dir = tempfile::tempdir()?;
    workspace(&dir, &[("dev.yaml", "name: demo\n")])?;
    // Self-hosted runners don't set ImageOS; the host's OS stands in.
    let vars: Vec<(&str, &str)> = KEY_VARS.iter().chain(&[("ImageOS", "")]).copied().collect();
    let r = run_with_dev("cache-key", &dir, &vars)?;
    let prefix = r.output.lines().next().unwrap_or_default();
    let image = prefix.split('-').nth(3).unwrap_or_default();
    let (name, version) = image.split_at(image.find(|c: char| c.is_ascii_digit()).unwrap_or(image.len()));
    assert!(
        !name.is_empty() && !version.is_empty() && version.chars().all(|c| c.is_ascii_digit()),
        "expected an image like ubuntu24 or macos15 in {prefix:?}: {}",
        r.stderr
    );
    Ok(())
}

#[test]
fn resolve_version_rejects_a_dev_older_than_the_action_needs() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for version in ["0.3.9", "v0.2.0"] {
        let r = run("resolve-version", &dir, &[("INPUT_VERSION", version)])?;
        assert_failed(
            &r,
            "is older than this action needs (0.4.0 or later); set the version input to 0.4.0 or later",
        );
    }
    for version in ["0.4.0", "0.10.0", "1.0.0"] {
        let r = run("resolve-version", &dir, &[("INPUT_VERSION", version)])?;
        assert!(r.status.success(), "{version}: {}", r.stderr);
    }
    Ok(())
}

#[test]
fn resolve_version_trusts_the_action_path_over_the_ref() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("_actions/pseudomuto/dev/v1.0.1");
    let vars = [
        ("GITHUB_ACTION_PATH", path.to_str().unwrap()),
        ("DEV_ACTION_REF", "main"),
    ];
    let r = run("resolve-version", &dir, &vars)?;
    assert!(r.status.success(), "{}", r.stderr);
    assert_eq!("version=1.0.1\ninstalled=false\n", r.output);
    Ok(())
}
