use std::collections::BTreeMap;

use dev::{
    Result,
    commands::{Arg, CmdRef, Flag, FlagKind, ProjectCommand, Script, Step, dep_order},
    project::{Project, Scope, vars},
};

/// Parses `yaml` as a project in the current dir, with no env vars set.
fn parse(yaml: &str) -> Result<Project> {
    Project::from_yaml(yaml, ".", Scope::Project, &vars(&BTreeMap::new()))
}

/// The command `name` from a dev.yaml whose `commands` section is `yaml`.
fn command(yaml: &str, name: &str) -> Result<ProjectCommand> {
    let mut project = parse(&format!("name: demo\ncommands:\n{yaml}"))?;
    Ok(project.commands.remove(name).expect("the command should be parsed"))
}

#[test]
fn long_form_parses_desc_flags_args_and_run() -> Result<()> {
    let cmd = command(
        r#"
  deploy:
    desc: Deploy an app
    flags:
      env: { short: e, default: staging, desc: Target environment }
      dry-run: { type: bool }
    args:
      - name: target
        desc: App to deploy
      - name: region
        required: false
    run: echo hi
"#,
        "deploy",
    )?;

    let expected = ProjectCommand {
        desc: Some("Deploy an app".to_string()),
        script: Some(Script::Run("echo hi".to_string())),
        depends: Vec::new(),
        flags: vec![
            Flag {
                name: "dry-run".to_string(),
                short: None,
                desc: None,
                default: None,
                kind: FlagKind::Bool,
            },
            Flag {
                name: "env".to_string(),
                short: Some('e'),
                desc: Some("Target environment".to_string()),
                default: Some("staging".to_string()),
                kind: FlagKind::String,
            },
        ],
        args: vec![
            Arg {
                name: "target".to_string(),
                desc: Some("App to deploy".to_string()),
                required: true,
                default: None,
            },
            Arg {
                name: "region".to_string(),
                desc: None,
                required: false,
                default: None,
            },
        ],
    };
    assert_eq!(expected, cmd);
    Ok(())
}

#[test]
fn long_form_parses_file() -> Result<()> {
    let cmd = command(
        "  release:\n    desc: Cut a release\n    file: scripts/release.sh\n",
        "release",
    )?;

    let expected = ProjectCommand {
        desc: Some("Cut a release".to_string()),
        script: Some(Script::File("scripts/release.sh".into())),
        depends: Vec::new(),
        flags: Vec::new(),
        args: Vec::new(),
    };
    assert_eq!(expected, cmd);
    Ok(())
}

#[test]
fn long_form_rejects_invalid_specs() {
    let cases = [
        ("run: a\n    file: b", "takes `run` or `file`, not both"),
        ("desc: nothing to run", "needs `run`, `file`, or `depends`"),
        ("depends: []", "needs `run`, `file`, or `depends`"),
        (
            "depends: [x]\n    flags:\n      fast: {}",
            "a command with only `depends` can't take flags or args",
        ),
        (
            "depends: [x]\n    args:\n      - name: target",
            "a command with only `depends` can't take flags or args",
        ),
        ("run: a\n    nope: 1", "unknown field `nope`"),
        (
            "run: a\n    flags:\n      Dry_Run: {}",
            "flag `Dry_Run` must be lowercase words",
        ),
        (
            "run: a\n    flags:\n      1st: {}",
            "flag `1st` must be lowercase words",
        ),
        (
            "run: a\n    flags:\n      path: {}",
            "flag `path` would be passed as $PATH, which dev reserves",
        ),
        (
            "run: a\n    flags:\n      help: {}",
            "flag `help` would be passed as $HELP, which dev reserves",
        ),
        (
            "run: a\n    flags:\n      host: { short: h }",
            "flag `host` can't use short `h`",
        ),
        (
            "run: a\n    flags:\n      all: { short: a }\n      any: { short: a }",
            "flag `any` has short `a`, which another flag already uses",
        ),
        (
            "run: a\n    flags:\n      yes: { type: bool, default: x }",
            "flag `yes` is a bool, so it can't have a default",
        ),
        (
            "run: a\n    flags:\n      target: {}\n    args:\n      - name: target",
            "arg `target` would be passed as $TARGET, which another flag or arg already is",
        ),
        (
            "run: a\n    args:\n      - name: x\n        required: true\n        default: y",
            "arg `x` can't be required and have a default",
        ),
        (
            "run: a\n    args:\n      - name: x\n        required: false\n      - name: y",
            "arg `y` is required, so it can't come after optional arg `x`",
        ),
    ];

    for (spec, want) in cases {
        let yaml = format!("name: demo\ncommands:\n  c:\n    {spec}\n");
        let err = parse(&yaml).err().unwrap_or_else(|| panic!("should fail:\n{yaml}"));
        let err = err.to_string();
        assert!(err.contains(want), "want `{want}` in error for:\n{yaml}\ngot: {err}");
    }
}

#[test]
fn long_form_parses_depends() -> Result<()> {
    let cmd = command(
        "  ci:\n    depends: [lint, test]\n    run: echo done\n  lint: x\n  test: x\n",
        "ci",
    )?;

    assert_eq!(vec!["lint".to_string(), "test".to_string()], cmd.depends);
    assert_eq!(Some(Script::Run("echo done".to_string())), cmd.script);
    Ok(())
}

