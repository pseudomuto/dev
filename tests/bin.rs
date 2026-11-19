mod common;

use std::{
    env,
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use common::write_executable;
use dev::{
    Result,
    deps::bin::{Found, locate},
};
use tempfile::tempdir;

fn search(dirs: &[PathBuf]) -> Result<OsString> {
    Ok(env::join_paths(dirs)?)
}

fn exe(path: &Path) -> Result<()> {
    write_executable(path, b"#!/bin/sh\n")
}

#[test]
fn locate_is_managed_when_path_finds_the_managed_binary() -> Result<()> {
    let d = tempdir()?;
    let managed = d.path().join("managed/mise");
    exe(&managed)?;

    let found = locate(&search(&[d.path().join("managed")])?, "mise", &managed);
    assert_eq!(Found::Managed(managed), found);
    Ok(())
}

#[test]
fn locate_is_external_when_path_finds_another() -> Result<()> {
    let d = tempdir()?;
    let managed = d.path().join("managed/mise");
    exe(&managed)?;
    exe(&d.path().join("other/mise"))?;

    let found = locate(&search(&[d.path().join("other")])?, "mise", &managed);
    assert_eq!(Found::External(d.path().join("other/mise")), found);
    Ok(())
}

#[test]
fn locate_is_managed_when_only_the_managed_binary_exists() -> Result<()> {
    let d = tempdir()?;
    let managed = d.path().join("managed/mise");
    exe(&managed)?;

    let found = locate(&search(&[d.path().join("empty")])?, "mise", &managed);
    assert_eq!(Found::Managed(managed), found);
    Ok(())
}

#[test]
fn locate_is_missing_when_nothing_is_there() -> Result<()> {
    let d = tempdir()?;
    let managed = d.path().join("managed/mise");

    let found = locate(&search(&[d.path().join("empty")])?, "mise", &managed);
    assert_eq!(Found::Missing(managed), found);
    Ok(())
}

#[test]
fn locate_skips_relative_path_entries() -> Result<()> {
    let d = tempdir()?;
    let managed = d.path().join("managed/mise");

    let found = locate(&OsString::from("bin:."), "mise", &managed);
    assert_eq!(Found::Missing(managed), found);
    Ok(())
}

#[test]
fn locate_handles_an_empty_search_path() -> Result<()> {
    let d = tempdir()?;
    let managed = d.path().join("managed/mise");

    let found = locate(&OsString::new(), "mise", &managed);
    assert_eq!(Found::Missing(managed), found);
    Ok(())
}

#[test]
fn locate_skips_files_that_are_not_executable() -> Result<()> {
    let d = tempdir()?;
    let managed = d.path().join("managed/mise");
    exe(&d.path().join("a/mise"))?;
    fs::set_permissions(d.path().join("a/mise"), fs::Permissions::from_mode(0o644))?;
    exe(&d.path().join("b/mise"))?;

    let found = locate(&search(&[d.path().join("a"), d.path().join("b")])?, "mise", &managed);
    assert_eq!(Found::External(d.path().join("b/mise")), found);
    Ok(())
}

#[test]
fn search_path_prefers_ctx_env() {
    let ctx = dev::Context {
        env: vec![("PATH".into(), "/a:/b".into())],
        ..dev::Context::default()
    };
    assert_eq!(OsString::from("/a:/b"), dev::deps::bin::search_path(&ctx));
}

/// Writes an installer script to `dir/install.sh` and returns it as a
/// `file://` URL.
fn installer(dir: &Path, body: &str) -> Result<String> {
    let path = dir.join("install.sh");
    fs::write(&path, body)?;
    Ok(format!("file://{}", path.display()))
}

fn script<'a>(url: &'a str, expect: &'a Path) -> dev::deps::bin::Script<'a> {
    dev::deps::bin::Script {
        what: "thing",
        url,
        shell: "sh",
        args: &[],
        env: &[],
        interactive: false,
        expect,
    }
}

#[test]
fn run_fails_with_the_url_when_the_script_fails() -> Result<()> {
    let d = tempdir()?;
    let url = installer(d.path(), "exit 3\n")?;
    let expect = d.path().join("out/thing");
    let (mut ctx, _, _) = common::capturing_ctx();

    let msg = dev::deps::bin::run(&mut ctx, script(&url, &expect))
        .expect_err("a failing installer should fail")
        .to_string();
    assert_eq!(format!("thing installer ({url}) failed"), msg);
    Ok(())
}

#[test]
fn run_fails_when_the_script_writes_no_binary() -> Result<()> {
    let d = tempdir()?;
    let url = installer(d.path(), "true\n")?;
    let expect = d.path().join("out/thing");
    let (mut ctx, _, _) = common::capturing_ctx();

    let msg = dev::deps::bin::run(&mut ctx, script(&url, &expect))
        .expect_err("an installer that installs nothing should fail")
        .to_string();
    assert_eq!(
        format!(
            "the thing installer ({url}) finished, but there's no thing at {}",
            expect.display()
        ),
        msg
    );
    Ok(())
}

#[test]
fn run_fails_when_the_script_cannot_be_fetched() -> Result<()> {
    // Without `set -o pipefail`, curl's failure would be swallowed and sh
    // would exit 0 after reading an empty script.
    let d = tempdir()?;
    let url = format!("file://{}", d.path().join("missing").display());
    let expect = d.path().join("out/thing");
    let (mut ctx, _, log) = common::capturing_ctx();

    let msg = dev::deps::bin::run(&mut ctx, script(&url, &expect))
        .expect_err("a missing installer should fail")
        .to_string();
    assert_eq!(format!("thing installer ({url}) failed"), msg);
    assert!(log.to_string().contains("curl: (37)"), "log: {log}");
    Ok(())
}

#[test]
fn run_passes_args_and_env_with_spaces_intact() -> Result<()> {
    let d = tempdir()?;
    let url = installer(
        d.path(),
        "mkdir -p \"$(dirname \"$OUT\")\"\nprintf '%s|%s\\n' \"$1\" \"$OUT\" > \"$OUT\"\nchmod +x \"$OUT\"\n",
    )?;
    let expect = d.path().join("a b/thing");
    let (mut ctx, _, _) = common::capturing_ctx();

    dev::deps::bin::run(
        &mut ctx,
        dev::deps::bin::Script {
            args: &["x y"],
            env: &[("OUT", expect.as_os_str())],
            ..script(&url, &expect)
        },
    )?;
    assert_eq!(format!("x y|{}\n", expect.display()), fs::read_to_string(&expect)?);
    Ok(())
}
