use crate::{
    error::{AppError, Result},
    logging::Verbosity,
};

/// Runs the `syp` terminal user interface.
///
/// # Errors
/// Returns an error when the TUI cannot start.
pub async fn run() -> Result<()> {
    let verbosity = Verbosity::new(false, false, false);
    println!("{}", verbosity.header_stdout("SortYourPapers TUI"));
    println!("The ratatui workflow is being initialized in subsequent changes.");
    Err(AppError::Execution(
        "tui mode is not fully implemented yet".to_string(),
    ))
}
