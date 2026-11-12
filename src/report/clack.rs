//! The terminal UI: a clack-style rail (via cliclack) with a line per met
//! dependency, and a block with a spinner per task for the rest. Hollow
//! diamonds are in progress, filled ones are done.
//!
//! ```text
//! ┌  dev up · demo-project
//! │
//! ◆  Install brew dependencies
//! ◇  Install tools
//! │  ✓ Install mise
//! │  ◒ Installing node@lts · mise node@24.11.1 download 61%
//! ```

#[cfg(test)]
mod tests;

use std::path::Path;

use cliclack::{MultiProgress, ProgressBar, Theme, ThemeState};
use color_eyre::{Report, eyre::eyre};
use console::{Emoji, Style, Term, strip_ansi_codes, style, truncate_str};

use super::{Reporter, Status};
use crate::Result;

/// Columns before a task's message: the rail, its padding, and the spinner
/// (`│  ◒ `).
const MESSAGE_INDENT: usize = 5;

const S_BAR_START: Emoji = Emoji("┌", "T");
const S_BAR: Emoji = Emoji("│", "|");
const S_BAR_END: Emoji = Emoji("└", "—");

/// Columns the rail takes before a line of text (`│  `).
const RAIL_WIDTH: usize = 3;

/// cliclack's theme with bold cyan borders, and its step symbols swapped so
/// a dependency's diamond fills in once it's done (cliclack fills the active
/// one).
struct DevTheme;

impl Theme for DevTheme {
    /// Every border (`┌`, `│`, `└`) is bold cyan, whatever the state.
    fn bar_color(&self, _state: &ThemeState) -> Style {
        Style::new().cyan().bold()
    }

    /// cliclack's symbol colors, set here because cliclack's own default
    /// falls back to [`DevTheme::bar_color`], which would make `▲` cyan.
    fn state_symbol_color(&self, state: &ThemeState) -> Style {
        match state {
            ThemeState::Active => Style::new().cyan(),
            ThemeState::Submit => Style::new().green(),
            ThemeState::Cancel => Style::new().red(),
            ThemeState::Error(_) => Style::new().yellow(),
        }
    }

    fn state_symbol(&self, state: &ThemeState) -> String {
        let symbol = match state {
            ThemeState::Active => Emoji("◇", "o"),
            ThemeState::Submit => Emoji("◆", "*"),
            ThemeState::Cancel => Emoji("■", "x"),
            ThemeState::Error(_) => Emoji("▲", "x"),
        };
        self.state_symbol_color(state).apply_to(symbol).to_string()
    }

    /// cliclack's version (theme.rs, 0.5.7) without the blank `│` lines it
    /// adds after a finished step and under the last task in a block, so
    /// the steps sit on consecutive lines. A footer with a message (`└  Install
    /// tools failed`) is kept.
    ///
    /// Recheck this against upstream when upgrading cliclack.
    fn format_progress_with_state(&self, msg: &str, grouped: bool, last: bool, state: &ThemeState) -> String {
        let prefix = if grouped {
            self.bar_color(state).apply_to(S_BAR).to_string() + "  "
        } else {
            match state {
                ThemeState::Active => String::new(),
                _ => self.state_symbol(state) + "  ",
            }
        };
        let suffix = match state {
            ThemeState::Error(_) | ThemeState::Cancel if grouped && last => format!("\n{}", self.format_footer(state)),
            _ => String::new(),
        };

        if msg.is_empty() {
            suffix
        } else {
            format!("{prefix}{msg}{suffix}")
        }
    }

    /// cliclack's version (theme.rs, 0.5.7) with only the `└`/`│` in the
    /// border color. cliclack colors the whole line, which would make an
    /// error like `Install tools failed` bold cyan; it keeps its own color.
    ///
    /// Recheck this against upstream when upgrading cliclack.
    fn format_footer_with_message(&self, state: &ThemeState, message: &str) -> String {
        let bar = self.bar_color(state);
        let line = match state {
            ThemeState::Active => format!("{}  {message}", bar.apply_to(S_BAR_END)),
            ThemeState::Cancel => format!("{}  {}", bar.apply_to(S_BAR_END), style("Operation cancelled.").red()),
            ThemeState::Submit => bar.apply_to(S_BAR).to_string(),
            ThemeState::Error(err) => format!("{}  {}", bar.apply_to(S_BAR_END), style(err).yellow()),
        };
        format!("{line}\n")
    }
}

/// The dependency being worked on: a block on the rail with its tasks under
/// it.
struct Block {
    name: String,
    multi: MultiProgress,
    /// Spins until the first task starts, which takes it over: a spinner in a
    /// block can't be removed without leaving a line behind.
    idle: Option<ProgressBar>,
    /// The task being worked on, with its name.
    task: Option<(ProgressBar, String)>,
}

