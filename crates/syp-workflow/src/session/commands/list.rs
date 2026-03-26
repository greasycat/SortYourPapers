use crate::{error::Result, session::RunWorkspace};

use super::selection::format_run_summary;

/// Lists saved sessions in the current workspace cache.
///
/// # Errors
/// Returns an error when the run cache cannot be read.
pub fn list_sessions() -> Result<()> {
    let runs = RunWorkspace::list_runs()?;
    let runs_root = RunWorkspace::runs_root()?;

    if runs.is_empty() {
        println!("No saved sessions found under {}", runs_root.display());
        return Ok(());
    }

    println!("Saved sessions:");
    for (index, run) in runs.iter().enumerate() {
        println!("  {}", format_run_summary(index + 1, run));
    }

    Ok(())
}
