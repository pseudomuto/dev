mod common;

use common::{FakeTask, Log, capturing_ctx};
use dev::{
    Result,
    deps::{Task, meet_tasks, tasks_met},
};

fn tasks(tasks: Vec<FakeTask>) -> Vec<Box<dyn Task>> {
    tasks.into_iter().map(|t| Box::new(t) as Box<dyn Task>).collect()
}

#[test]
fn tasks_met_when_all_tasks_are_applied() -> Result<()> {
    let log = Log::default();
    let tasks = tasks(vec![FakeTask::new("a", true, &log), FakeTask::new("b", true, &log)]);

    assert!(tasks_met(&tasks, &mut capturing_ctx().0)?);
    assert_eq!(vec!["check a", "check b"], *log.borrow());
    Ok(())
}

#[test]
fn tasks_met_stops_at_first_unapplied_task() -> Result<()> {
    let log = Log::default();
    let tasks = tasks(vec![
        FakeTask::new("a", true, &log),
        FakeTask::new("b", false, &log),
        FakeTask::new("c", true, &log),
    ]);

    assert!(!tasks_met(&tasks, &mut capturing_ctx().0)?);
    assert_eq!(vec!["check a", "check b"], *log.borrow());
    Ok(())
}

#[test]
fn meet_tasks_applies_only_unapplied_tasks() -> Result<()> {
    let log = Log::default();
    let tasks = tasks(vec![FakeTask::new("a", true, &log), FakeTask::new("b", false, &log)]);
    let (mut ctx, out, _) = capturing_ctx();

    meet_tasks(&tasks, &mut ctx)?;
    assert_eq!(vec!["check a", "check b", "apply b"], *log.borrow());
    assert_eq!("  a ... met\n  b ... applying\n", out.to_string());
    Ok(())
}

#[test]
fn meet_tasks_stops_and_names_the_failing_task() {
    let log = Log::default();
    let tasks = tasks(vec![
        FakeTask::new("a", false, &log).failing_apply(),
        FakeTask::new("b", false, &log),
    ]);

    let err = meet_tasks(&tasks, &mut capturing_ctx().0).expect_err("a failing task should fail meet");
    let chain: Vec<String> = err.chain().map(ToString::to_string).collect();

    assert_eq!(vec!["task failed: a", "boom"], chain);
    assert_eq!(vec!["check a", "apply a"], *log.borrow());
}
