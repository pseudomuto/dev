use color_eyre::eyre::WrapErr;

use crate::{
    Context,
    Result,
    deps::{Status, report},
    project::Project,
};

/// Meets every dependency in `project` that isn't already met, in order,
/// reporting progress to `ctx.out`:
///
/// ```text
/// Install brew dependencies ... applying
///   Install zerobrew ... met
///   Installing jq ... applying
/// ```
///
/// Each dependency reports its own steps (the indented lines) as it's met;
/// see [`meet_tasks`](crate::deps::meet_tasks).
///
/// Stops at the first failure.
pub fn up(ctx: &mut Context, project: &Project) -> Result<()> {
    for dep in &project.deps {
        let name = dep.name();
        let met = dep
            .is_met(ctx)
            .wrap_err_with(|| format!("failed to check dependency: {name}"))?;

        if met {
            report(ctx, "", name, Status::Met)?;
            continue;
        }

        report(ctx, "", name, Status::Applying)?;
        dep.meet(ctx).wrap_err_with(|| format!("dependency failed: {name}"))?;
    }

    Ok(())
}
