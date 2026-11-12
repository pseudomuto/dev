mod common;

use std::{fs, path::PathBuf};

use common::capturing_ctx;
use dev::{
    Result,
    deps::{
        Dependency,
        Task,
        rust::{Entry, Rust, Rustup, Spec, Toolchain},
    },
};
use tempfile::{TempDir, tempdir};

/// A fake `rustup` in `dir` that appends each call's args to `dir/calls`, and
/// answers `toolchain list`, `component list` and `target list` with the
/// contents of `dir/toolchains`, `dir/components` and `dir/targets`.
struct FakeRustup {
    dir: PathBuf,
    path: PathBuf,
}

impl FakeRustup {
    fn new(dir: &TempDir) -> Result<Self> {
        let d = dir.path().display();
        let body = format!(
            r#"#!/bin/sh
echo "$*" >> '{d}/calls'
case "$1 $2" in
  "toolchain list") cat '{d}/toolchains' 2>/dev/null ;;
  "component list") cat '{d}/components' 2>/dev/null ;;
  "target list") cat '{d}/targets' 2>/dev/null ;;
esac
exit 0
"#
        );
        let path = dir.path().join("bin/rustup");
        common::write_executable(&path, body.as_bytes())?;
        Ok(Self {
            dir: dir.path().to_path_buf(),
            path,
        })
    }

    /// Makes `list` (`toolchains`, `components` or `targets`) answer `lines`.
    fn has(&self, list: &str, lines: &str) -> Result<()> {
        Ok(fs::write(self.dir.join(list), lines)?)
    }

    fn calls(&self) -> Result<Vec<String>> {
        let calls = match fs::read_to_string(self.dir.join("calls")) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            res => res?,
        };
        Ok(calls.lines().map(str::to_owned).collect())
    }
}

fn named(name: &str, components: &[&str]) -> Entry {
    Entry::Named {
        name: name.to_string(),
        components: components.iter().map(|c| c.to_string()).collect(),
    }
}

#[test]
fn spec_reads_channel_components_and_multiline_targets() -> Result<()> {
    let toml = r#"
[toolchain]
channel = "1.99.0"  # pinned
components = ["clippy", "rustfmt"]
targets = [
  "aarch64-apple-darwin",
  "x86_64-unknown-linux-musl",
]
profile = "minimal"
"#;
    let expected = Spec {
        channel: "1.99.0".to_string(),
        components: vec!["clippy".to_string(), "rustfmt".to_string()],
        targets: vec![
            "aarch64-apple-darwin".to_string(),
            "x86_64-unknown-linux-musl".to_string(),
        ],
    };
    assert_eq!(expected, Spec::parse(toml)?);

    let err = Spec::parse("[toolchain]\nprofile = \"minimal\"\n").expect_err("no channel");
    assert_eq!("it has no `channel = \"...\"`", err.to_string());
    Ok(())
}

#[test]
fn named_toolchain_installs_with_its_components_until_they_are_there() -> Result<()> {
    let dir = tempdir()?;
    let rustup = FakeRustup::new(&dir)?;
    let task = Toolchain::new(named("nightly", &["rustfmt"]))
        .with_rustup(&rustup.path)
        .with_dir(dir.path());
    let (mut ctx, _, _) = capturing_ctx();
    assert_eq!("Installing Rust nightly", task.name());

    rustup.has("toolchains", "stable-aarch64-apple-darwin (default)\n")?;
    assert!(!task.is_applied(&mut ctx)?, "no nightly yet");

    task.apply(&mut ctx)?;
    assert_eq!(
        Some("toolchain install nightly --profile minimal --component rustfmt"),
        rustup.calls()?.last().map(String::as_str)
    );

    rustup.has(
        "toolchains",
        "stable-aarch64-apple-darwin (default)\nnightly-aarch64-apple-darwin\n",
    )?;
    rustup.has("components", "rust-std-aarch64-apple-darwin\n")?;
    assert!(!task.is_applied(&mut ctx)?, "nightly, but no rustfmt");

    rustup.has(
        "components",
        "rust-std-aarch64-apple-darwin\nrustfmt-aarch64-apple-darwin\n",
    )?;
    assert!(task.is_applied(&mut ctx)?);
    Ok(())
}

