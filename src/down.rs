use std::io::Write;

use color_eyre::eyre::WrapErr;

use crate::{Context, Result, project::Project};

/// Takes down every dependency in `project`, in reverse order, so each one
/// goes before the dependencies listed ahead of it (which it may rely on).
/// Reports each one to `ctx.out` once it's done:
///
/// ```text
/// Install brew dependencies ... down
/// ```
///
/// Stops at the first failure.
pub fn down(ctx: &mut Context, project: &Project) -> Result<()> {
    for dep in project.deps.iter().rev() {
        let name = dep.name();
        dep.down(ctx)
            .wrap_err_with(|| format!("failed to take down dependency: {name}"))?;
        writeln!(ctx.out, "{name} ... down")?;
    }

    Ok(())
}
