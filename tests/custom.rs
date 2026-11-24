mod common;

use std::{fs, path::Path};

use common::capturing_ctx;
use dev::{
    Result,
    deps::{
        Dependency,
        custom::{Custom, Entry},
    },
};

fn entry(name: &str, met: &str, meet: &str, down: Option<&str>) -> Entry {
    Entry {
        name: name.to_string(),
        met: met.to_string(),
        meet: meet.to_string(),
        down: down.map(str::to_string),
    }
}

fn custom(root: &Path, met: &str, meet: &str) -> Custom {
    Custom::new(root, entry("Thing", met, meet, None))
}

#[test]
fn is_met_when_met_exits_zero() -> Result<()> {
    let dir = tempfile::tempdir()?;
    assert!(custom(dir.path(), "true", "true").is_met(&mut capturing_ctx().0)?);
    Ok(())
}

#[test]
fn is_not_met_when_met_fails() -> Result<()> {
    let dir = tempfile::tempdir()?;
    assert!(!custom(dir.path(), "exit 3", "true").is_met(&mut capturing_ctx().0)?);
    Ok(())
}

#[test]
fn meet_runs_in_the_root_and_rechecks_met() -> Result<()> {
    let dir = tempfile::tempdir()?;
    custom(dir.path(), "test -f f", "echo hi > f").meet(&mut capturing_ctx().0)?;
    assert_eq!("hi\n", fs::read_to_string(dir.path().join("f"))?);
    Ok(())
}

#[test]
fn meet_fails_when_met_still_fails() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let err = custom(dir.path(), "false", "true")
        .meet(&mut capturing_ctx().0)
        .expect_err("met still failing should be an error");
    assert_eq!(r#"`meet` ran but `met` still fails for "Thing""#, err.to_string());
    Ok(())
}

#[test]
fn meet_failure_includes_its_output() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let err = custom(dir.path(), "false", "echo boom; exit 1")
        .meet(&mut capturing_ctx().0)
        .expect_err("a failing meet should be an error");
    let chain = format!("{err:#}");
    assert!(chain.contains("boom"), "error: {chain}");
    Ok(())
}

#[test]
fn meet_cant_read_stdin() -> Result<()> {
    let dir = tempfile::tempdir()?;
    // With stdin attached this would wait for input; it must fail instead.
    let res = custom(dir.path(), "false", "read x").meet(&mut capturing_ctx().0);
    assert!(res.is_err(), "read from /dev/null should fail meet");
    Ok(())
}

#[test]
fn snippets_get_the_context_env() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (mut ctx, _, _) = capturing_ctx();
    ctx.env = vec![("FOO".to_string(), "bar".to_string())];
    custom(dir.path(), "test -f f", r#"echo "$FOO" > f"#).meet(&mut ctx)?;
    assert_eq!("bar\n", fs::read_to_string(dir.path().join("f"))?);
    Ok(())
}

#[test]
fn down_runs_the_down_snippet() -> Result<()> {
    let dir = tempfile::tempdir()?;
    fs::write(dir.path().join("f"), "x")?;
    Custom::new(dir.path(), entry("Thing", "true", "true", Some("rm f"))).down(&mut capturing_ctx().0)?;
    assert!(!dir.path().join("f").exists());
    Ok(())
}

#[test]
fn down_without_a_snippet_does_nothing() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (mut ctx, _, log) = capturing_ctx();
    custom(dir.path(), "true", "true").down(&mut ctx)?;
    assert_eq!("", log.to_string());
    Ok(())
}

#[test]
fn entry_rejects_blank_fields() {
    let msg = serde_saphyr::from_str::<Entry>("name: x\nmet: ' '\nmeet: y\n")
        .expect_err("a blank met should be rejected")
        .to_string();
    assert!(
        msg.contains(r#"custom dependency "x": `met` can't be empty"#),
        "error: {msg}"
    );
}

#[test]
fn entry_rejects_a_blank_name() {
    let msg = serde_saphyr::from_str::<Entry>("name: ''\nmet: x\nmeet: y\n")
        .expect_err("a blank name should be rejected")
        .to_string();
    assert!(msg.contains("custom dependency `name` can't be empty"), "error: {msg}");
}
