pub mod commands;
mod context;
pub mod deps;
mod down;
pub mod new;
pub mod process;
pub mod project;
pub mod report;
pub mod run_log;
pub mod shell;
mod up;
pub mod update;

pub type Result<T> = color_eyre::eyre::Result<T>;

pub use commands::run_command;
pub use context::Context;
pub use down::down;
pub use up::up;
