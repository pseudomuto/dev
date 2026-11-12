use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
};

use clap::ArgAction;
use color_eyre::eyre::{WrapErr, bail};
use serde::{
    Deserialize,
    Deserializer,
    de::{self, MapAccess, Visitor},
};

use crate::{Context, Result, project::Project};

/// Flag and arg names whose env var would clobber something the script
/// itself depends on.
const RESERVED_VARS: &[&str] = &["PATH", "HOME", "PWD", "IFS", "SHELL", "HELP"];

/// The clap id for args after `--`. Can't clash with a flag or arg, since
/// those are lowercase and dashed.
const REST: &str = "_rest";

/// A command from the `commands` section of `dev.yaml`, run by `dev <name>`.
///
/// A plain string is a shell snippet whose args pass through as `$@`. A map
/// can point at a script `file` instead, and declare `flags` and `args` that
/// dev parses and hands to the script as env vars, e.g. `--dry-run` as
/// `$DRY_RUN`:
///
/// ```yaml
/// commands:
///   test: cargo test
///   deploy:
///     desc: Deploy an app
///     flags:
///       env: { short: e, default: staging }
///       dry-run: { type: bool }
///     args:
///       - name: target
///     run: echo "$TARGET to $ENV"
/// ```
#[derive(Debug, PartialEq)]
pub struct ProjectCommand {
    /// Shown in `dev <name> --help` and the list of commands.
    pub desc: Option<String>,
    pub script: Script,
    /// Sorted by name.
    pub flags: Vec<Flag>,
    /// Positional, in the order they're declared.
    pub args: Vec<Arg>,
}

/// What a [`ProjectCommand`] runs.
#[derive(Debug, PartialEq)]
pub enum Script {
    /// A snippet run with `sh -c`.
    Run(String),
    /// An executable, relative to dev.yaml, run directly so its shebang
    /// picks the interpreter.
    File(PathBuf),
}

/// A `--name` flag.
#[derive(Debug, PartialEq)]
pub struct Flag {
    pub name: String,
    pub short: Option<char>,
    pub desc: Option<String>,
    /// Only for string flags.
    pub default: Option<String>,
    pub kind: FlagKind,
}

/// What a [`Flag`] holds.
#[derive(Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlagKind {
    /// Takes a value. Its var is unset when not given and with no default.
    #[default]
    String,
    /// Takes no value. Its var is `true` or `false`.
    Bool,
}

/// A positional arg.
#[derive(Debug, PartialEq)]
pub struct Arg {
    pub name: String,
    pub desc: Option<String>,
    /// True unless declared `required: false` or given a default.
    pub required: bool,
    pub default: Option<String>,
}

impl ProjectCommand {
    /// True when there's nothing for dev to parse, so args pass through raw.
    fn takes_raw_args(&self) -> bool {
        self.flags.is_empty() && self.args.is_empty()
    }
}

/// The env var a flag or arg named `name` is passed in, e.g. `DRY_RUN` for
/// `dry-run`.
fn var_name(name: &str) -> String {
    name.to_ascii_uppercase().replace('-', "_")
}

impl<'de> Deserialize<'de> for ProjectCommand {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        deserializer.deserialize_any(CommandVisitor)
    }
}

struct CommandVisitor;

