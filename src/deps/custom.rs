use std::{
    path::{Path, PathBuf},
    process::Command,
};

use color_eyre::eyre::bail;
use serde::Deserialize;

use crate::{
    Context,
    Result,
    deps::{Dependency, Example},
    process,
};

/// One entry under `deps.custom`: a shell check, a fix, and an optional
/// teardown.
///
/// ```yaml
/// deps:
///   custom:
///     - name: Ensure file exists
///       met: test -f test-file.txt
///       meet: echo "yo yo yo" > test-file.txt
///       down: rm -f test-file.txt
/// ```
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, try_from = "RawEntry")]
pub struct Entry {
    /// Shown as the dependency's name.
    pub name: String,
    /// Exits 0 when the dependency is met.
    pub met: String,
    /// Run when `met` fails. `met` must pass afterwards.
    pub meet: String,
    /// Run by `dev down`, if set.
    pub down: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntry {
    name: String,
    met: String,
    meet: String,
    #[serde(default)]
    down: Option<String>,
}

impl TryFrom<RawEntry> for Entry {
    type Error = String;

    fn try_from(raw: RawEntry) -> std::result::Result<Self, String> {
        if raw.name.trim().is_empty() {
            return Err("custom dependency `name` can't be empty".to_string());
        }
        for (field, value) in [("met", &raw.met), ("meet", &raw.meet)] {
            if value.trim().is_empty() {
                return Err(format!("custom dependency \"{}\": `{field}` can't be empty", raw.name));
            }
        }

        Ok(Self {
            name: raw.name,
            met: raw.met,
            meet: raw.meet,
            down: raw.down,
        })
    }
}

/// A dependency defined by shell snippets in `dev.yaml`. Each one runs with
/// `sh -c` in `root`, with the project env and no stdin.
pub struct Custom {
    root: PathBuf,
    entry: Entry,
}

impl Custom {
    /// Creates a dependency that runs `entry`'s snippets in `root`.
    pub fn new(root: impl AsRef<Path>, entry: Entry) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            entry,
        }
    }

    fn snippet(&self, ctx: &Context, snippet: &str) -> Command {
        let mut cmd = ctx.command("sh");
        cmd.arg("-c").arg(snippet).current_dir(&self.root);
        cmd
    }
}

impl Dependency for Custom {
    fn name(&self) -> &str {
        &self.entry.name
    }

    fn desc(&self) -> &str {
        "Runs a shell check, and a fix when it fails"
    }

    fn examples(&self) -> Vec<Example> {
        vec![Example::new(
            "Ensure a file exists",
            "deps:
              custom:
                - name: Ensure file exists
                  met: test -f test-file.txt
                  meet: echo \"yo yo yo\" > test-file.txt
                  down: rm -f test-file.txt
                ",
        )]
    }

    fn meet(&self, ctx: &mut Context) -> Result<()> {
        let mut cmd = self.snippet(ctx, &self.entry.meet);
        process::run(ctx, &mut cmd)?;

        if !self.is_met(ctx)? {
            bail!("`meet` ran but `met` still fails for \"{}\"", self.entry.name);
        }
        Ok(())
    }

    fn is_met(&self, ctx: &mut Context) -> Result<bool> {
        let mut cmd = self.snippet(ctx, &self.entry.met);
        process::succeeds(ctx, &mut cmd)
    }

    fn down(&self, ctx: &mut Context) -> Result<()> {
        let Some(down) = &self.entry.down else {
            return Ok(());
        };
        let mut cmd = self.snippet(ctx, down);
        process::run(ctx, &mut cmd)
    }
}
