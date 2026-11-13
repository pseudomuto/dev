mod common;

use std::fs;

use dev::{
    Result,
    deps::{
        Dependency,
        packages::{Eco, Mode, Packages},
    },
};
use tempfile::{TempDir, tempdir};

/// A project dir holding `files`, each empty.
fn project(files: &[&str]) -> Result<TempDir> {
    let dir = tempdir()?;
    for file in files {
        fs::write(dir.path().join(file), "")?;
    }
    Ok(dir)
}

fn names(dep: &dyn Dependency) -> Vec<&str> {
    dep.tasks().iter().map(|t| t.name()).collect()
}

#[test]
fn detects_ecosystems_from_their_files() -> Result<()> {
    let dir = project(&[
        "package.json",
        "yarn.lock",
        "Gemfile",
        "go.mod",
        "Cargo.toml",
        "uv.lock",
    ])?;

    let packages = Packages::detect(dir.path(), Mode::Local);

    assert_eq!(
        names(&packages),
        [
            "Installing node packages (yarn)",
            "Installing ruby packages (bundler)",
            "Installing go packages (go)",
            "Installing rust packages (cargo)",
            "Installing python packages (uv)",
        ]
    );
    Ok(())
}

/// Fake `npm` and `node` in `<dir>/bin`. `npm` appends its args and cwd to
/// `<dir>/calls` and creates `node_modules`, unless `<dir>/fail` exists.
/// With `<dir>/touch-lock`, it also changes `package-lock.json`.
/// `node --version` prints `<dir>/version`.
fn fake_node(dir: &TempDir) -> Result<String> {
    let d = dir.path().display();
    common::write_executable(
        &dir.path().join("bin/npm"),
        format!(
            r#"#!/bin/sh
echo "$* in $PWD" >> '{d}/calls'
[ -e '{d}/fail' ] && {{ echo "npm broke" >&2; exit 1; }}
[ -e '{d}/touch-lock' ] && echo changed >> package-lock.json
mkdir -p node_modules
"#
        )
        .as_bytes(),
    )?;
    common::write_executable(
        &dir.path().join("bin/node"),
        format!("#!/bin/sh\ncat '{d}/version'\n").as_bytes(),
    )?;
    fs::write(dir.path().join("version"), "v22.0.0\n")?;
    Ok(format!("{d}/bin:/usr/bin:/bin"))
}

