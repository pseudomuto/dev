mod common;

use std::collections::BTreeMap;

use common::capturing_ctx;
use dev::{
    Result,
    commands::Script,
    deps::{Dependency, mise::ToolSpec, packages::Mode},
    project::{Project, Scope, vars},
};

/// A project dir that doesn't exist, so building deps finds no packages.
const ROOT: &str = "/nonexistent/dev-test/demo";

/// Parses `yaml` as a project's dev.yaml in [`ROOT`], with no env vars set.
fn parse(yaml: &str) -> Result<Project> {
    Project::from_yaml(yaml, ROOT, Scope::Project, &vars(&BTreeMap::new()))
}

/// An env with only `HOME` set, to `dir`.
fn home(dir: &std::path::Path) -> BTreeMap<String, String> {
    BTreeMap::from([(
        "HOME".to_string(),
        dir.to_str().expect("temp paths are UTF-8").to_string(),
    )])
}

/// The names of `deps`.
fn names(deps: &[Box<dyn Dependency>]) -> Vec<&str> {
    deps.iter().map(|d| d.name()).collect()
}

/// The names of the tasks of `dep`.
fn tasks(dep: &dyn Dependency) -> Vec<&str> {
    dep.tasks().iter().map(|t| t.name()).collect()
}

#[test]
fn from_yaml_parses_name_and_brew_dep() -> Result<()> {
    let project = parse(
        r#"
name: demo-project
deps:
  brew:
    - jq
    - yq
"#,
    )?;

    assert_eq!("demo-project", project.name);
    let deps = project.deps(Mode::Local)?;
    let names = names(&deps);
    assert_eq!(vec!["Install brew dependencies"], names);

    let tasks = tasks(deps[0].as_ref());
    assert_eq!(vec!["Install zerobrew", "Installing jq", "Installing yq"], tasks);
    Ok(())
}

