pub mod app;
pub mod cli;
pub mod config;
pub mod domain;
pub mod error;
pub mod fs_ops;
pub mod llm;
pub mod papers;
pub mod placement;
pub mod session;
pub mod taxonomy;
pub mod terminal;

mod app_run;
mod categorize;
mod discovery;
mod execute;
mod logging;
mod models;
mod pdf_extract;
mod place;
mod planner;
mod report;
mod run_state;
mod session_ops;
mod text_preprocess;

#[cfg(test)]
mod lib_tests;

pub use app_run::{run, run_extract_text, run_with_args};
pub use cli::{Cli, CliArgs, Commands, ExtractTextArgs, SessionCommands};
pub use session_ops::{
    clear_sessions, init_config, list_sessions, remove_sessions, rerun_run, resume_run,
    review_session,
};