fn calls(dir: &TempDir) -> Result<Vec<String>> {
    match fs::read_to_string(dir.path().join("calls")) {
        Ok(calls) => Ok(calls.lines().map(str::to_owned).collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

/// A node project in its own dir, with fake tools in another, and a context
/// whose PATH finds them.
fn node_project() -> Result<(TempDir, TempDir, dev::Context)> {
    let root = project(&["package.json", "package-lock.json"])?;
    let tools = tempdir()?;
    let path = fake_node(&tools)?;
    let (mut ctx, _, _) = common::capturing_ctx();
    ctx.env.push(("PATH".into(), path));
    Ok((root, tools, ctx))
}

#[test]
fn installs_until_the_fingerprint_matches() -> Result<()> {
    let (root, tools, mut ctx) = node_project()?;
    let packages = Packages::detect(root.path(), Mode::Local);
    let task = &packages.tasks()[0];

    assert!(!task.is_applied(&mut ctx)?);
    task.apply(&mut ctx)?;

    let cwd = root.path().canonicalize()?;
    assert_eq!(calls(&tools)?, [format!("install in {}", cwd.display())]);
    assert!(root.path().join(".dev/packages/node").is_file());
    assert!(task.is_applied(&mut ctx)?);
    Ok(())
}

#[test]
fn a_new_lockfile_or_runtime_is_out_of_date() -> Result<()> {
    let (root, tools, mut ctx) = node_project()?;
    let packages = Packages::detect(root.path(), Mode::Local);
    let task = &packages.tasks()[0];

    task.apply(&mut ctx)?;
    fs::write(root.path().join("package-lock.json"), "{}")?;
    assert!(!task.is_applied(&mut ctx)?, "lockfile changed");

    task.apply(&mut ctx)?;
    fs::write(tools.path().join("version"), "v24.0.0\n")?;
    assert!(!task.is_applied(&mut ctx)?, "node changed");
    Ok(())
}

#[test]
fn a_missing_install_dir_is_out_of_date() -> Result<()> {
    let (root, _tools, mut ctx) = node_project()?;
    let packages = Packages::detect(root.path(), Mode::Local);
    let task = &packages.tasks()[0];

    task.apply(&mut ctx)?;
    fs::remove_dir(root.path().join("node_modules"))?;

    assert!(!task.is_applied(&mut ctx)?);
    Ok(())
}

#[test]
fn a_failed_install_writes_no_fingerprint() -> Result<()> {
    let (root, tools, mut ctx) = node_project()?;
    fs::write(tools.path().join("fail"), "")?;
    let packages = Packages::detect(root.path(), Mode::Local);

    let err = packages.tasks()[0].apply(&mut ctx).unwrap_err();

    assert!(format!("{err:?}").contains("npm broke"), "{err:?}");
    assert!(!root.path().join(".dev/packages/node").exists());
    Ok(())
}

#[test]
fn only_installs_the_listed_ecosystems_in_order() -> Result<()> {
    let dir = project(&["package.json", "Gemfile", "Cargo.toml"])?;

    let packages = Packages::only(dir.path(), &[Eco::Ruby, Eco::Node], Mode::Local)?;
    assert_eq!(
        names(&packages),
        ["Installing ruby packages (bundler)", "Installing node packages (npm)"]
    );

    let err = Packages::only(dir.path(), &[Eco::Go], Mode::Local)
        .err()
        .expect("go has no files");
    let msg = err.to_string();
    assert!(msg.contains("`go`") && msg.contains("go.mod"), "{msg}");
    Ok(())
}

#[test]
fn ci_installs_from_the_lockfile_as_is() -> Result<()> {
    let (root, tools, mut ctx) = node_project()?;
    let packages = Packages::detect(root.path(), Mode::Ci);

    packages.tasks()[0].apply(&mut ctx)?;

    let cwd = root.path().canonicalize()?;
    assert_eq!(calls(&tools)?, [format!("ci in {}", cwd.display())]);
    Ok(())
}

#[test]
fn ci_freezes_bundler() -> Result<()> {
    let root = project(&["Gemfile"])?;
    let tools = tempdir()?;
    let d = tools.path().display();
    common::write_executable(
        &tools.path().join("bin/bundle"),
        format!("#!/bin/sh\necho \"$* frozen=$BUNDLE_FROZEN\" >> '{d}/calls'\n").as_bytes(),
    )?;
    common::write_executable(&tools.path().join("bin/ruby"), b"#!/bin/sh\necho ruby 3.4.0\n")?;
    let (mut ctx, _, _) = common::capturing_ctx();
    ctx.env.push(("PATH".into(), format!("{d}/bin:/usr/bin:/bin")));

    Packages::detect(root.path(), Mode::Ci).tasks()[0].apply(&mut ctx)?;
    Packages::detect(root.path(), Mode::Local).tasks()[0].apply(&mut ctx)?;

    assert_eq!(calls(&tools)?, ["install frozen=true", "install frozen="]);
    Ok(())
}

#[test]
fn ci_fails_when_an_install_changes_its_files() -> Result<()> {
    let (root, tools, mut ctx) = node_project()?;
    fs::write(tools.path().join("touch-lock"), "")?;
    let packages = Packages::detect(root.path(), Mode::Ci);

    let err = packages.tasks()[0].apply(&mut ctx).unwrap_err();

    let msg = format!("{err:?}");
    assert!(msg.contains("`npm ci` changed package-lock.json"), "{msg}");
    assert!(!root.path().join(".dev/packages/node").exists());
    Ok(())
}
