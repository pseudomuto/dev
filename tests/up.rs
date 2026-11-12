mod common;

use common::{FakeDep, FakeTask, Log, capturing_ctx, project};
use dev::Result;

#[test]
fn up_skips_dependencies_that_are_met() -> Result<()> {
    let log = Log::default();
    let task = FakeTask::new("t", false, &log);
    let project = project(vec![FakeDep::new("a", true, &log).with_tasks(vec![task])]);
    let (mut ctx, out, _) = capturing_ctx();

    dev::up(&mut ctx, &project)?;
    assert_eq!("a ... met\n", out.to_string());
    assert_eq!(vec!["check dep a"], *log.borrow());
    Ok(())
}

#[test]
fn up_applies_unapplied_tasks_with_indented_status() -> Result<()> {
    let log = Log::default();
    let tasks = vec![FakeTask::new("t1", true, &log), FakeTask::new("t2", false, &log)];
    let project = project(vec![FakeDep::new("a", false, &log).with_tasks(tasks)]);
    let (mut ctx, out, _) = capturing_ctx();

    dev::up(&mut ctx, &project)?;
    assert_eq!("a ... applying\n  t1 ... met\n  t2 ... applying\n", out.to_string());
    assert_eq!(
        vec!["check dep a", "meet a", "check t1", "check t2", "apply t2"],
        *log.borrow()
    );
    Ok(())
}

#[test]
fn up_meets_dependencies_without_tasks_in_order() -> Result<()> {
    let log = Log::default();
    let project = project(vec![FakeDep::new("a", false, &log), FakeDep::new("b", true, &log)]);
    let (mut ctx, out, _) = capturing_ctx();

    dev::up(&mut ctx, &project)?;
    assert_eq!("a ... applying\nb ... met\n", out.to_string());
    assert_eq!(vec!["check dep a", "meet a", "check dep b"], *log.borrow());
    Ok(())
}

#[test]
fn up_stops_at_first_failure() {
    let log = Log::default();
    let tasks = vec![
        FakeTask::new("t1", false, &log).failing_apply(),
        FakeTask::new("t2", false, &log),
    ];
    let project = project(vec![
        FakeDep::new("a", false, &log).with_tasks(tasks),
        FakeDep::new("b", false, &log),
    ]);
    let (mut ctx, out, _) = capturing_ctx();

    let err = dev::up(&mut ctx, &project).expect_err("a failing task should fail up");
    let chain: Vec<String> = err.chain().map(ToString::to_string).collect();

    assert_eq!(vec!["dependency failed: a", "task failed: t1", "boom"], chain);
    assert_eq!("a ... applying\n  t1 ... applying\n", out.to_string());
    assert_eq!(vec!["check dep a", "meet a", "check t1", "apply t1"], *log.borrow());
}
