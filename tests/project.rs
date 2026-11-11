use dev::{Result, project::Project};

#[test]
fn from_yaml_parses_name_and_brew_dep() -> Result<()> {
    let project = Project::from_yaml(
        r#"
name: demo-project
deps:
  brew:
    - jq
    - yq
"#,
    )?;

    assert_eq!("demo-project", project.name);
    let names: Vec<&str> = project.deps.iter().map(|d| d.name()).collect();
    assert_eq!(vec!["Install brew dependencies"], names);

    let tasks: Vec<&str> = project.deps[0].tasks().iter().map(|t| t.name()).collect();
    assert_eq!(vec!["Install zerobrew", "Installing jq", "Installing yq"], tasks);
    Ok(())
}

fn project_named(name: &str) -> Result<Project> {
    Project::from_yaml(&format!("name: {name:?}\n"))
}

#[test]
fn from_yaml_accepts_lowercase_dashed_names() -> Result<()> {
    for name in ["demo", "demo-project", "api2", "my-2nd-app"] {
        assert_eq!(name, project_named(name)?.name);
    }
    Ok(())
}

#[test]
fn from_yaml_rejects_names_that_are_not_lowercase_dashed() {
    for name in [
        "",
        "Demo",
        "demo_project",
        "demo project",
        "-demo",
        "demo-",
        "demo--project",
        "démo",
    ] {
        let msg = match project_named(name) {
            Ok(_) => panic!("expected `{name}` to be rejected"),
            Err(e) => e.to_string(),
        };
        assert_eq!(
            format!("project name `{name}` must be lowercase words separated by dashes, e.g. `demo-project`"),
            msg
        );
    }
}

#[test]
fn from_yaml_rejects_unsupported_dependencies() {
    let yaml = "name: demo\ndeps:\n  brew: [jq]\n  go: 1.27\n";
    let msg = match Project::from_yaml(yaml) {
        Ok(_) => panic!("expected `go` to be rejected"),
        Err(e) => e.to_string(),
    };
    // The location should point at the `go` entry, not the start of `deps`.
    assert!(
        msg.starts_with("error: line 4 column 7: unsupported dependency `go`"),
        "unexpected error: {msg}"
    );
}

#[test]
fn from_yaml_rejects_unknown_top_level_fields() {
    let msg = match Project::from_yaml("name: demo\ndpes:\n  brew: [jq]\n") {
        Ok(_) => panic!("expected the `dpes` typo to be rejected"),
        Err(e) => e.to_string(),
    };
    assert!(msg.contains("unknown field `dpes`"), "unexpected error: {msg}");
}

#[test]
fn load_reads_project_file() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("dev.yaml");
    std::fs::write(&path, "name: demo\ndeps:\n  brew: [jq]\n")?;

    let project = Project::load(&path)?;
    assert_eq!("demo", project.name);
    assert_eq!(1, project.deps.len());
    Ok(())
}

#[test]
fn load_errors_name_the_file() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("dev.yaml");

    let missing = Project::load(&path).err().expect("a missing file should fail");
    assert_eq!(format!("failed to read {}", path.display()), missing.to_string());

    std::fs::write(&path, "name: Demo\n")?;
    let invalid = Project::load(&path).err().expect("an invalid name should fail");
    assert_eq!(format!("invalid project file {}", path.display()), invalid.to_string());
    Ok(())
}

#[test]
fn from_yaml_parses_env_and_commands() -> Result<()> {
    let project = Project::from_yaml(
        r#"
name: demo
env:
  GOFLAGS: -mod=mod
  CGO_ENABLED: "0"
commands:
  test: cargo test
  lint: |
    cargo clippy
"#,
    )?;

    let env = vec![
        ("CGO_ENABLED".to_string(), "0".to_string()),
        ("GOFLAGS".to_string(), "-mod=mod".to_string()),
    ];
    assert_eq!(env, project.env);
    assert_eq!(Some("cargo test"), project.commands.get("test").map(String::as_str));
    assert_eq!(Some("cargo clippy\n"), project.commands.get("lint").map(String::as_str));
    Ok(())
}

#[test]
fn from_yaml_defaults_env_and_commands_to_empty() -> Result<()> {
    let project = Project::from_yaml("name: demo\n")?;
    assert!(project.env.is_empty());
    assert!(project.commands.is_empty());
    Ok(())
}

#[test]
fn from_yaml_expands_braced_vars_in_env() -> Result<()> {
    let home = std::env::var("HOME")?;
    let project = Project::from_yaml("name: demo\nenv:\n  BIN: \"${HOME}/bin:$HOME\"\n")?;

    assert_eq!(vec![("BIN".to_string(), format!("{home}/bin:$HOME"))], project.env);
    Ok(())
}

#[test]
fn from_yaml_fails_on_unset_vars_in_env() {
    let err = Project::from_yaml("name: demo\nenv:\n  A: \"${DEV_TEST_UNSET_6F1C}/x\"\n")
        .err()
        .expect("an unset var should fail");
    assert_eq!(
        "env `A` references `${DEV_TEST_UNSET_6F1C}`, which isn't set",
        err.to_string()
    );
}

#[test]
fn from_yaml_fails_on_unclosed_braces_in_env() {
    let err = Project::from_yaml("name: demo\nenv:\n  A: \"${HOME/x\"\n")
        .err()
        .expect("an unclosed `${` should fail");
    assert_eq!("env `A` has a `${` with no closing `}`", err.to_string());
}