#[test]
fn from_yaml_parses_other_deps_as_one_tools_dep_where_the_first_appears() -> Result<()> {
    let project = parse(
        r#"
name: demo
deps:
  node: lts
  brew: [jq]
  python: 3.10
  "go:golang.org/x/tools/gopls": latest
"#,
    )?;

    let deps = project.deps(Mode::Local)?;
    let names = names(&deps);
    assert_eq!(vec!["Install tools", "Install brew dependencies"], names);

    let tasks = tasks(deps[0].as_ref());
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
    parse(&format!("name: {name:?}\n"))
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
fn from_yaml_rejects_a_malformed_custom_value() {
    let msg = match parse("name: demo\ndeps:\n  brew: [jq]\n  custom: nope\n") {
        Ok(_) => panic!("expected `custom: nope` to be rejected"),
        Err(e) => e.to_string(),
    };
    // The location should point at the `custom` entry, not the start of `deps`.
    assert!(msg.starts_with("error: line 4 column 11:"), "unexpected error: {msg}");
}

#[test]
fn from_yaml_parses_custom_deps_in_order() -> Result<()> {
    let project = parse(
        r#"
name: demo
deps:
  brew: [jq]
  custom:
    - { name: A, met: "true", meet: "true" }
    - { name: B, met: "true", meet: "true", down: "true" }
  node: lts
"#,
    )?;

    let deps = project.deps(Mode::Local)?;
    assert_eq!(
        vec!["Install brew dependencies", "A", "B", "Install tools"],
        names(&deps)
    );
    Ok(())
}

#[test]
fn from_yaml_allows_empty_custom() -> Result<()> {
    let project = parse("name: demo\ndeps:\n  custom: []\n")?;
    assert!(project.deps(Mode::Local)?.is_empty());
    Ok(())
}

#[test]
fn from_yaml_rejects_unknown_custom_fields() {
    let yaml = "name: demo\ndeps:\n  custom:\n    - { name: A, met: x, meet: y, unmet: z }\n";
    let msg = match parse(yaml) {
        Ok(_) => panic!("expected `unmet` to be rejected"),
        Err(e) => e.to_string(),
    };
    assert!(msg.starts_with("error: line "), "unexpected error: {msg}");
    assert!(msg.contains("unknown field `unmet`"), "unexpected error: {msg}");
}

#[test]
fn from_yaml_rejects_tools_without_a_version_string() {
    let msg = match parse("name: demo\ndeps:\n  node: [22, 24]\n") {
        Ok(_) => panic!("expected a list of versions to be rejected"),
        Err(e) => e.to_string(),
    };
    assert!(
        msg.starts_with("error: line 3 column 3: invalid type: sequence, expected a version like `lts`"),
        "unexpected error: {msg}"
    );
}

#[test]
fn from_yaml_rejects_unknown_top_level_fields() {
    let msg = match parse("name: demo\ndpes:\n  brew: [jq]\n") {
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

    let project = Project::load(&path, Scope::Project, &vars(&BTreeMap::new()))?;
    assert_eq!("demo", project.name);
    assert_eq!(1, project.deps(Mode::Local)?.len());
    Ok(())
}

#[test]
fn load_errors_name_the_file() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("dev.yaml");

    let missing = Project::load(&path, Scope::Project, &vars(&BTreeMap::new()))
        .err()
        .expect("a missing file should fail");
    assert_eq!(format!("failed to read {}", path.display()), missing.to_string());

    std::fs::write(&path, "name: Demo\n")?;
    let invalid = Project::load(&path, Scope::Project, &vars(&BTreeMap::new()))
        .err()
        .expect("an invalid name should fail");
    assert_eq!(format!("invalid project file {}", path.display()), invalid.to_string());
    Ok(())
}

#[test]
fn from_yaml_parses_env_and_commands() -> Result<()> {
    let project = parse(
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
    let project = parse("name: demo\n")?;
    assert!(project.env.is_empty());
    assert!(project.commands.is_empty());
    Ok(())
}

#[test]
fn from_yaml_expands_braced_vars_in_env() -> Result<()> {
    let env = BTreeMap::from([("HOME".to_string(), "/home/me".to_string())]);
    let yaml = "name: demo\nenv:\n  BIN: \"${HOME}/bin:$HOME\"\n";
    let project = Project::from_yaml(yaml, ROOT, Scope::Project, &vars(&env))?;

    assert_eq!(vec![("BIN".to_string(), "/home/me/bin:$HOME".to_string())], project.env);
    Ok(())
}

#[test]
fn from_yaml_fails_on_unset_vars_in_env() {
    let err = parse("name: demo\nenv:\n  A: \"${DEV_TEST_UNSET_6F1C}/x\"\n")
        .err()
        .expect("an unset var should fail");
    assert_eq!(
        "env `A` references `${DEV_TEST_UNSET_6F1C}`, which isn't set",
        err.to_string()
    );
}

#[test]
fn from_yaml_fails_on_unclosed_braces_in_env() {
    let err = parse("name: demo\nenv:\n  A: \"${HOME/x\"\n")
        .err()
        .expect("an unclosed `${` should fail");
    assert_eq!("env `A` has a `${` with no closing `}`", err.to_string());
}

#[test]
fn from_yaml_expands_from_the_given_lookup() -> Result<()> {
    let lookup = |var: &str| match var {
        "ROOT" => Ok("/proj".to_string()),
        _ => Err(std::env::VarError::NotPresent),
    };
    let project = Project::from_yaml(
        "name: demo\nenv:\n  BIN: \"${ROOT}/bin\"\n",
        ROOT,
        Scope::Project,
        &lookup,
    )?;
    assert_eq!(vec![("BIN".to_string(), "/proj/bin".to_string())], project.env);

    let err = Project::from_yaml("name: demo\nenv:\n  A: \"${HOME}\"\n", ROOT, Scope::Project, &lookup)
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
    let mut project = Project::from_yaml(
        &format!("name: demo\nenv:\n  A: {value:?}\n"),
        ROOT,
        Scope::Project,
        &lookup,
    )?;
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
    let project = parse(
        r#"
name: demo
deps:
  rust:
    - rust-toolchain.toml
    - nightly: [rustfmt]
    - "1.80"
"#,
    )?;

    let deps = project.deps(Mode::Local)?;
    let names = names(&deps);
    assert_eq!(vec!["Install Rust"], names, "not a mise tool");
    let tasks = tasks(deps[0].as_ref());
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
        let err = parse(&yaml).err().unwrap_or_else(|| panic!("should fail:\n{yaml}"));
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
    let project = Project::from_yaml(yaml, dir.path(), Scope::Project, &vars(&BTreeMap::new()))?;
    Ok(names(&project.deps(Mode::Local)?)
        .into_iter()
        .map(String::from)
        .collect())
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

#[test]
fn missing_package_files_fail_building_deps_not_loading() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("dev.yaml");
    std::fs::write(&path, "name: demo\nenv:\n  A: a\ndeps:\n  packages: [node]\n")?;

    let project = Project::load(&path, Scope::Project, &vars(&BTreeMap::new()))?;
    assert_eq!(vec![("A".to_string(), "a".to_string())], project.env);

    let err = project.deps(Mode::Local).err().expect("node has no package.json here");
    assert!(
        err.to_string().contains("has none of its files (package.json)"),
        "{err}"
    );
    Ok(())
}

#[test]
fn load_global_expands_pwd_to_home_wherever_it_runs() -> Result<()> {
    let home = tempfile::tempdir()?;
    let path = home.path().join("dev.yaml");
    std::fs::write(&path, "env:\n  BIN: \"${PWD}/bin\"\n")?;
    let mut env = self::home(home.path());
    env.insert("PWD".to_string(), "/somewhere/else".to_string());

    let project = Project::load(&path, Scope::Global, &vars(&env))?;
    assert_eq!(
        vec![("BIN".to_string(), format!("{}/bin", home.path().display()))],
        project.env
    );
    Ok(())
}

#[test]
fn load_global_needs_home_for_its_deps() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("dev.yaml");
    std::fs::write(&path, "deps:\n  node: lts\n")?;

    let err = Project::load(&path, Scope::Global, &vars(&BTreeMap::new()))
        .err()
        .expect("the global deps run in HOME");
    assert_eq!(
        format!("HOME is not set, so the deps in {} have nowhere to run", path.display()),
        err.to_string()
    );
    Ok(())
}

#[test]
fn load_global_runs_rust_in_home_not_the_current_dir() -> Result<()> {
    let home = tempfile::tempdir()?;
    let path = home.path().join("dev.yaml");
    std::fs::write(&path, "deps:\n  rust: [rust-toolchain.toml]\n")?;
    // Unreadable as a toolchain, so checking it fails before rustup runs.
    let toolchain = home.path().join("rust-toolchain.toml");
    std::fs::write(&toolchain, "not toml")?;

    let project = Project::load(&path, Scope::Global, &vars(&self::home(home.path())))?;
    let deps = project.deps(Mode::Local)?;
    let check = &deps[0].tasks()[1];
    assert_eq!("Installing the toolchain in rust-toolchain.toml", check.name());

    // No rustup to find, in case the toolchain is read from somewhere else.
    let empty = tempfile::tempdir()?;
    let mut ctx = dev::Context {
        env: vec![("PATH".to_string(), empty.path().display().to_string())],
        ..dev::Context::default()
    };
    let err = check.is_applied(&mut ctx).expect_err("the toolchain file is invalid");
    assert_eq!(
        format!("can't tell which toolchain {} names", toolchain.display()),
        err.to_string()
    );
    Ok(())
}

#[test]
fn from_yaml_points_mise_at_the_project_config_when_there_are_tools() -> Result<()> {
    let project = parse("name: demo\nenv:\n  A: a\n  Z: z\ndeps:\n  node: lts\n")?;
    assert_eq!(
        vec![
            ("A".to_string(), "a".to_string()),
            ("MISE_DEFAULT_CONFIG_FILENAME".to_string(), ".dev/mise.toml".to_string()),
            ("Z".to_string(), "z".to_string()),
        ],
        project.env
    );

    let project = parse("name: demo\ndeps:\n  brew: [jq]\n")?;
    assert!(project.env.is_empty(), "no tools, so mise needs no project config");
    Ok(())
}

#[test]
fn from_yaml_rejects_the_mise_config_var_in_env() {
    let err = parse("name: demo\nenv:\n  MISE_DEFAULT_CONFIG_FILENAME: x.toml\n")
        .err()
        .expect("dev owns that var");
    assert_eq!(
        "env `MISE_DEFAULT_CONFIG_FILENAME` is set by dev, so mise finds a project's tools; remove it from dev.yaml",
        err.root_cause().to_string()
    );
}

#[test]
fn load_writes_tools_to_the_project_mise_config() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("dev.yaml");
    std::fs::write(&path, "name: demo\ndeps:\n  node: lts\n")?;

    let project = Project::load(&path, Scope::Project, &vars(&BTreeMap::new()))?;
    // Guard: a project whose tools are still global would write to the real
    // ~/.config below.
    assert!(
        project.env.iter().any(|(k, _)| k == "MISE_DEFAULT_CONFIG_FILENAME"),
        "tools should be in project scope"
    );
    let deps = project.deps(Mode::Local)?;
    let config = deps[0]
        .tasks()
        .iter()
        .find(|t| t.name() == "Configure tools")
        .expect("a tools dep configures mise");
    config.apply(&mut dev::Context::default())?;

    let written = std::fs::read_to_string(dir.path().join(".dev/mise.toml"))?;
    assert!(written.ends_with("[tools]\n\"node\" = \"lts\"\n"), "got {written:?}");
    Ok(())
}

