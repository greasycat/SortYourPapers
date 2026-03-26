use std::io::{self, IsTerminal, Write};

use crate::{
    config::AppConfig,
    error::{AppError, Result},
    session::{RunStage, RunSummary, RunWorkspace},
};

use super::impact::{available_rerun_stages, rerun_stage_name};

pub(crate) fn apply_resume_overrides(
    config: &mut AppConfig,
    apply_override: bool,
    verbosity_override: u8,
    quiet_override: bool,
) {
    if apply_override {
        config.dry_run = false;
    }
    if verbosity_override > 0 {
        config.verbose = true;
        config.debug = verbosity_override > 1;
    }
    if quiet_override {
        config.quiet = true;
    }
}

pub(crate) fn select_run_for_resume_interactively(apply_override: bool) -> Result<String> {
    if !stdin_is_interactive() {
        return Err(AppError::Execution(
            "session resume requires a RUN_ID when stdin is not interactive".to_string(),
        ));
    }

    let runs = selectable_runs(RunWorkspace::list_runs()?, apply_override);
    if runs.is_empty() {
        let runs_root = RunWorkspace::runs_root()?;
        return Err(AppError::Execution(format!(
            "no {} runs found under {}",
            if apply_override {
                "selectable"
            } else {
                "resumable"
            },
            runs_root.display()
        )));
    }

    select_run_interactively(
        &runs,
        &format!(
            "Available sessions to resume (mode: {}):",
            if apply_override { "apply" } else { "preview" }
        ),
        &format!(
            "Choose a session to resume in {} mode by number or run id: ",
            if apply_override { "apply" } else { "preview" }
        ),
        "resume selection cancelled before a run was chosen",
    )
}

pub(crate) fn select_run_for_rerun_interactively() -> Result<String> {
    if !stdin_is_interactive() {
        return Err(AppError::Execution(
            "session rerun requires a RUN_ID when stdin is not interactive".to_string(),
        ));
    }

    let runs = RunWorkspace::list_runs()?;
    if runs.is_empty() {
        let runs_root = RunWorkspace::runs_root()?;
        return Err(AppError::Execution(format!(
            "no saved sessions found under {}",
            runs_root.display()
        )));
    }

    select_run_interactively(
        &runs,
        "Available sessions to rerun:",
        "Choose a session to rerun by number or run id: ",
        "rerun selection cancelled before a run was chosen",
    )
}

pub(crate) fn select_run_for_removal_interactively(runs: &[RunSummary]) -> Result<String> {
    if !stdin_is_interactive() {
        return Err(AppError::Execution(
            "session remove requires a RUN_ID when stdin is not interactive".to_string(),
        ));
    }

    select_run_interactively(
        runs,
        "Saved sessions:",
        "Choose a session to remove by number or run id: ",
        "session removal cancelled before a run was chosen",
    )
}

pub(crate) fn select_completed_run_for_review_interactively() -> Result<String> {
    if !stdin_is_interactive() {
        return Err(AppError::Execution(
            "session review requires a RUN_ID when stdin is not interactive".to_string(),
        ));
    }

    let runs = completed_runs(RunWorkspace::list_runs()?);
    if runs.is_empty() {
        let runs_root = RunWorkspace::runs_root()?;
        return Err(AppError::Execution(format!(
            "no completed runs found under {}",
            runs_root.display()
        )));
    }

    select_run_interactively(
        &runs,
        "Completed sessions:",
        "Choose a completed session to review by number or run id: ",
        "review selection cancelled before a run was chosen",
    )
}

pub(crate) fn select_stage_for_rerun_interactively(config: &AppConfig) -> Result<RunStage> {
    if !stdin_is_interactive() {
        return Err(AppError::Execution(
            "session rerun requires a STAGE when stdin is not interactive".to_string(),
        ));
    }

    let stages = available_rerun_stages(config);
    eprintln!("Available rerun stages:");
    for (index, stage) in stages.iter().enumerate() {
        eprintln!(
            "  {}. {} | {}",
            index + 1,
            rerun_stage_name(*stage),
            stage.description()
        );
    }

    let mut stderr = io::stderr();
    let mut input = String::new();
    loop {
        write!(
            stderr,
            "Choose a stage to restart from by number or stage name: "
        )?;
        stderr.flush()?;

        input.clear();
        let bytes_read = io::stdin().read_line(&mut input)?;
        if bytes_read == 0 {
            return Err(AppError::Execution(
                "rerun stage selection cancelled before a stage was chosen".to_string(),
            ));
        }

        match resolve_stage_selection(input.trim(), &stages) {
            Ok(stage) => return Ok(stage),
            Err(err) => eprintln!("error: {err}"),
        }
    }
}

pub(crate) fn selectable_runs(runs: Vec<RunSummary>, apply_override: bool) -> Vec<RunSummary> {
    if apply_override {
        runs
    } else {
        runs.into_iter()
            .filter(|run| run.last_completed_stage != Some(RunStage::Completed))
            .collect()
    }
}

