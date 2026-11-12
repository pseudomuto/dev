use std::{collections::BTreeMap, env};

use clap::{CommandFactory, Parser, Subcommand};
use color_eyre::eyre::{Result, WrapErr, bail};
use dev::{Context, project::Project, shell};

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
    let cli = Cli::parse();
    let vars = process_env()?;

    // Runs anywhere, with or without ./dev.yaml.
    if let Command::HookEnv = cli.command {
        if let Some(script) = shell::env::hook(&vars)? {
            print!("{script}");
        }
        return Ok(());
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
        Command::HookEnv => unreachable!("handled before loading dev.yaml"),
        Command::Up => dev::up(&mut ctx, &project)?,
        Command::Down => dev::down(&mut ctx, &project)?,
        Command::Project(args) => {
            let (name, args) = args.split_first().expect("clap passes the command name first");
            let code = dev::run_command(&ctx, &project, name, args)?;
            std::process::exit(code);
        }
    }

    Ok(())
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
