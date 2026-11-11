use color_eyre::eyre::WrapErr;

use crate::{Context, Result, project::Project};

/// Meets every dependency in `project`, in order, reporting progress to
/// `ctx.out`:
///
/// ```text
/// Install brew dependencies ... applying
///   Install zerobrew ... met
///   Installing jq ... applying
/// ```
///
/// A dependency with tasks is met one task at a time, checking each task just
/// before applying it, since later tasks can rely on earlier ones (e.g.
/// packages need zerobrew). One without tasks is met with
/// [`Dependency::meet`](crate::deps::Dependency::meet).
///
/// Stops at the first failure.
pub fn up(ctx: &mut Context, project: &Project) -> Result<()> {
    for dep in &project.deps {
        let name = dep.name();
        let met = dep
            .is_met(ctx)
            .wrap_err_with(|| format!("failed to check dependency: {name}"))?;

        if met {
            writeln!(ctx.out, "{name} ... met")?;
            continue;
        }

        writeln!(ctx.out, "{name} ... applying")?;
        if dep.tasks().is_empty() {
            dep.meet(ctx).wrap_err_with(|| format!("dependency failed: {name}"))?;
            continue;
        }

        for task in dep.tasks() {
            let task_name = task.name();
            let applied = task
                .is_applied(ctx)
                .wrap_err_with(|| format!("failed to check task: {task_name} (in {name})"))?;

            if applied {
                writeln!(ctx.out, "  {task_name} ... met")?;
            } else {
                writeln!(ctx.out, "  {task_name} ... applying")?;
                task.apply(ctx)
                    .wrap_err_with(|| format!("task failed: {task_name} (in {name})"))?;
            }
        }
    }

    Ok(())
}
