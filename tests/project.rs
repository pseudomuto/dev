use dev::{Result, commands::Script, project::Project};

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

#[test]
fn from_yaml_parses_other_deps_as_one_tools_dep_where_the_first_appears() -> Result<()> {
    let project = Project::from_yaml(
        r#"
name: demo
deps:
  node: lts
  brew: [jq]
  python: 3.10
  "go:golang.org/x/tools/gopls": latest
"#,
    )?;

    let names: Vec<&str> = project.deps.iter().map(|d| d.name()).collect();
    assert_eq!(vec!["Install tools", "Install brew dependencies"], names);

    let tasks: Vec<&str> = project.deps[0].tasks().iter().map(|t| t.name()).collect();
    assert_eq!(
        vec![
            "Install mise",
            "Configure tools",
            "Installing node@lts",
            "Installing python@3.10",
            "Installing go:golang.org/x/tools/gopls@latest",
        ],
        tasks
    );
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
fn from_yaml_rejects_reserved_dependencies() {
    let yaml = "name: demo\ndeps:\n  brew: [jq]\n  custom: []\n";
    let msg = match Project::from_yaml(yaml) {
        Ok(_) => panic!("expected `custom` to be rejected"),
        Err(e) => e.to_string(),
    };
    // The location should point at the `custom` entry, not the start of `deps`.
    assert!(
        msg.starts_with("error: line 4 column 11: unsupported dependency `custom`"),
        "unexpected error: {msg}"
    );
}

#[test]
fn from_yaml_rejects_tools_without_a_version_string() {
    let msg = match Project::from_yaml("name: demo\ndeps:\n  node: [22, 24]\n") {
        Ok(_) => panic!("expected a list of versions to be rejected"),
        Err(e) => e.to_string(),
    };
    assert!(msg.starts_with("error: line 3 column 9: "), "unexpected error: {msg}");
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
    let script = |name: &str| project.commands.get(name).map(|cmd| &cmd.script);
    assert_eq!(Some(&Script::Run("cargo test".to_string())), script("test"));
    assert_eq!(Some(&Script::Run("cargo clippy\n".to_string())), script("lint"));
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

#[test]
fn from_yaml_with_expands_from_the_given_lookup() -> Result<()> {
    let lookup = |var: &str| match var {
        "ROOT" => Ok("/proj".to_string()),
        _ => Err(std::env::VarError::NotPresent),
    };
    let project = Project::from_yaml_with("name: demo\nenv:\n  BIN: \"${ROOT}/bin\"\n", &lookup)?;
    assert_eq!(vec![("BIN".to_string(), "/proj/bin".to_string())], project.env);

    let err = Project::from_yaml_with("name: demo\nenv:\n  A: \"${HOME}\"\n", &lookup)
        .err()
        .expect("vars missing from the lookup should fail, even if the process has them");
    assert_eq!("env `A` references `${HOME}`, which isn't set", err.to_string());
    Ok(())
}

/// Expands `value` as env var `A`, with only `vars` set.
fn expand_with(value: &str, vars: &[(&str, &str)]) -> Result<String> {
    let lookup = |var: &str| {
        vars.iter()
            .find(|(name, _)| *name == var)
            .map(|(_, val)| val.to_string())
            .ok_or(std::env::VarError::NotPresent)
    };
    let mut project = Project::from_yaml_with(&format!("name: demo\nenv:\n  A: {value:?}\n"), &lookup)?;
    Ok(project.env.remove(0).1)
}

#[test]
fn from_yaml_uses_the_default_when_the_var_is_unset() -> Result<()> {
    assert_eq!(
        "postgres://localhost/app",
        expand_with("${DB:-postgres://localhost/app}", &[])?
    );
    Ok(())
}

#[test]
fn from_yaml_uses_the_var_over_the_default_when_set() -> Result<()> {
    let vars = [("DB", "postgres://ci/app")];
    assert_eq!(
        "postgres://ci/app?x=1",
        expand_with("${DB:-postgres://localhost/app}?x=1", &vars)?
    );
    Ok(())
}

#[test]
fn from_yaml_uses_the_default_when_the_var_is_empty() -> Result<()> {
    assert_eq!("false", expand_with("${CI:-false}", &[("CI", "")])?);
    assert_eq!("", expand_with("${CI:-}", &[])?);
    Ok(())
}

#[test]
fn from_yaml_fails_on_vars_inside_a_default() {
    let err = expand_with("${A:-${B}}", &[("B", "b")]).expect_err("a `${` inside a default should fail");
    assert_eq!(
        "env `A` has a `${` inside the default for `${A}`, which isn't supported",
        err.to_string()
    );
}

#[test]
fn from_yaml_parses_rust_toolchains_as_their_own_dependency() -> Result<()> {
    let project = Project::from_yaml(
        r#"
name: demo
deps:
  rust:
    - rust-toolchain.toml
    - nightly: [rustfmt]
    - "1.80"
"#,
    )?;

    let names: Vec<&str> = project.deps.iter().map(|d| d.name()).collect();
    assert_eq!(vec!["Install Rust"], names, "not a mise tool");
    let tasks: Vec<&str> = project.deps[0].tasks().iter().map(|t| t.name()).collect();
    assert_eq!(
        vec![
            "Install rustup",
            "Installing the toolchain in rust-toolchain.toml",
            "Installing Rust nightly",
            "Installing Rust 1.80",
        ],
        tasks
    );
    Ok(())
}

#[test]
fn from_yaml_rejects_unclear_rust_entries() {
    let cases = [
        ("- 1.80", "quoted, so YAML keeps it as text"),
        (
            "- nightly: [rustfmt]\n      beta: [clippy]",
            "put `beta` in its own entry",
        ),
        (
            "- rust-toolchain.toml: [clippy]",
            "`rust-toolchain.toml` takes no components",
        ),
    ];
    for (entries, want) in cases {
        let yaml = format!("name: demo\ndeps:\n  rust:\n    {entries}\n");
        let err = Project::from_yaml(&yaml)
            .err()
            .unwrap_or_else(|| panic!("should fail:\n{yaml}"));
        let err = format!("{err:#}");
        assert!(err.contains(want), "want `{want}` for:\n{yaml}\ngot: {err}");
    }
}

/// Parses `yaml` as the dev.yaml of a dir holding `files` (each empty), and
/// returns its deps' names.
fn deps_in(yaml: &str, files: &[&str]) -> Result<Vec<String>> {
    let dir = tempfile::tempdir()?;
    for file in files {
        std::fs::write(dir.path().join(file), "")?;
    }
    let project = Project::from_yaml_at(yaml, dir.path(), &|var| std::env::var(var))?;
    Ok(project.deps.iter().map(|d| d.name().to_string()).collect())
}

#[test]
fn packages_found_in_the_project_dir_are_installed_last() -> Result<()> {
    let names = deps_in("name: demo\ndeps:\n  brew: [jq]\n  node: lts\n", &["package.json"])?;

    assert_eq!(
        names,
        ["Install brew dependencies", "Install tools", "Install project packages"]
    );
    Ok(())
}

#[test]
fn no_packages_dep_when_turned_off_or_none_are_found() -> Result<()> {
    let off = deps_in(
        "name: demo\ndeps:\n  brew: [jq]\n  packages: false\n",
        &["package.json"],
    )?;
    assert_eq!(off, ["Install brew dependencies"]);

    let none = deps_in("name: demo\ndeps:\n  brew: [jq]\n", &[])?;
    assert_eq!(none, ["Install brew dependencies"]);
    Ok(())
}

#[test]
fn listed_packages_run_where_they_appear() -> Result<()> {
    let files = &["package.json"];

    let before = deps_in("name: demo\ndeps:\n  packages: [node]\n  node: lts\n", files)?;
    assert_eq!(before, ["Install project packages", "Install tools"]);

    let after = deps_in("name: demo\ndeps:\n  node: lts\n  packages: [node]\n", files)?;
    assert_eq!(after, ["Install tools", "Install project packages"]);

    let later = deps_in(
        "name: demo\ndeps:\n  node: lts\n  brew: [jq]\n  packages: [node]\n",
        files,
    )?;
    assert_eq!(
        later,
        ["Install tools", "Install brew dependencies", "Install project packages"]
    );
    Ok(())
}

#[test]
fn rejects_bad_packages_settings() -> Result<()> {
    for (value, expected) in [
        ("true", "`packages: true` is the default"),
        ("[java]", "java"),
        ("[go]", "has none of its files (go.mod)"),
    ] {
        let yaml = format!("name: demo\ndeps:\n  packages: {value}\n");
        let err = deps_in(&yaml, &["package.json"]).expect_err(value);
        let msg = format!("{err:?}");
        assert!(msg.contains(expected), "{value}: {msg}");
    }
    Ok(())
}
