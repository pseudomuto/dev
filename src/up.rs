use color_eyre::eyre::WrapErr;

use crate::{
    Context,
    Result,
    project::Project,
    report::{self, Status},
};

/// Meets every dependency in `project` that isn't already met, in order,
/// reporting each one's progress to `ctx.reporter` as a step at depth 0. With
/// the [`Plain`](crate::report::Plain) reporter that reads:
///
/// ```text
/// Install brew dependencies ... applying
///   Install zerobrew ... met
///   Installing jq ... applying
/// ```
///
/// Each dependency reports its own tasks (depth 1) as it's met; see
/// [`meet_tasks`](crate::deps::meet_tasks).
///
/// Stops at the first failure.
pub fn up(ctx: &mut Context, project: &Project) -> Result<()> {
    for dep in &project.deps {
        let name = dep.name();
        report::step(ctx, 0, name, Status::Checking)?;
        let met = dep.is_met(ctx);
        let met = report::failing(ctx, 0, name, met).wrap_err_with(|| format!("failed to check dependency: {name}"))?;

        if met {
            report::step(ctx, 0, name, Status::Met)?;
            continue;
        }

        report::step(ctx, 0, name, Status::Applying)?;
        let res = dep.meet(ctx);
        report::failing(ctx, 0, name, res).wrap_err_with(|| format!("dependency failed: {name}"))?;
        report::step(ctx, 0, name, Status::Done)?;
    }

    Ok(())
}