impl<'de> Visitor<'de> for CommandVisitor {
    type Value = ProjectCommand;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a shell snippet, or a map with `run` or `file`")
    }

    fn visit_str<E: de::Error>(self, snippet: &str) -> std::result::Result<ProjectCommand, E> {
        Ok(ProjectCommand {
            desc: None,
            script: Script::Run(snippet.to_string()),
            flags: Vec::new(),
            args: Vec::new(),
        })
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> std::result::Result<ProjectCommand, A::Error> {
        let raw = RawLong::deserialize(de::value::MapAccessDeserializer::new(map))?;
        raw.validate().map_err(de::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLong {
    desc: Option<String>,
    run: Option<String>,
    file: Option<PathBuf>,
    #[serde(default)]
    flags: BTreeMap<String, RawFlag>,
    #[serde(default)]
    args: Vec<RawArg>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFlag {
    short: Option<char>,
    desc: Option<String>,
    default: Option<String>,
    #[serde(default, rename = "type")]
    kind: FlagKind,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawArg {
    name: String,
    desc: Option<String>,
    required: Option<bool>,
    default: Option<String>,
}

impl RawLong {
    /// Checks everything clap would otherwise panic on, or that would make
    /// the script's env ambiguous.
    fn validate(self) -> Result<ProjectCommand> {
        let script = match (self.run, self.file) {
            (Some(run), None) => Script::Run(run),
            (None, Some(file)) => Script::File(file),
            (Some(_), Some(_)) => bail!("a command takes `run` or `file`, not both"),
            (None, None) => bail!("a command needs `run` or `file`"),
        };

        let mut vars = BTreeSet::new();
        let mut shorts = BTreeSet::new();

        let mut flags = Vec::with_capacity(self.flags.len());
        for (name, raw) in self.flags {
            check_name("flag", &name, &mut vars)?;
            if let Some(short) = raw.short {
                if !short.is_ascii_alphanumeric() {
                    bail!("flag `{name}` has short `{short}`; it must be a letter or digit");
                }
                if short == 'h' {
                    bail!("flag `{name}` can't use short `h`; it's taken by `-h` for help");
                }
                if !shorts.insert(short) {
                    bail!("flag `{name}` has short `{short}`, which another flag already uses");
                }
            }
            if raw.kind == FlagKind::Bool && raw.default.is_some() {
                bail!("flag `{name}` is a bool, so it can't have a default");
            }
            flags.push(Flag {
                name,
                short: raw.short,
                desc: raw.desc,
                default: raw.default,
                kind: raw.kind,
            });
        }

        let mut args = Vec::with_capacity(self.args.len());
        let mut optional_seen = None;
        for raw in self.args {
            check_name("arg", &raw.name, &mut vars)?;
            let required = match (raw.required, &raw.default) {
                (Some(true), Some(_)) => bail!("arg `{}` can't be required and have a default", raw.name),
                (Some(required), _) => required,
                (None, default) => default.is_none(),
            };
            // clap can't tell which positional a lone value is for otherwise.
            if required && let Some(optional) = &optional_seen {
                bail!(
                    "arg `{}` is required, so it can't come after optional arg `{optional}`",
                    raw.name
                );
            }
            if !required {
                optional_seen.get_or_insert_with(|| raw.name.clone());
            }
            args.push(Arg {
                name: raw.name,
                desc: raw.desc,
                required,
                default: raw.default,
            });
        }

        Ok(ProjectCommand {
            desc: self.desc,
            script,
            flags,
            args,
        })
    }
}

/// Requires `name` (of a flag or arg, per `what`) to be lowercase dashed words
/// starting with a letter, and its env var to be unused and not reserved.
fn check_name(what: &str, name: &str, vars: &mut BTreeSet<String>) -> Result<()> {
    let words = name
        .split('-')
        .all(|word| !word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
    if !words || !name.starts_with(|c: char| c.is_ascii_lowercase()) {
        bail!("{what} `{name}` must be lowercase words separated by dashes, starting with a letter, e.g. `dry-run`");
    }

    let var = var_name(name);
    if RESERVED_VARS.contains(&var.as_str()) {
        bail!("{what} `{name}` would be passed as ${var}, which dev reserves");
    }
    if !vars.insert(var.clone()) {
        bail!("{what} `{name}` would be passed as ${var}, which another flag or arg already is");
    }
    Ok(())
}

/// Runs the project command `name` from `dev.yaml` and returns its exit code.
///
/// A `run` snippet runs as `sh -c '<snippet>' <name> <args...>`, so `$0` is
/// the command's name and `$@` its args. A `file` runs as `<file> <args...>`.
/// With declared flags or args, dev parses them first: each one is passed as
/// an env var, and `$@` is whatever came after `--`. A parse failure or
/// `--help` prints clap's message and returns its exit code.
///
/// The command is attached to the terminal directly, not through
/// `ctx.out`/`ctx.err`, so colors, prompts, and TTY checks work.
pub fn run_command(ctx: &Context, project: &Project, name: &str, args: &[String]) -> Result<i32> {
    let Some(cmd) = project.commands.get(name) else {
        if project.commands.is_empty() {
            bail!("unknown command `{name}`; dev.yaml has no commands");
        }
        let list: Vec<String> = project
            .commands
            .iter()
            .map(|(name, cmd)| match &cmd.desc {
                Some(desc) => format!("  {name} - {desc}"),
                None => format!("  {name}"),
            })
            .collect();
        bail!("unknown command `{name}`; dev.yaml has:\n{}", list.join("\n"));
    };

    let Parsed { vars, rest } = if cmd.takes_raw_args() {
        Parsed {
            vars: Vec::new(),
            rest: args.to_vec(),
        }
    } else {
        match parse(name, cmd, args) {
            Ok(parsed) => parsed,
            Err(e) => {
                e.print().wrap_err("failed to print usage")?;
                return Ok(e.exit_code());
            }
        }
    };

    let mut process = match &cmd.script {
        Script::Run(snippet) => {
            let mut process = ctx.command("sh");
            process.arg("-c").arg(snippet).arg(name);
            process
        }
        // A bare `script.sh` would be looked up on PATH, not next to dev.yaml.
        Script::File(file) if file.is_relative() => ctx.command(Path::new(".").join(file)),
        Script::File(file) => ctx.command(file),
    };
    let status = process
        .args(rest)
        .envs(vars)
        .status()
        .wrap_err_with(|| match &cmd.script {
            Script::Run(_) => format!("failed to start command `{name}`"),
            Script::File(file) => format!("failed to start command `{name}` ({})", file.display()),
        })?;

    // A command killed by a signal has no code; report it the way shells do.
    Ok(status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(0)))
}

/// A command line parsed by [`parse`].
struct Parsed {
    /// Env vars for the declared flags and args.
    vars: Vec<(String, String)>,
    /// The script's own args: everything after `--`.
    rest: Vec<String>,
}

/// Parses `args` against `cmd`'s flags and args.
fn parse(name: &str, cmd: &ProjectCommand, args: &[String]) -> clap::error::Result<Parsed> {
    let mut app = clap::Command::new(name.to_string()).bin_name(format!("dev {name}"));
    if let Some(desc) = &cmd.desc {
        app = app.about(desc.clone());
    }

    for flag in &cmd.flags {
        let mut arg = clap::Arg::new(flag.name.clone()).long(flag.name.clone());
        if let Some(short) = flag.short {
            arg = arg.short(short);
        }
        if let Some(desc) = &flag.desc {
            arg = arg.help(desc.clone());
        }
        arg = match flag.kind {
            FlagKind::Bool => arg.action(ArgAction::SetTrue),
            FlagKind::String => arg.action(ArgAction::Set),
        };
        if let Some(default) = &flag.default {
            arg = arg.default_value(default.clone());
        }
        app = app.arg(arg);
    }

    for positional in &cmd.args {
        let mut arg = clap::Arg::new(positional.name.clone())
            .value_name(var_name(&positional.name))
            .required(positional.required);
        if let Some(default) = &positional.default {
            arg = arg.default_value(default.clone());
        }
        if let Some(desc) = &positional.desc {
            arg = arg.help(desc.clone());
        }
        app = app.arg(arg);
    }

    app = app.arg(
        clap::Arg::new(REST)
            .value_name("ARGS")
            .num_args(0..)
            .last(true)
            .help("Passed to the script as its args"),
    );

    let matches = app.try_get_matches_from(std::iter::once(name).chain(args.iter().map(String::as_str)))?;

    let mut vars = Vec::new();
    for flag in &cmd.flags {
        let value = match flag.kind {
            FlagKind::Bool => Some(matches.get_flag(&flag.name).to_string()),
            FlagKind::String => matches.get_one::<String>(&flag.name).cloned(),
        };
        vars.extend(value.map(|v| (var_name(&flag.name), v)));
    }
    for arg in &cmd.args {
        let value = matches.get_one::<String>(&arg.name).cloned();
        vars.extend(value.map(|v| (var_name(&arg.name), v)));
    }

    let rest = matches
        .get_many::<String>(REST)
        .map(|values| values.cloned().collect())
        .unwrap_or_default();
    Ok(Parsed { vars, rest })
}
