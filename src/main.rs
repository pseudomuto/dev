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
    new::{self, Templates},
    project::Project,
    report::{Plain, Reporter, clack::Clack},
    run_log::{self, RunLog},
    shell,
    update::{self, Outcome, Updater},
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
    /// Create ./<NAME> from a template, then git init it and run dev up in it
    New {
        /// The project's name: lowercase words separated by dashes
        name: String,
        /// The template to use, e.g. go or rust. Your own in
        /// ~/.config/dev/templates/<name> win over the built-in ones
        #[arg(short, long)]
        template: String,
    },
    /// Take down every dependency in ./dev.yaml, last one first
    Down,
    /// Print the shell setup. Add `eval "$(dev shell-init)"` to your
    /// .zshrc or .bashrc
    ShellInit,
    /// Install the latest dev release, if it's newer than this one
    SelfUpdate {
        /// Run by the prompt hook: save a notice for the next dev command
        /// instead of only printing it.
        #[arg(long, hide = true)]
        background: bool,
    },
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

    // The template's notes, printed once `dev new` has run `dev up`.
    let mut notes = None;

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
            // After the script, and never instead of it: a failure here
            // mustn't leave the shell without its env.
            if let Err(e) = auto_update(&vars) {
                eprintln!("dev: failed to check for updates: {e:#}");
            }
            return Ok(());
        }
        Command::SelfUpdate { background } => {
            let updater = Updater::for_current_exe()?;
            let outcome = updater.update(&Context::default())?;
            let notice = update::describe(&outcome);
            if background && matches!(outcome, Outcome::Updated { .. }) {
                updater.save_notice(&notice)?;
            } else if !background {
                // Anything saved is older news than this.
                updater.take_notice()?;
            }
            println!("{notice}");
            return Ok(());
        }
        // Creates the project, then carries on below as `dev up` in it.
        Command::New { ref name, ref template } => {
            let dest = Path::new(name);
            notes = Templates::user_default()?.find(template)?.create(name, dest)?;
            new::git_init(&Context::default(), dest)?;
            env::set_current_dir(dest).wrap_err_with(|| format!("failed to change to {name}"))?;
        }
        _ => {}
    }

    print_update_notice();

    let project = match cli.command {
        // The shell's `$PWD` is still the parent dir, but `${PWD}` in
        // dev.yaml must be the new project's.
        Command::New { .. } => {
            let cwd = env::current_dir().wrap_err("failed to read the current directory")?;
            let cwd = cwd.to_string_lossy().into_owned();
            Project::load_with(PROJECT_FILE, &|var| match var {
                "PWD" => Ok(cwd.clone()),
                _ => env::var(var),
            })?
        }
        _ => Project::load(PROJECT_FILE)?,
    };
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
        Command::ShellInit | Command::HookEnv | Command::SelfUpdate { .. } => {
            unreachable!("handled before loading dev.yaml")
        }
        Command::Up => with_run_log(&mut ctx, "up", &project.name, |ctx| dev::up(ctx, &project))?,
        Command::New { name, .. } => {
            let up = with_run_log(&mut ctx, "up", &project.name, |ctx| dev::up(ctx, &project));
            if up.is_ok() {
                println!("\nCreated {name}. Next: cd {name}");
            }
            // The project exists either way, so its notes still apply.
            if let Some(notes) = notes {
                println!("\n{}", notes.trim_end());
            }
            up.with_note(|| format!("{name} was created; run `dev up` in it to try again"))?;
        }
        Command::Down => with_run_log(&mut ctx, "down", &project.name, |ctx| dev::down(ctx, &project))?,
        Command::Project(args) => {
            let (name, args) = args.split_first().expect("clap passes the command name first");
            let code = dev::run_command(&ctx, &project, name, args)?;
            std::process::exit(code);
        }
    }

    Ok(())
}

/// Starts a background `dev self-update` if `DEV_AUTO_UPDATE` (in `vars`)
/// allows it and the last check was long enough ago.
fn auto_update(vars: &BTreeMap<String, String>) -> Result<()> {
    let updater = Updater::for_current_exe()?;
    if updater.auto_update_enabled(vars.get("DEV_AUTO_UPDATE").map(String::as_str)) {
        updater.spawn_if_due(&Context::default())?;
    }
    Ok(())
}

/// Prints what the last background update did, once. A failure to read it
/// is only a warning: it shouldn't stop the command that was asked for.
fn print_update_notice() {
    let notice = Updater::for_current_exe().and_then(|u| u.take_notice());
    match notice {
        Ok(Some(notice)) => eprintln!("{notice}"),
        Ok(None) => {}
        Err(e) => eprintln!("warning: failed to read the update notice: {e:#}"),
    }
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
