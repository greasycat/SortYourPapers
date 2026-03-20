use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
};

use crate::{
    config,
    config::AppConfig,
    error::{AppError, Result},
    llm::LlmUsageSummary,
    papers::SynthesizeCategoriesState,
    report::RunReport,
    session::{RunStage, RunSummary, RunWorkspace, run_with_workspace, stage_sequence},
    terminal::{self, Verbosity},
};

const KEYWORD_BATCH_PROGRESS_FILE: &str = "06-extract-keywords-partial-batches.json";
const TAXONOMY_BATCH_PROGRESS_FILE: &str = "07-synthesize-categories-partial-batches.json";
const PLACEMENT_BATCH_PROGRESS_FILE: &str = "09-generate-placements-partial-batches.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RerunArtifact {
    KeywordBatchProgress,
    TaxonomyBatchProgress,
    PlacementBatchProgress,
}

impl RerunArtifact {
    pub(crate) fn file_name(self) -> &'static str {
        match self {
            Self::KeywordBatchProgress => KEYWORD_BATCH_PROGRESS_FILE,
            Self::TaxonomyBatchProgress => TAXONOMY_BATCH_PROGRESS_FILE,
            Self::PlacementBatchProgress => PLACEMENT_BATCH_PROGRESS_FILE,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::KeywordBatchProgress => "keyword batch progress",
            Self::TaxonomyBatchProgress => "taxonomy batch progress",
            Self::PlacementBatchProgress => "placement batch progress",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RerunImpact {
    pub(crate) start_stage: RunStage,
    pub(crate) previous_last_completed_stage: Option<RunStage>,
    pub(crate) cleared_stage_files: Vec<RunStage>,
    pub(crate) cleared_artifacts: Vec<RerunArtifact>,
    pub(crate) report_reset_sections: Vec<&'static str>,
}

impl RerunImpact {
    pub(crate) fn lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!(
                "Restart stage: {} | {}",
                rerun_stage_name(self.start_stage),
                self.start_stage.description()
            ),
            format!(
                "Saved progress before restart: {}",
                self.previous_last_completed_stage.map_or_else(
                    || "none (the run will restart from scratch)".to_string(),
                    |stage| { format!("{} | {}", rerun_stage_name(stage), stage.description()) }
                )
            ),
            String::new(),
            "Stage files removed:".to_string(),
        ];

        if self.cleared_stage_files.is_empty() {
            lines.push("  none".to_string());
        } else {
            for stage in &self.cleared_stage_files {
                lines.push(format!(
                    "  {} | {}",
                    rerun_stage_name(*stage),
                    stage.description()
                ));
            }
        }

        lines.push(String::new());
        lines.push("Extra artifacts cleared:".to_string());
        if self.cleared_artifacts.is_empty() {
            lines.push("  none".to_string());
        } else {
            for artifact in &self.cleared_artifacts {
                lines.push(format!("  {}", artifact.label()));
            }
        }

        lines.push(String::new());
        lines.push("Report sections reset:".to_string());
        if self.report_reset_sections.is_empty() {
            lines.push("  none".to_string());
        } else {
            for section in &self.report_reset_sections {
                lines.push(format!("  {section}"));
            }
        }

        lines
    }
}

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

