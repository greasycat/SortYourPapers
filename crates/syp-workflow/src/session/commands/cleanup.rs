use crate::{
    error::{AppError, Result},
    session::{RunStage, RunWorkspace},
};

use super::selection::{
    confirm_session_deletion, runs_matching_ids, select_run_for_removal_interactively,
    validate_run_ids,
};

/// Removes one or more saved sessions.
///
/// # Errors
/// Returns an error when no sessions exist, a run id is invalid, or deletion fails.
#[allow(clippy::needless_pass_by_value)]
pub fn remove_sessions(run_ids: Vec<String>) -> Result<()> {
    let runs = RunWorkspace::list_runs()?;
    if runs.is_empty() {
        return Err(AppError::Execution(format!(
            "no saved sessions found under {}",
            RunWorkspace::runs_root()?.display()
        )));
    }

    let selected_run_ids = if run_ids.is_empty() {
        vec![select_run_for_removal_interactively(&runs)?]
    } else {
        validate_run_ids(&run_ids, &runs)?
    };
    let selected_runs = runs_matching_ids(&runs, &selected_run_ids);

    confirm_session_deletion(
        &selected_runs,
        &format!(
            "Remove {} saved session(s)? [y/N]: ",
            selected_run_ids.len()
        ),
    )?;
    let removed = RunWorkspace::remove_runs(&selected_run_ids)?;
    println!("Removed {} saved session(s)", removed.len());
    Ok(())
}

/// Clears all incomplete saved sessions.
///
/// # Errors
/// Returns an error when selection or deletion fails.
pub fn clear_sessions() -> Result<()> {
    let runs = RunWorkspace::list_runs()?;
    let incomplete_runs = runs
        .into_iter()
        .filter(|run| run.last_completed_stage != Some(RunStage::Completed))
        .collect::<Vec<_>>();

    if incomplete_runs.is_empty() {
        println!("No incomplete saved sessions found");
        return Ok(());
    }

    let selected_runs = incomplete_runs.iter().collect::<Vec<_>>();
    confirm_session_deletion(
        &selected_runs,
        &format!(
            "Clear {} incomplete saved session(s)? [y/N]: ",
            incomplete_runs.len()
        ),
    )?;
    let removed = RunWorkspace::clear_incomplete_runs()?;
    println!("Cleared {} incomplete saved session(s)", removed.len());
    Ok(())
}
