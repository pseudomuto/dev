mod common;

use common::{FakeDep, Log, capturing_ctx, project, recording_ctx};
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

#[test]
fn down_reports_each_dependency_from_taking_down_to_down() -> Result<()> {
    let log = Log::default();
    let project = project(vec![FakeDep::new("a", true, &log), FakeDep::new("b", true, &log)]);
    let (mut ctx, events) = recording_ctx();

    dev::down(&mut ctx, &project)?;
    assert_eq!(
        vec!["0 b taking down", "0 b down", "0 a taking down", "0 a down"],
        *events.lock().unwrap()
    );
    Ok(())
}

#[test]
fn down_reports_a_failing_dependency_as_failed() {
    let log = Log::default();
    let project = project(vec![FakeDep::new("a", true, &log).failing_down()]);
    let (mut ctx, events) = recording_ctx();

    let err = dev::down(&mut ctx, &project).expect_err("a failing down should fail");
    assert_eq!("failed to take down dependency: a", err.to_string());
    assert_eq!(vec!["0 a taking down", "0 a failed"], *events.lock().unwrap());
}
