mod common;

use common::{FakeDep, Log, capturing_ctx, project};
use dev::Result;

#[test]
fn down_runs_every_dependency_in_reverse_order() -> Result<()> {
    let log = Log::default();
    let project = project(vec![
        FakeDep::new("a", true, &log),
        FakeDep::new("b", false, &log),
        FakeDep::new("c", true, &log),
    ]);
    let (mut ctx, out, _) = capturing_ctx();

    dev::down(&mut ctx, &project)?;
    assert_eq!(vec!["down c", "down b", "down a"], *log.borrow());
    assert_eq!("c ... down\nb ... down\na ... down\n", out.to_string());
    Ok(())
}

#[test]
fn down_stops_at_first_failure() {
    let log = Log::default();
    let project = project(vec![
        FakeDep::new("a", true, &log),
        FakeDep::new("b", true, &log).failing_down(),
    ]);
    let (mut ctx, out, _) = capturing_ctx();

    let err = dev::down(&mut ctx, &project).expect_err("a failing down should fail");
    let chain: Vec<String> = err.chain().map(ToString::to_string).collect();

    assert_eq!(vec!["failed to take down dependency: b", "boom"], chain);
    assert_eq!(vec!["down b"], *log.borrow());
    assert_eq!("", out.to_string(), "only finished deps are reported");
}
