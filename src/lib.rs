pub mod app;
pub mod cli;
pub mod config;
pub mod error;
pub mod fs_ops;
pub mod llm;
pub mod papers;
pub mod placement;
pub mod session;
pub mod syp;
pub mod taxonomy;
pub mod terminal;
pub mod tui;

mod entrypoints;
mod report;

#[cfg(test)]
mod lib_tests;

pub use app::{run, run_extract_text, run_with_args};
pub use cli::{Cli, CliArgs, Commands, ExtractTextArgs, SessionCommands};
pub use entrypoints::{print_error_with_hints, run_cli};
pub use session::{
    clear_sessions, init_config, list_sessions, remove_sessions, rerun_run, resume_run,
    review_session,
};
pub use syp::{SypCli, run_syp};