#[test]
fn load_requires_a_project_name() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("dev.yaml");
    std::fs::write(&path, "deps:\n  brew: [jq]\n")?;

    let err = Project::load(&path, Scope::Project, &vars(&BTreeMap::new()))
        .err()
        .expect("a project needs a name");
    assert_eq!(
        "`name` is required, e.g. `name: demo-project`",
        err.root_cause().to_string()
    );
    Ok(())
}

#[test]
fn load_global_needs_no_name_and_detects_no_packages() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("dev.yaml");
    std::fs::write(dir.path().join("package.json"), "{}")?;
    std::fs::write(&path, "env:\n  A: a\ndeps:\n  node: lts\n")?;

    let project = Project::load(&path, Scope::Global, &vars(&home(dir.path())))?;
    assert_eq!("global", project.name);
    assert_eq!(
        vec![("A".to_string(), "a".to_string())],
        project.env,
        "global tools need no var"
    );
    let deps = project.deps(Mode::Local)?;
    let names = names(&deps);
    assert_eq!(vec!["Install tools"], names);
    Ok(())
}

#[test]
fn load_global_still_checks_a_name_when_given() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("dev.yaml");
    std::fs::write(&path, "name: Mine\n")?;

    let err = Project::load(&path, Scope::Global, &vars(&home(dir.path())))
        .err()
        .expect("an invalid name should fail");
    assert_eq!(format!("invalid project file {}", path.display()), err.to_string());
    Ok(())
}

