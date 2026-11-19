use std::{
    collections::BTreeMap,
    env::VarError,
    fs::{self, File},
    path::Path,
    time::Duration,
};

use dev::{
    Result,
    project::{Project, Scope, vars},
    run::{Dirs, Plan, Target},
};
use tempfile::TempDir;

mod common;

/// A shell with `PWD` at `/shell` and `HOME` at `/h`.
fn shell_vars() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("PWD".to_string(), "/shell".to_string()),
        ("HOME".to_string(), "/h".to_string()),
    ])
}

#[test]
fn plan_for_a_project_reads_dev_yaml_here() {
    assert_eq!(
        Plan {
            file: "dev.yaml".into(),
            scope: Scope::Project,
            logs: None,
            pwd: None,
        },
        Plan::new(Target::Project)
    );
}

#[test]
fn plan_for_global_reads_the_config_home_and_logs_to_the_state_home() {
    let dirs = Dirs {
        config_home: "/c".into(),
        state_home: "/s".into(),
    };

    assert_eq!(
        Plan {
            file: "/c/dev.yaml".into(),
            scope: Scope::Global,
            logs: Some("/s/dev/log".into()),
            pwd: None,
        },
        Plan::new(Target::Global(dirs))
    );
}

#[test]
fn plan_for_new_resolves_pwd_to_the_new_dir() {
    let plan = Plan::new(Target::New("/x/demo".into()));
    assert_eq!(
        Plan {
            file: "dev.yaml".into(),
            scope: Scope::Project,
            logs: None,
            pwd: Some("/x/demo".into()),
        },
        plan
    );

    let shell = shell_vars();
    let base = vars(&shell);
    let lookup = plan.lookup(&base);
    assert_eq!(Ok("/x/demo".to_string()), lookup("PWD"));
    assert_eq!(Ok("/h".to_string()), lookup("HOME"));
}

#[test]
fn project_lookup_keeps_the_shells_pwd() {
    let plan = Plan::new(Target::Project);
    let shell = shell_vars();
    let base = vars(&shell);
    let lookup = plan.lookup(&base);

    assert_eq!(Ok("/shell".to_string()), lookup("PWD"));
    assert_eq!(Err(VarError::NotPresent), lookup("NOPE"));
}

/// A temp dir holding a dev.yaml that sets `A=a`, loaded in `scope`.
fn dev_yaml(scope: Scope) -> Result<(TempDir, Project)> {
    let dir = tempfile::tempdir()?;
    fs::write(dir.path().join("dev.yaml"), "name: demo\nenv:\n  A: a\n")?;
    let home = BTreeMap::from([("HOME".to_string(), path_str(dir.path()))]);
    let project = Project::load(dir.path().join("dev.yaml"), scope, &vars(&home))?;
    Ok((dir, project))
}

fn path_str(path: &Path) -> String {
    path.to_str().expect("temp paths are UTF-8").to_string()
}

fn a() -> Vec<(String, String)> {
    vec![("A".to_string(), "a".to_string())]
}

#[test]
fn env_applies_when_the_shell_has_not_loaded_it() -> Result<()> {
    let (dir, project) = dev_yaml(Scope::Project)?;

    let env = Plan::new(Target::Project).env(&project, &BTreeMap::new(), dir.path())?;

    assert_eq!(a(), env);
    Ok(())
}

#[test]
fn env_is_skipped_when_the_shell_loaded_this_dev_yaml() -> Result<()> {
    let (dir, project) = dev_yaml(Scope::Project)?;
    let state = common::shell_state(&dir.path().join("dev.yaml"), false)?;
    let shell = BTreeMap::from([("__DEV_ENV".to_string(), state)]);

    let env = Plan::new(Target::Project).env(&project, &shell, dir.path())?;

    assert!(env.is_empty(), "{env:?}");
    Ok(())
}

#[test]
fn env_applies_again_after_dev_yaml_changes() -> Result<()> {
    let (dir, project) = dev_yaml(Scope::Project)?;
    let file = dir.path().join("dev.yaml");
    let state = common::shell_state(&file, false)?;
    let shell = BTreeMap::from([("__DEV_ENV".to_string(), state)]);
    // An edit, with its mtime set rather than waiting on the clock.
    let later = fs::metadata(&file)?.modified()? + Duration::from_secs(1);
    File::options().write(true).open(&file)?.set_modified(later)?;

    let env = Plan::new(Target::Project).env(&project, &shell, dir.path())?;

    assert_eq!(a(), env);
    Ok(())
}

#[test]
fn env_is_skipped_for_the_loaded_global_file() -> Result<()> {
    let (dir, project) = dev_yaml(Scope::Global)?;
    let state = common::shell_state(&dir.path().join("dev.yaml"), true)?;
    let shell = BTreeMap::from([("__DEV_ENV".to_string(), state)]);
    let plan = Plan::new(Target::Global(Dirs {
        config_home: dir.path().into(),
        state_home: dir.path().into(),
    }));
    let elsewhere = tempfile::tempdir()?;

    let env = plan.env(&project, &shell, elsewhere.path())?;

    assert!(env.is_empty(), "{env:?}");
    Ok(())
}
