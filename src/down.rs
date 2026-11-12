use color_eyre::eyre::WrapErr;

use crate::{
    Context,
    Result,
    project::Project,
    report::{self, Status},
};

/// Takes down every dependency in `project`, in reverse order, so each one
/// goes before the dependencies listed ahead of it (which it may rely on).
/// Reports each one to `ctx.reporter` once it's done, which the
/// [`Plain`](crate::report::Plain) reporter prints as:
///
/// ```text
/// Install brew dependencies ... down
/// ```
///
/// Stops at the first failure.
pub fn down(ctx: &mut Context, project: &Project) -> Result<()> {
    for dep in project.deps.iter().rev() {
        let name = dep.name();
        report::step(ctx, 0, name, Status::TakingDown)?;
        let res = dep.down(ctx);
        report::failing(ctx, 0, name, res).wrap_err_with(|| format!("failed to take down dependency: {name}"))?;
        report::step(ctx, 0, name, Status::Down)?;
    }

    Ok(())
}
