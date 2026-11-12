use std::{
    collections::BTreeMap,
    env,
    io::{self, IsTerminal},
    path::Path,
};

use clap::{CommandFactory, Parser, Subcommand, error::ErrorKind};
use color_eyre::{
    Section,
    eyre::{Result, WrapErr, bail},
};
use dev::{
    Context,
    project::Project,
    report::{Plain, Reporter, clack::Clack},
    run_log::{self, RunLog},
    shell,
};

/// The project file `dev` reads, relative to the current directory.
const PROJECT_FILE: &str = "dev.yaml";

#[derive(Parser)]
#[command(version, about = "Set up a project's development dependencies")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Meet every dependency in ./dev.yaml
    Up,
    /// Take down every dependency in ./dev.yaml, last one first
    Down,
    /// Print the shell setup. Add `eval "$(dev shell-init)"` to your
    /// .zshrc or .bashrc
    ShellInit,
    /// Print the sh code that loads dev.yaml env into the shell. Run by
    /// `share/shell` before every prompt.
    #[command(hide = true)]
    HookEnv,
    /// Run a command from the `commands` section of ./dev.yaml
    #[command(external_subcommand)]
    Project(Vec<String>),
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = parse_cli();
    let vars = process_env()?;

    // These run anywhere, with or without ./dev.yaml.
    match cli.command {
        Command::ShellInit => {
            print!("{}", shell::SETUP);
            return Ok(());
        }
        Command::HookEnv => {
            if let Some(script) = shell::env::hook(&vars)? {
                print!("{script}");
            }
            return Ok(());
        }
        _ => {}
    }

    let project = Project::load(PROJECT_FILE)?;
    reject_built_in_names(&project)?;

    // A shell that already loaded this dev.yaml has its env, so expanding it
    // again would apply it twice, e.g. `${PWD}/bin` twice on PATH.
    let cwd = env::current_dir().wrap_err("failed to read the current directory")?;
    let env = if shell::env::applied(&vars, &cwd.join(PROJECT_FILE))? {
        Vec::new()
    } else {
        project.env.clone()
    };
    let mut ctx = Context {
        env,
        ..Context::default()
    };

    match cli.command {
        Command::ShellInit | Command::HookEnv => unreachable!("handled before loading dev.yaml"),
        Command::Up => with_run_log(&mut ctx, "up", &project.name, |ctx| dev::up(ctx, &project))?,
        Command::Down => with_run_log(&mut ctx, "down", &project.name, |ctx| dev::down(ctx, &project))?,
        Command::Project(args) => {
            let (name, args) = args.split_first().expect("clap passes the command name first");
            let code = dev::run_command(&ctx, &project, name, args)?;
            std::process::exit(code);
        }
    }

    Ok(())
}

/// Parses the command line, exiting on errors and `--help`.
///
/// Help lists the commands in ./dev.yaml, so only help reads it here.
/// Everything else, `hook-env` before every prompt especially, would just
/// pay for a parse it doesn't use.
fn parse_cli() -> Cli {
    let err = match Cli::try_parse() {
        Ok(cli) => return cli,
        Err(err) => err,
    };
    // A bare `dev` shows help too, as a usage error.
    if !matches!(
        err.kind(),
        ErrorKind::DisplayHelp | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    ) {
        err.exit();
    }

    let loaded = Project::load(PROJECT_FILE);
    let mut cmd = Cli::command();
    if let Some(help) = project_commands_help(&cmd, &loaded) {
        cmd = cmd.after_help(help);
    }
    // Same args, so this is the same help request, now with the section.
    cmd.try_get_matches().err().unwrap_or(err).exit()
}

/// A "Project commands:" section for `dev --help`, styled like clap's own
/// sections. `None` without a dev.yaml or when it has no commands. An
/// invalid one gets a note instead of a list, so help still works.
fn project_commands_help(cmd: &clap::Command, loaded: &Result<Project>) -> Option<String> {
    let styles = cmd.get_styles();
    let (header, literal) = (styles.get_header(), styles.get_literal());
    let mut help = format!("{header}Project commands:{header:#}\n");

    match loaded {
        Ok(project) if project.commands.is_empty() => return None,
        Ok(project) => {
            let width = project.commands.keys().map(String::len).max().unwrap_or_default();
            for (name, command) in &project.commands {
                match &command.desc {
                    Some(desc) => help.push_str(&format!("  {literal}{name:<width$}{literal:#}  {desc}\n")),
                    None => help.push_str(&format!("  {literal}{name}{literal:#}\n")),
                }
            }
        }
        Err(_) if !Path::new(PROJECT_FILE).exists() => return None,
        Err(e) => {
            let cause = e.root_cause().to_string();
            let cause = cause.lines().next().unwrap_or_default();
            help.push_str(&format!("  ({PROJECT_FILE} is invalid: {cause})\n"));
        }
    }
    Some(help)
}

/// Runs `f` (the `command` run, e.g. `up`, for `project`) with progress
/// going to a [`reporter`] and the output of the commands it runs going to a
/// new log in `./.dev/log` (next to dev.yaml). A failure says where that log
/// is.
fn with_run_log(
    ctx: &mut Context,
    command: &str,
    project: &str,
    f: impl FnOnce(&mut Context) -> Result<()>,
) -> Result<()> {
    let log = RunLog::create(Path::new("."), command, run_log::KEEP)?;
    ctx.log = Box::new(log.file);
    ctx.reporter = reporter()?;

    ctx.reporter.start(command, project)?;
    let res = f(ctx);
    let finished = ctx.reporter.finish(res.is_ok());

    if let Err(err) = &res
        && ctx.reporter.error(err, &log.path)?
    {
        // It's on screen already, and returning it would print it again.
        std::process::exit(1);
    }
    // The run's own failure is the one worth reporting.
    res.with_note(|| format!("full output is in {}", log.path.display()))?;
    finished
}

/// The clack UI when stderr (where it draws) is a terminal, plain lines on
/// stdout otherwise. `DEV_UI=plain` forces plain lines.
fn reporter() -> Result<Box<dyn Reporter>> {
    let plain = match env::var("DEV_UI") {
        Ok(ui) if ui == "plain" => true,
        Ok(ui) => bail!("unknown DEV_UI `{ui}`; the only option is `plain`"),
        Err(env::VarError::NotPresent) => false,
        Err(env::VarError::NotUnicode(ui)) => bail!("unknown DEV_UI {ui:?}; the only option is `plain`"),
    };

    if !plain && io::stderr().is_terminal() {
        Ok(Box::new(Clack::new()))
    } else {
        Ok(Box::new(Plain::new(Box::new(io::stdout()))))
    }
}

/// The process env, skipping vars that aren't UTF-8. `PWD` falls back to the
/// current directory when the shell didn't set it.
fn process_env() -> Result<BTreeMap<String, String>> {
    let mut vars: BTreeMap<String, String> = env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .collect();

    if !vars.contains_key("PWD") {
        let cwd = env::current_dir().wrap_err("failed to read the current directory")?;
        let cwd = cwd.to_str().map(str::to_string);
        vars.extend(cwd.map(|cwd| ("PWD".to_string(), cwd)));
    }
    Ok(vars)
}

/// Fails if a project command has the same name as a built-in one, which
/// would always win.
fn reject_built_in_names(project: &Project) -> Result<()> {
    let cli = Cli::command();
    let built_ins = cli.get_subcommands().map(|c| c.get_name()).chain(["help"]);

    for name in built_ins {
        if project.commands.contains_key(name) {
            bail!("command `{name}` in dev.yaml conflicts with a built-in command");
        }
    }
    Ok(())
}