impl Block {
    fn open(name: impl Into<String>, idle_message: &str) -> Self {
        let name = name.into();
        let multi = cliclack::multi_progress(&name);
        let idle = multi.add(cliclack::spinner());
        idle.start(idle_message);
        Self {
            name,
            multi,
            idle: Some(idle),
            task: None,
        }
    }

    /// The spinner the current output belongs to: the task's, or the idle
    /// one for a dependency without tasks.
    fn active(&self) -> Option<(&ProgressBar, &str)> {
        match (&self.task, &self.idle) {
            (Some((pb, name)), _) => Some((pb, name)),
            (None, Some(pb)) => Some((pb, &self.name)),
            (None, None) => None,
        }
    }
}

/// Shows progress as a clack-style rail on stderr. Only for terminals: use
/// [`Plain`](super::Plain) otherwise.
#[derive(Default)]
pub struct Clack {
    /// The dependency being checked or taken down, as a lone spinner.
    /// Stopped, it's the dependency's line; cleared (which leaves nothing),
    /// it makes way for its block.
    lone: Option<ProgressBar>,
    block: Option<Block>,
    /// The dependency and task a [`Reporter::pause`] interrupted.
    paused: Option<(String, Option<String>)>,
}

impl Clack {
    /// Also sets cliclack's theme, which is global.
    pub fn new() -> Self {
        cliclack::set_theme(DevTheme);
        Self::default()
    }

    fn block(&mut self, name: &str) -> Result<&mut Block> {
        self.block
            .as_mut()
            .ok_or_else(|| eyre!("task `{name}` was reported outside of a dependency"))
    }

    fn task_step(&mut self, name: &str, status: Status) -> Result<()> {
        let block = self.block(name)?;
        match status {
            Status::Checking => {
                let pb = match block.idle.take() {
                    Some(pb) => {
                        pb.set_message(name);
                        pb
                    }
                    None => {
                        let pb = block.multi.add(cliclack::spinner());
                        pb.start(name);
                        pb
                    }
                };
                block.task = Some((pb, name.to_string()));
            }
            Status::Applying => {
                if let Some((pb, _)) = &block.task {
                    pb.set_message(name);
                }
            }
            Status::Met | Status::Done => {
                if let Some((pb, _)) = block.task.take() {
                    pb.stop(finished(name));
                }
            }
            Status::Failed => {
                if let Some((pb, _)) = block.task.take() {
                    pb.error(failed(name));
                }
            }
            Status::TakingDown | Status::Down => {}
        }
        Ok(())
    }

    fn dependency_step(&mut self, name: &str, status: Status) -> Result<()> {
        match status {
            Status::Checking | Status::TakingDown => {
                let pb = cliclack::spinner();
                pb.start(name);
                self.lone = Some(pb);
            }
            // A filled diamond, like a met dependency.
            Status::Met | Status::Down => {
                if let Some(pb) = self.lone.take() {
                    pb.stop(name);
                }
            }
            Status::Applying => {
                if let Some(pb) = self.lone.take() {
                    pb.clear();
                }
                self.block = Some(Block::open(name, "applying"));
            }
            Status::Done => {
                if let Some(block) = self.block.take() {
                    if let Some(pb) = block.idle {
                        pb.stop(finished("done"));
                    }
                    block.multi.stop();
                }
            }
            Status::Failed => {
                // Checking or taking it down failed.
                if let Some(pb) = self.lone.take() {
                    pb.error(name);
                }
                if let Some(block) = self.block.take() {
                    if let Some(pb) = block.idle {
                        pb.error(failed("failed"));
                    }
                    block.multi.error(format!("{name} failed"));
                }
            }
        }
        Ok(())
    }
}

impl Reporter for Clack {
    /// Starts the rail with a bold title: the command in magenta and the
    /// project in bright white, e.g. `dev up · demo-project`.
    fn start(&mut self, command: &str, project: &str) -> Result<()> {
        let title = format!(
            "{} {} {}",
            style(format!("dev {command}")).magenta().bold(),
            style("·").bold(),
            style(project).bright().white().bold(),
        );
        cliclack::intro(title)?;
        Ok(())
    }

    fn step(&mut self, depth: usize, name: &str, status: Status) -> Result<()> {
        match depth {
            0 => self.dependency_step(name, status),
            _ => self.task_step(name, status),
        }
    }

    fn output(&mut self, line: &str) -> Result<()> {
        if let Some((pb, name)) = self.block.as_ref().and_then(Block::active) {
            let (_, cols) = Term::stderr().size();
            pb.set_message(tail_message(name, line, usize::from(cols)));
        }
        Ok(())
    }