#[test]
fn toolchain_file_installs_what_the_file_names() -> Result<()> {
    let dir = tempdir()?;
    let rustup = FakeRustup::new(&dir)?;
    fs::write(
        dir.path().join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"1.99.0\"\ncomponents = [\"clippy\"]\ntargets = [\n  \"x86_64-unknown-linux-musl\",\n]\n",
    )?;
    let task = Toolchain::new(Entry::File)
        .with_rustup(&rustup.path)
        .with_dir(dir.path());
    let (mut ctx, _, _) = capturing_ctx();
    assert_eq!("Installing the toolchain in rust-toolchain.toml", task.name());

    rustup.has("toolchains", "1.99.0-aarch64-apple-darwin\n")?;
    rustup.has("components", "clippy-aarch64-apple-darwin\n")?;
    rustup.has("targets", "aarch64-apple-darwin\n")?;
    assert!(!task.is_applied(&mut ctx)?, "the musl target is missing");

    task.apply(&mut ctx)?;
    // No name: rustup reads rust-toolchain.toml from the dir it runs in.
    assert_eq!(Some("toolchain install"), rustup.calls()?.last().map(String::as_str));

    rustup.has("targets", "aarch64-apple-darwin\nx86_64-unknown-linux-musl\n")?;
    assert!(task.is_applied(&mut ctx)?);
    Ok(())
}

#[test]
fn rustup_uses_an_external_rustup_as_is() -> Result<()> {
    let dir = tempdir()?;
    let rustup = FakeRustup::new(&dir)?;
    let task = Rustup::default()
        .with_search_path(rustup.path.parent().expect("rustup is in bin/"))
        .with_cargo_home(dir.path().join("cargo"))
        .with_shell_dir(dir.path().join("shell"))
        .with_install_url("file:///nonexistent/installer");
    let (mut ctx, _, _) = capturing_ctx();

    assert!(task.is_applied(&mut ctx)?);
    task.apply(&mut ctx)?;
    assert!(!dir.path().join("shell").exists(), "whatever installed it set it up");
    Ok(())
}

/// A fake rustup installer at `dir/installer.sh`, returned as a `file://`
/// URL. Like the real one it installs `$CARGO_HOME/bin/rustup` and writes
/// `$CARGO_HOME/env`, and it saves its args to `$CARGO_HOME/installer-args`.
fn installer(dir: &TempDir) -> Result<String> {
    let path = dir.path().join("installer.sh");
    fs::write(
        &path,
        r#"set -e
mkdir -p "$CARGO_HOME/bin"
echo "$*" > "$CARGO_HOME/installer-args"
printf '#!/bin/sh\nexit 0\n' > "$CARGO_HOME/bin/rustup"
chmod 755 "$CARGO_HOME/bin/rustup"
printf 'export PATH="%s/bin:$PATH"\n' "$CARGO_HOME" > "$CARGO_HOME/env"
"#,
    )?;
    Ok(format!("file://{}", path.display()))
}

#[test]
fn rustup_installs_into_cargo_home_when_missing() -> Result<()> {
    let dir = tempdir()?;
    let cargo_home = dir.path().join("cargo");
    let task = Rustup::default()
        .with_search_path(dir.path().join("empty"))
        .with_cargo_home(&cargo_home)
        .with_shell_dir(dir.path().join("shell"))
        .with_install_url(installer(&dir)?);
    let (mut ctx, _, _) = capturing_ctx();

    assert!(!task.is_applied(&mut ctx)?);
    task.apply(&mut ctx)?;

    assert_eq!(
        "-y --no-modify-path --default-toolchain none\n",
        fs::read_to_string(cargo_home.join("installer-args"))?
    );
    assert_eq!(
        fs::read_to_string(cargo_home.join("env"))?,
        fs::read_to_string(dir.path().join("shell/rustup"))?,
        "the shell setup is rustup's own env script"
    );
    assert!(task.is_applied(&mut ctx)?);
    Ok(())
}

#[test]
fn rust_puts_cargo_bin_on_path_for_what_comes_after() -> Result<()> {
    let dir = tempdir()?;
    let cargo_home = dir.path().join("cargo");
    let dep = Rust::with_tasks(Vec::new()).with_cargo_home(&cargo_home);
    let (mut ctx, _, _) = capturing_ctx();
    ctx.env = vec![("PATH".to_string(), "/proj/bin:/usr/bin".to_string())];

    assert!(dep.is_met(&mut ctx)?, "no tasks, so met");
    dep.is_met(&mut ctx)?;

    let want = format!("{}/bin:/proj/bin:/usr/bin", cargo_home.display());
    assert_eq!(
        vec![("PATH".to_string(), want)],
        ctx.env,
        "first, once, and keeping the project's"
    );
    Ok(())
}
