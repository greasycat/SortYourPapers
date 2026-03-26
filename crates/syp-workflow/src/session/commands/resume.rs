use crate::{
    error::{AppError, Result},
    report::RunReport,
    session::{RunStage, RunWorkspace, run_with_workspace},
    terminal::Verbosity,
};

use super::selection::{apply_resume_overrides, select_run_for_resume_interactively};

/// Resumes a saved run from its persisted workspace state.
///
/// # Errors
/// Returns an error when the run cannot be selected or opened, resume overrides
/// are invalid, or a resumed pipeline stage fails.
pub async fn resume_run(
    run_id: Option<String>,
    apply_override: bool,
    verbosity_override: u8,
    quiet_override: bool,
) -> Result<RunReport> {
    let mut workspace = if let Some(run_id) = run_id {
        RunWorkspace::open(&run_id)?
    } else {
        let selected_run = select_run_for_resume_interactively(apply_override)?;
        RunWorkspace::open(&selected_run)?
    };

    if workspace.last_completed_stage() == Some(RunStage::Completed) && !apply_override {
        return Err(AppError::Execution(format!(
            "run '{}' is already completed",
            workspace.run_id()
        )));
    }

    let mut config = workspace.load_config()?;
    apply_resume_overrides(
        &mut config,
        apply_override,
        verbosity_override,
        quiet_override,
    );
    let verbosity = Verbosity::new(config.verbose, config.debug, config.quiet);
    verbosity.run_line(
        "RESUME",
        format!(
            "run_id={} mode={} state_dir={}",
            verbosity.accent(workspace.run_id()),
            if config.dry_run { "preview" } else { "apply" },
            verbosity.muted(workspace.root_dir().display().to_string())
        ),
    );
    run_with_workspace(config, &mut workspace).await
}
