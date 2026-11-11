use clap::{CommandFactory, Parser, Subcommand};
use color_eyre::eyre::{Result, bail};
use dev::{Context, project::Project};

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
    /// Run a command from the `commands` section of ./dev.yaml
    #[command(external_subcommand)]
    Project(Vec<String>),
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    let project = Project::load(PROJECT_FILE)?;
    reject_built_in_names(&project)?;
    let mut ctx = Context {
        env: project.env.clone(),
        ..Context::default()
    };

    match cli.command {
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