    /// Ends the current block, so nothing redraws over the command's prompt.
    /// A redraw would wipe lines written below the block.
    fn pause(&mut self) -> Result<()> {
        if let Some(block) = self.block.take() {
            let task = block.task.map(|(pb, name)| {
                pb.stop(format!("{name} · waiting for input"));
                name
            });
            if let Some(pb) = block.idle {
                pb.stop("waiting for input");
            }
            block.multi.stop();
            self.paused = Some((block.name, task));
        }
        Ok(())
    }

    /// Picks up where [`Reporter::pause`] left off, in a new block.
    fn resume(&mut self) -> Result<()> {
        if let Some((dep, task)) = self.paused.take() {
            let mut block = Block::open(format!("{dep} (continued)"), "applying");
            if let (Some(name), Some(pb)) = (task, block.idle.take()) {
                pb.set_message(&name);
                block.task = Some((pb, name));
            }
            // Steps keep reporting under the dependency's own name.
            block.name = dep;
            self.block = Some(block);
        }
        Ok(())
    }

    /// Closes the rail after a successful run. A failed one was already
    /// closed by the dependency's error, and a second closing line looks
    /// broken.
    fn finish(&mut self, ok: bool) -> Result<()> {
        if ok {
            cliclack::outro("Done")?;
        }
        Ok(())
    }

    /// Shows the error in a red block of its own, under the failed
    /// dependency's.
    fn error(&mut self, err: &Report, log: &Path) -> Result<bool> {
        // The first cause repeats the failed dependency's line above, unless
        // it's the only one.
        let mut causes: Vec<String> = err.chain().map(ToString::to_string).collect();
        if causes.len() > 1 {
            causes.remove(0);
        }

        let (_, cols) = Term::stderr().size();
        let red = Style::new().red().bold();
        let mut block = format!("\n{}  {}\n", red.apply_to(S_BAR_START), red.apply_to("Error"));
        for line in error_lines(&causes, &log.display().to_string(), usize::from(cols)) {
            if line.is_empty() {
                block += &format!("{}\n", red.apply_to(S_BAR));
            } else {
                block += &format!("{}  {line}\n", red.apply_to(S_BAR));
            }
        }
        block += &format!("{}\n", red.apply_to(S_BAR_END));

        Term::stderr().write_str(&block)?;
        Ok(true)
    }
}

/// The lines of an error block, without the rail: each cause, its command
/// output (the lines after a cause's first) set off by blank lines, then
/// where the full output is. Lines are wrapped to fit `cols` after the rail,
/// keeping their indent.
fn error_lines(causes: &[String], log: &str, cols: usize) -> Vec<String> {
    let width = cols.saturating_sub(RAIL_WIDTH);
    let mut lines = Vec::new();

    for cause in causes {
        let mut cause_lines = cause.lines();
        if let Some(first) = cause_lines.next() {
            lines.extend(wrap(first, width));
        }
        let output: Vec<&str> = cause_lines.collect();
        if !output.is_empty() {
            lines.push(String::new());
            for line in output {
                lines.extend(wrap(&strip_ansi_codes(line), width));
            }
        }
    }

    lines.push(String::new());
    lines.extend(wrap(&format!("Full output: {log}"), width));
    lines
}

/// `line` wrapped to `width`, each piece keeping its indent, with two more
/// spaces for the pieces after the first.
fn wrap(line: &str, width: usize) -> Vec<String> {
    let text = line.trim_start();
    let indent = &line[..line.len() - text.len()];
    if text.is_empty() {
        return vec![String::new()];
    }

    let rest = format!("{indent}  ");
    let options = textwrap::Options::new(width.max(1))
        .initial_indent(indent)
        .subsequent_indent(&rest);
    textwrap::wrap(text, options)
        .into_iter()
        .map(|l| l.into_owned())
        .collect()
}

/// A finished (or met) task's line: `✓ Installing jq`, the mark in green.
/// The mark sits where the spinner was, so the names stay lined up.
fn finished(name: &str) -> String {
    format!("{} {name}", style(Emoji("✓", "v")).green())
}

/// A failed task's line: `✗ Installing jq`, the mark in red.
fn failed(name: &str) -> String {
    format!("{} {name}", style(Emoji("✗", "x")).red())
}

/// The message for task `name` showing its latest output `line`, e.g.
/// `Installing jq · ==> Downloading jq`. It's cut to fit `cols` columns,
/// since a message that wraps breaks cliclack's redraw.
fn tail_message(name: &str, line: &str, cols: usize) -> String {
    let line = strip_ansi_codes(line);
    let line = line.trim();
    let message = if line.is_empty() {
        name.to_string()
    } else {
        format!("{name} · {line}")
    };
    truncate_str(&message, cols.saturating_sub(MESSAGE_INDENT), "…").into_owned()
}