#[test]
fn depends_alone_needs_no_script() -> Result<()> {
    let cmd = command("  ci:\n    desc: CI\n    depends: [lint]\n  lint: cargo clippy\n", "ci")?;

    assert_eq!(None, cmd.script);
    assert_eq!(vec!["lint".to_string()], cmd.depends);
    Ok(())
}

#[test]
fn lists_become_one_set_e_script() -> Result<()> {
    let yaml = "  short:\n    - a\n    - |\n      b\n      c\n  long:\n    run: [a, b]\n";
    for (name, want) in [("short", "set -e\na\nb\nc\n"), ("long", "set -e\na\nb")] {
        assert_eq!(
            Some(Script::Run(want.to_string())),
            command(yaml, name)?.script,
            "command `{name}`"
        );
    }
    Ok(())
}

#[test]
fn lists_reject_empty_and_nested_items() {
    let cases = [
        ("c: []", "a list of commands can't be empty"),
        ("c: [a, '']", "command 2 in the list can't be empty"),
        (
            "c: [a, [b]]",
            "invalid type: sequence, expected a shell snippet, or `{ cmd: <name> }`",
        ),
        ("c:\n    run: []", "a list of commands can't be empty"),
        (
            "c:\n    run: [a, [b]]",
            "invalid type: sequence, expected a shell snippet, or `{ cmd: <name> }`",
        ),
    ];
    for (spec, want) in cases {
        let yaml = format!("name: demo\ncommands:\n  {spec}\n");
        let err = parse(&yaml).err().unwrap_or_else(|| panic!("should fail:\n{yaml}"));
        let err = err.to_string();
        assert!(err.contains(want), "want `{want}` in error for:\n{yaml}\ngot: {err}");
    }
}

#[test]
fn cmd_items_split_a_list_into_steps() -> Result<()> {
    let yaml = r#"
  test: cargo test
  check:
    - a
    - b
    - { cmd: test, args: [--verbose], env: { RUST_LOG: debug } }
    - { cmd: test }
    - c
"#;
    let want = Script::Steps(vec![
        Step::Run("set -e\na\nb".to_string()),
        Step::Cmd(CmdRef {
            cmd: "test".to_string(),
            args: vec!["--verbose".to_string()],
            env: BTreeMap::from([("RUST_LOG".to_string(), "debug".to_string())]),
        }),
        Step::Cmd(CmdRef {
            cmd: "test".to_string(),
            args: Vec::new(),
            env: BTreeMap::new(),
        }),
        Step::Run("set -e\nc".to_string()),
    ]);
    assert_eq!(Some(want), command(yaml, "check")?.script);
    Ok(())
}

#[test]
fn cmd_items_reject_bad_refs() {
    let cases = [
        ("a: [{ cmd: nope }]", "command `a` runs `nope`, which isn't in dev.yaml"),
        ("a: [{ cmd: a }]", "command `a` ends up running itself: a -> a"),
        (
            "a: [{ cmd: b }]\n  b: [x, { cmd: c }]\n  c: [{ cmd: b }]",
            "command `b` ends up running itself: b -> c -> b",
        ),
        ("a: [{ cmd: b, nope: 1 }]\n  b: x", "unknown field `nope`"),
        ("a: [{ cmd: '' }]", "command 1 in the list has an empty `cmd`"),
        ("a:\n    run: { cmd: b }\n  b: x", "invalid type: map"),
    ];
    for (spec, want) in cases {
        let yaml = format!("name: demo\ncommands:\n  {spec}\n");
        let err = parse(&yaml).err().unwrap_or_else(|| panic!("should fail:\n{yaml}"));
        let err = err.to_string();
        assert!(err.contains(want), "want `{want}` in error for:\n{yaml}\ngot: {err}");
    }
}

#[test]
fn dep_order_runs_deps_of_deps_first_and_each_once() -> Result<()> {
    let yaml = r#"name: demo
commands:
  a: { depends: [b, c], run: x }
  b: { depends: [d], run: x }
  c: { depends: [d], run: x }
  d: x
"#;
    let commands = parse(yaml)?.commands;

    assert_eq!(vec!["d", "b", "c"], dep_order(&commands, "a")?);
    assert!(dep_order(&commands, "d")?.is_empty());
    Ok(())
}

#[test]
fn depends_rejects_bad_graphs() {
    let cases = [
        (
            "a: { depends: [nope], run: x }",
            "command `a` depends on `nope`, which isn't in dev.yaml",
        ),
        (
            "a: { depends: [a], run: x }",
            "command `a` ends up running itself: a -> a",
        ),
        (
            "a: { depends: [b], run: x }\n  b: { depends: [a], run: x }",
            "command `a` ends up running itself: a -> b -> a",
        ),
        (
            "a: { depends: [deploy], run: x }\n  deploy: { args: [{ name: target }], run: x }",
            "command `a` depends on `deploy`, whose arg `target` is required",
        ),
        (
            "a: { depends: [b], run: x }\n  b: [{ cmd: a }]",
            "command `a` ends up running itself: a -> b -> a",
        ),
    ];
    for (spec, want) in cases {
        let yaml = format!("name: demo\ncommands:\n  {spec}\n");
        let err = parse(&yaml).err().unwrap_or_else(|| panic!("should fail:\n{yaml}"));
        let err = format!("{err:#}");
        assert!(err.contains(want), "want `{want}` in error for:\n{yaml}\ngot: {err}");
    }
}
