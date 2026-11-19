//! How a `dev up`, `down`, `new` or project command reads its dev.yaml,
//! decided before anything runs.

use std::{
    collections::BTreeMap,
    env::VarError,
    path::{Path, PathBuf},
};

use crate::{
    Result,
    project::{Lookup, Project, Scope},
    shell,
};

/// The project file `dev` reads.
pub const PROJECT_FILE: &str = "dev.yaml";

/// Where the user's global dev.yaml and its logs live.
pub struct Dirs {
    pub config_home: PathBuf,
    pub state_home: PathBuf,
}

impl Dirs {
    /// `$XDG_CONFIG_HOME` and `$XDG_STATE_HOME`, or their defaults under
    /// `$HOME`.
    pub fn current() -> Result<Self> {
        Ok(Self {
            config_home: shell::config_home()?,
            state_home: shell::state_home()?,
        })
    }
}

/// Which dev.yaml a run is for.
pub enum Target {
    /// ./dev.yaml.
    Project,
    /// The user's global dev.yaml.
    Global(Dirs),
    /// ./dev.yaml of the project `dev new` just made in this dir (absolute),
    /// once the process has changed to it.
    New(PathBuf),
}

/// How a run reads its dev.yaml.
#[derive(Debug, PartialEq)]
pub struct Plan {
    /// Relative to the cwd for a project, so errors name it as the user
    /// knows it; absolute for the global file.
    pub file: PathBuf,
    pub scope: Scope,
    /// Where run logs go. `None` is the project's `.dev/log`.
    pub logs: Option<PathBuf>,
    /// What `${PWD}` resolves to. `None` leaves it to the caller's lookup.
    pub pwd: Option<String>,
}

impl Plan {
    pub fn new(target: Target) -> Self {
        match target {
            Target::Project => Self {
                file: PROJECT_FILE.into(),
                scope: Scope::Project,
                logs: None,
                pwd: None,
            },
            Target::Global(dirs) => Self {
                file: dirs.config_home.join(PROJECT_FILE),
                scope: Scope::Global,
                logs: Some(dirs.state_home.join("dev/log")),
                pwd: None,
            },
            // The shell's `$PWD` is still the parent dir, but `${PWD}` in
            // dev.yaml must be the new project's.
            Target::New(dir) => Self {
                file: PROJECT_FILE.into(),
                scope: Scope::Project,
                logs: None,
                pwd: Some(dir.to_string_lossy().into_owned()),
            },
        }
    }

    /// `base`, with `PWD` replaced when this plan sets it.
    pub fn lookup<'a>(&'a self, base: &'a Lookup<'a>) -> impl Fn(&str) -> std::result::Result<String, VarError> + 'a {
        move |var| match (var, &self.pwd) {
            ("PWD", Some(pwd)) => Ok(pwd.clone()),
            _ => base(var),
        }
    }

    /// The dev.yaml this plan is for, with `${VAR}` resolved by `base`
    /// (see [`Plan::lookup`]).
    pub fn load(&self, base: &Lookup<'_>) -> Result<Project> {
        Project::load(&self.file, self.scope, &self.lookup(base))
    }

    /// `project`'s env, or none when the shell with `vars` already loaded
    /// this dev.yaml (in `cwd`) as it is on disk now. Applying it again
    /// would apply it twice, e.g. `${PWD}/bin` twice on PATH.
    pub fn env(&self, project: &Project, vars: &BTreeMap<String, String>, cwd: &Path) -> Result<Vec<(String, String)>> {
        if shell::env::applied(vars, &cwd.join(&self.file))? {
            Ok(Vec::new())
        } else {
            Ok(project.env.clone())
        }
    }
}