pub(crate) fn completed_runs(runs: Vec<RunSummary>) -> Vec<RunSummary> {
    runs.into_iter()
        .filter(|run| run.last_completed_stage == Some(RunStage::Completed))
        .collect()
}

pub(crate) fn resolve_run_selection<'a>(
    selection: &str,
    runs: &'a [RunSummary],
) -> Result<&'a str> {
    if selection.is_empty() {
        return Err(AppError::Execution(
            "enter a run number or run id".to_string(),
        ));
    }

    if let Ok(index) = selection.parse::<usize>() {
        let Some(run) = index.checked_sub(1).and_then(|position| runs.get(position)) else {
            return Err(AppError::Execution(format!(
                "selection '{selection}' is out of range"
            )));
        };
        return Ok(&run.run_id);
    }

    runs.iter()
        .find(|run| run.run_id == selection)
        .map(|run| run.run_id.as_str())
        .ok_or_else(|| AppError::Execution(format!("run '{selection}' was not found")))
}

pub(crate) fn validate_run_ids(run_ids: &[String], runs: &[RunSummary]) -> Result<Vec<String>> {
    let mut validated = Vec::new();
    for run_id in run_ids {
        let resolved = runs
            .iter()
            .find(|run| run.run_id == *run_id)
            .map(|run| run.run_id.clone())
            .ok_or_else(|| AppError::Execution(format!("run '{run_id}' was not found")))?;
        if !validated.iter().any(|existing| existing == &resolved) {
            validated.push(resolved);
        }
    }
    Ok(validated)
}

pub(crate) fn resolve_stage_selection(selection: &str, stages: &[RunStage]) -> Result<RunStage> {
    if selection.is_empty() {
        return Err(AppError::Execution(
            "enter a stage number or stage name".to_string(),
        ));
    }

    if let Ok(index) = selection.parse::<usize>() {
        let Some(stage) = index
            .checked_sub(1)
            .and_then(|position| stages.get(position))
        else {
            return Err(AppError::Execution(format!(
                "selection '{selection}' is out of range"
            )));
        };
        return Ok(*stage);
    }

    stages
        .iter()
        .copied()
        .find(|stage| rerun_stage_name(*stage) == selection)
        .ok_or_else(|| AppError::Execution(format!("stage '{selection}' was not found")))
}

pub(crate) fn runs_matching_ids<'a>(
    runs: &'a [RunSummary],
    run_ids: &[String],
) -> Vec<&'a RunSummary> {
    run_ids
        .iter()
        .filter_map(|run_id| runs.iter().find(|run| run.run_id == *run_id))
        .collect()
}

pub(crate) fn confirm_session_deletion(runs: &[&RunSummary], prompt: &str) -> Result<()> {
    if !stdin_is_interactive() {
        return Ok(());
    }

    eprintln!("Selected sessions:");
    for run in runs {
        eprintln!("  {}", format_run_summary(0, run));
    }

    let mut stderr = io::stderr();
    write!(stderr, "{prompt}")?;
    stderr.flush()?;

    let mut input = String::new();
    let bytes_read = io::stdin().read_line(&mut input)?;
    if bytes_read == 0 {
        return Err(AppError::Execution(
            "session deletion cancelled before confirmation".to_string(),
        ));
    }

    let confirmation = input.trim().to_ascii_lowercase();
    if confirmation == "y" || confirmation == "yes" {
        return Ok(());
    }

    Err(AppError::Execution(
        "session deletion cancelled".to_string(),
    ))
}

pub(crate) fn format_run_summary(index: usize, run: &RunSummary) -> String {
    let stage = run
        .last_completed_stage
        .map_or_else(|| "NotStarted".to_string(), |stage| format!("{stage:?}"));
    let latest = if run.is_latest { " latest" } else { "" };
    let prefix = if index == 0 {
        "-".to_string()
    } else {
        format!("{index}.")
    };
    format!(
        "{prefix} {} | stage={} | cwd={} | created_unix_ms={}{}",
        run.run_id,
        stage,
        run.cwd.display(),
        run.created_unix_ms,
        latest
    )
}

fn select_run_interactively(
    runs: &[RunSummary],
    heading: &str,
    prompt: &str,
    cancelled_message: &str,
) -> Result<String> {
    eprintln!("{heading}");
    for (index, run) in runs.iter().enumerate() {
        eprintln!("  {}", format_run_summary(index + 1, run));
    }

    let mut stderr = io::stderr();
    let mut input = String::new();
    loop {
        write!(stderr, "{prompt}")?;
        stderr.flush()?;

        input.clear();
        let bytes_read = io::stdin().read_line(&mut input)?;
        if bytes_read == 0 {
            return Err(AppError::Execution(cancelled_message.to_string()));
        }

        match resolve_run_selection(input.trim(), runs) {
            Ok(run_id) => return Ok(run_id.to_string()),
            Err(err) => eprintln!("error: {err}"),
        }
    }
}

fn stdin_is_interactive() -> bool {
    io::stdin().is_terminal()
}
