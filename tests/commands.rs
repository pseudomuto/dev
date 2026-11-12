use dev::{
    Result,
    commands::{Arg, Flag, FlagKind, ProjectCommand, Script},
    project::Project,
};

/// The command `name` from a dev.yaml whose `commands` section is `yaml`.
fn command(yaml: &str, name: &str) -> Result<ProjectCommand> {
    let mut project = Project::from_yaml(&format!("name: demo\ncommands:\n{yaml}"))?;
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
        script: Script::Run("echo hi".to_string()),
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
        script: Script::File("scripts/release.sh".into()),
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
        ("desc: nothing to run", "needs `run` or `file`"),
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
        let err = Project::from_yaml(&yaml)
            .err()
            .unwrap_or_else(|| panic!("should fail:\n{yaml}"));
        let err = err.to_string();
        assert!(err.contains(want), "want `{want}` in error for:\n{yaml}\ngot: {err}");
    }
}