#[test]
fn from_yaml_parses_tool_options_keeping_values_as_written() -> Result<()> {
    let yaml = "name: demo\ndeps:\n  aws: { version: 2.30, symlink_bins: true, level: 3.10 }\n  python: 3.10\n";
    let project = parse(yaml)?;
    let deps = project.deps(Mode::Local)?;
    let tasks = tasks(deps[0].as_ref());
    assert_eq!(
        vec![
            "Install mise",
            "Configure tools",
            "Installing aws@2.30",
            "Installing python@3.10"
        ],
        tasks
    );

    assert_eq!(
        vec![
            ToolSpec::new("aws", "2.30")
                .with_option("symlink_bins", "true")
                .with_option("level", "3.10"),
            ToolSpec::new("python", "3.10"),
        ],
        project.tools
    );
    Ok(())
}

fn tools_error(deps: &str) -> String {
    match parse(&format!("name: demo\ndeps:\n{deps}")) {
        Ok(_) => panic!("expected {deps:?} to be rejected"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn from_yaml_requires_a_version_with_tool_options() {
    let msg = tools_error("  aws: { symlink_bins: true }\n");
    assert!(
        msg.contains("tool `aws` needs a `version` next to its options"),
        "unexpected error: {msg}"
    );
}

#[test]
fn from_yaml_rejects_tool_options_that_arent_plain_values() {
    let msg = tools_error("  aws: { version: \"1\", platforms: { linux: x } }\n");
    assert!(
        msg.contains("option `platforms` of tool `aws` must be a string, number, or bool"),
        "unexpected error: {msg}"
    );
}

#[test]
fn from_yaml_rejects_tool_options_mise_cant_take_on_install() {
    for deps in [
        "  aws: { version: \"1\", exe: \"a,b\" }\n",
        "  aws: { version: \"1\", \"a=b\": x }\n",
        "  aws: { version: \"1\", exe: \"[x]\" }\n",
    ] {
        let msg = tools_error(deps);
        assert!(
            msg.contains("options of tool `aws` can't contain `[`, `]`, `,`, or `=`"),
            "unexpected error for {deps:?}: {msg}"
        );
    }
}

#[test]
fn from_yaml_reads_plain_versions_back_exactly_after_unicode_comments_and_aliases() -> Result<()> {
    let yaml = "name: demo\nenv:\n  GREETING: \"héllo wörld 👋\"\ndeps:\n  python: 3.10 # the one we test on\n  ruby: &v 3.40\n  aws: { version: *v, level: 1.0 }\n";
    assert_eq!(
        vec![
            ToolSpec::new("python", "3.10"),
            ToolSpec::new("ruby", "3.40"),
            ToolSpec::new("aws", "3.40").with_option("level", "1.0"),
        ],
        parse(yaml)?.tools
    );
    Ok(())
}

#[test]
fn load_global_runs_custom_deps_in_home() -> Result<()> {
    let home = tempfile::tempdir()?;
    let path = home.path().join("dev.yaml");
    std::fs::write(
        &path,
        "deps:\n  custom:\n    - { name: A, met: test -f f, meet: touch f }\n",
    )?;

    let project = Project::load(&path, Scope::Global, &vars(&self::home(home.path())))?;
    let deps = project.deps(Mode::Local)?;
    deps[0].meet(&mut capturing_ctx().0)?;
    assert!(home.path().join("f").exists());
    Ok(())
}