/// Rewinds a saved run to a selected stage and reruns from there.
///
/// # Errors
/// Returns an error when the run or stage cannot be selected, when rerun state
/// reset fails, or when the rerun pipeline fails.
pub async fn rerun_run(
    run_id: Option<String>,
    stage: Option<RunStage>,
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

/// Displays the saved synthesized category tree for a completed session.
///
/// # Errors
/// Returns an error when the run cannot be selected or opened, the run is not
/// completed, or the synthesized category stage is unavailable.
pub fn review_session(run_id: Option<String>) -> Result<()> {
    let workspace = if let Some(run_id) = run_id {
        RunWorkspace::open(&run_id)?
    } else {
        let selected_run = select_completed_run_for_review_interactively()?;
        RunWorkspace::open(&selected_run)?
    };

    if workspace.last_completed_stage() != Some(RunStage::Completed) {
        return Err(AppError::Execution(format!(
            "run '{}' is not completed",
            workspace.run_id()
        )));
    }

    let categories = workspace
        .load_stage::<SynthesizeCategoriesState>(RunStage::SynthesizeCategories)?
        .ok_or_else(|| {
            AppError::Execution(format!(
                "run '{}' has no saved synthesized categories",
                workspace.run_id()
            ))
        })?;
    let verbosity = Verbosity::new(false, false, false);
    println!(
        "Reviewing {} from {}",
        workspace.run_id(),
        workspace.root_dir().display()
    );
    terminal::report::print_category_tree(&categories.categories, verbosity);
    Ok(())
}

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

/// Initializes the default XDG config file for the application.
///
/// # Errors
/// Returns an error when the config path cannot be resolved or written.
pub fn init_config(force: bool) -> Result<PathBuf> {
    config::init_xdg_config(force)
}

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

fn select_run_for_resume_interactively(apply_override: bool) -> Result<String> {
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

fn select_run_for_rerun_interactively() -> Result<String> {
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

fn select_run_for_removal_interactively(runs: &[RunSummary]) -> Result<String> {
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

fn select_completed_run_for_review_interactively() -> Result<String> {
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

fn select_stage_for_rerun_interactively(config: &AppConfig) -> Result<RunStage> {
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

fn runs_matching_ids<'a>(runs: &'a [RunSummary], run_ids: &[String]) -> Vec<&'a RunSummary> {
    run_ids
        .iter()
        .filter_map(|run_id| runs.iter().find(|run| run.run_id == *run_id))
        .collect()
}

fn confirm_session_deletion(runs: &[&RunSummary], prompt: &str) -> Result<()> {
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

fn stdin_is_interactive() -> bool {
    io::stdin().is_terminal()
}

fn available_rerun_stages(config: &AppConfig) -> Vec<RunStage> {
    stage_sequence(config.rebuild && config.output.exists(), true)
}

fn validate_rerun_stage(stage: RunStage, stages: &[RunStage]) -> Result<RunStage> {
    stages
        .iter()
        .copied()
        .find(|candidate| *candidate == stage)
        .ok_or_else(|| {
            AppError::Execution(format!(
                "stage '{}' is not available for this run",
                rerun_stage_name(stage)
            ))
        })
}

pub(crate) fn describe_rerun_impact(
    config: &AppConfig,
    start_stage: RunStage,
) -> Result<RerunImpact> {
    let stages = available_rerun_stages(config);
    let Some(start_index) = stages.iter().position(|stage| *stage == start_stage) else {
        return Err(AppError::Execution(format!(
            "stage '{}' is not available for this run",
            rerun_stage_name(start_stage)
        )));
    };

    let mut cleared_artifacts = Vec::new();
    if start_index
        <= stages
            .iter()
            .position(|stage| *stage == RunStage::ExtractKeywords)
            .unwrap_or(usize::MAX)
    {
        cleared_artifacts.push(RerunArtifact::KeywordBatchProgress);
    }
    if start_index
        <= stages
            .iter()
            .position(|stage| *stage == RunStage::SynthesizeCategories)
            .unwrap_or(usize::MAX)
    {
        cleared_artifacts.push(RerunArtifact::TaxonomyBatchProgress);
    }
    if start_index
        <= stages
            .iter()
            .position(|stage| *stage == RunStage::GeneratePlacements)
            .unwrap_or(usize::MAX)
    {
        cleared_artifacts.push(RerunArtifact::PlacementBatchProgress);
    }

    Ok(RerunImpact {
        start_stage,
        previous_last_completed_stage: start_index.checked_sub(1).map(|index| stages[index]),
        cleared_stage_files: stages
            .iter()
            .copied()
            .skip(start_index)
            .filter(|stage| stage.file_name().is_some())
            .collect(),
        cleared_artifacts,
        report_reset_sections: report_reset_sections(start_stage),
    })
}

fn rerun_stage_name(stage: RunStage) -> &'static str {
    match stage {
        RunStage::DiscoverInput => "discover-input",
        RunStage::DiscoverOutput => "discover-output",
        RunStage::Dedupe => "dedupe",
        RunStage::FilterSize => "filter-size",
        RunStage::ExtractText => "extract-text",
        RunStage::BuildLlmClient => "build-llm-client",
        RunStage::ExtractKeywords => "extract-keywords",
        RunStage::SynthesizeCategories => "synthesize-categories",
        RunStage::InspectOutput => "inspect-output",
        RunStage::GeneratePlacements => "generate-placements",
        RunStage::BuildPlan => "build-plan",
        RunStage::ExecutePlan => "execute-plan",
        RunStage::Completed => "completed",
    }
}

fn prepare_workspace_for_rerun(
    workspace: &mut RunWorkspace,
    config: &AppConfig,
    start_stage: RunStage,
) -> Result<()> {
    let impact = describe_rerun_impact(config, start_stage)?;

    for stage in impact.cleared_stage_files.iter().copied() {
        workspace.remove_stage_file(stage)?;
    }
    workspace.set_last_completed_stage(impact.previous_last_completed_stage)?;

    for artifact in impact.cleared_artifacts {
        workspace.remove_artifact(artifact.file_name())?;
    }

    let mut report = workspace
        .load_report()?
        .unwrap_or_else(|| RunReport::new(config.dry_run));
    reset_report_for_rerun(&mut report, start_stage);
    workspace.save_report(&report)?;
    Ok(())
}

fn reset_report_for_rerun(report: &mut RunReport, start_stage: RunStage) {
    match start_stage {
        RunStage::DiscoverInput
        | RunStage::DiscoverOutput
        | RunStage::Dedupe
        | RunStage::FilterSize
        | RunStage::ExtractText => {
            report.scanned = 0;
            report.processed = 0;
            report.skipped = 0;
            report.failed = 0;
            report.actions.clear();
            report.llm_usage.keywords = LlmUsageSummary::default();
            report.llm_usage.taxonomy = LlmUsageSummary::default();
            report.llm_usage.placements = LlmUsageSummary::default();
        }
        RunStage::BuildLlmClient | RunStage::ExtractKeywords => {
            report.actions.clear();
            report.llm_usage.keywords = LlmUsageSummary::default();
            report.llm_usage.taxonomy = LlmUsageSummary::default();
            report.llm_usage.placements = LlmUsageSummary::default();
        }
        RunStage::SynthesizeCategories => {
            report.actions.clear();
            report.llm_usage.taxonomy = LlmUsageSummary::default();
            report.llm_usage.placements = LlmUsageSummary::default();
        }
        RunStage::InspectOutput | RunStage::GeneratePlacements => {
            report.actions.clear();
            report.llm_usage.placements = LlmUsageSummary::default();
        }
        RunStage::BuildPlan => {
            report.actions.clear();
        }
        RunStage::ExecutePlan | RunStage::Completed => {}
    }
}

fn report_reset_sections(start_stage: RunStage) -> Vec<&'static str> {
    match start_stage {
        RunStage::DiscoverInput
        | RunStage::DiscoverOutput
        | RunStage::Dedupe
        | RunStage::FilterSize
        | RunStage::ExtractText => vec![
            "scan and extraction counters",
            "planned actions",
            "keyword LLM usage",
            "taxonomy LLM usage",
            "placement LLM usage",
        ],
        RunStage::BuildLlmClient | RunStage::ExtractKeywords => vec![
            "planned actions",
            "keyword LLM usage",
            "taxonomy LLM usage",
            "placement LLM usage",
        ],
        RunStage::SynthesizeCategories => vec![
            "planned actions",
            "taxonomy LLM usage",
            "placement LLM usage",
        ],
        RunStage::InspectOutput | RunStage::GeneratePlacements => {
            vec!["planned actions", "placement LLM usage"]
        }
        RunStage::BuildPlan => vec!["planned actions"],
        RunStage::ExecutePlan | RunStage::Completed => Vec::new(),
    }
}

fn format_run_summary(index: usize, run: &RunSummary) -> String {
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
