use crate::{
    error::Result,
    report::RunReport,
    session::{RunWorkspace, run_with_workspace},
    terminal::Verbosity,
};

use super::{
    impact::{
        available_rerun_stages, prepare_workspace_for_rerun, rerun_stage_name, validate_rerun_stage,
    },
    selection::{
        apply_resume_overrides, select_run_for_rerun_interactively,
        select_stage_for_rerun_interactively,
    },
};

/// Rewinds a saved run to a selected stage and reruns from there.
///
/// # Errors
/// Returns an error when the run or stage cannot be selected, when rerun state
/// reset fails, or when the rerun pipeline fails.
pub async fn rerun_run(
    run_id: Option<String>,
    stage: Option<crate::session::RunStage>,
    apply_override: bool,
    verbosity_override: u8,
    quiet_override: bool,
) -> Result<RunReport> {
    let selected_run_id = if let Some(run_id) = run_id {
        run_id
    } else {
        select_run_for_rerun_interactively()?
    };
    let mut workspace = RunWorkspace::open(&selected_run_id)?;
    let mut config = workspace.load_config()?;
    apply_resume_overrides(
        &mut config,
        apply_override,
        verbosity_override,
        quiet_override,
    );

    let selected_stage = if let Some(stage) = stage {
        validate_rerun_stage(stage, &available_rerun_stages(&config))?
    } else {
        select_stage_for_rerun_interactively(&config)?
    };

    prepare_workspace_for_rerun(&mut workspace, &config, selected_stage)?;
    let verbosity = Verbosity::new(config.verbose, config.debug, config.quiet);
    verbosity.run_line(
        "RERUN",
        format!(
            "run_id={} restart_stage={} mode={} state_dir={}",
            verbosity.accent(workspace.run_id()),
            verbosity.accent(rerun_stage_name(selected_stage)),
            if config.dry_run { "preview" } else { "apply" },
            verbosity.muted(workspace.root_dir().display().to_string())
        ),
    );
    run_with_workspace(config, &mut workspace).await
}
