mod common;

use std::io::Write;

use common::Buffer;
use dev::{Context, Result};

#[test]
fn can_set_out() -> Result<()> {
    let out = Buffer::default();
    let mut ctx = Context {
        out: Box::new(out.clone()),
        ..Context::default()
    };

    write!(ctx.out, "Some output")?;
    assert_eq!("Some output", out.to_string());
    Ok(())
}

#[test]
fn can_set_err() -> Result<()> {
    let err = Buffer::default();
    let mut ctx = Context {
        err: Box::new(err.clone()),
        ..Context::default()
    };

    write!(ctx.err, "Some error output")?;
    assert_eq!("Some error output", err.to_string());
    Ok(())
}

#[test]
fn command_applies_env() -> Result<()> {
    let ctx = Context {
        env: vec![("DEV_GREETING".into(), "hi there".into())],
        ..Context::default()
    };

    let output = ctx.command("sh").args(["-c", "echo $DEV_GREETING"]).output()?;
    assert_eq!("hi there\n", String::from_utf8_lossy(&output.stdout));
    Ok(())
}
