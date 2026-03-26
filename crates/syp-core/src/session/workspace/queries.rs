use std::{env, fs, path::Path};

use crate::{
    error::Result, llm::LlmProvider, papers::SynthesizeCategoriesState, report::RunReport,
};

use super::{
    paths::{LATEST_RUN_FILE, MANIFEST_FILE, resume_workspace_root, runs_root_in},
    store::read_json,
    types::{
        RunManifest, RunStage, RunSummary, RunWorkspace, SessionConfigSummary, SessionDetails,
        SessionStatusSummary,
    },
};

impl RunWorkspace {
    pub fn list_runs() -> Result<Vec<RunSummary>> {
        let cwd = env::current_dir()?;
        list_runs_in(&cwd)
    }

    pub fn runs_root() -> Result<std::path::PathBuf> {
        let cwd = env::current_dir()?;
        runs_root_in(&cwd)
    }

    pub fn inspect_run(run: &RunSummary) -> Result<SessionDetails> {
        let workspace = Self::open_in(&run.cwd, &run.run_id)?;
        workspace.inspect_with_summary(run.clone())
    }

    pub fn inspect_run_status(run: &RunSummary) -> Result<SessionStatusSummary> {
        let workspace = Self::open_in(&run.cwd, &run.run_id)?;
        Ok(workspace.inspect_status(run))
    }

    pub fn inspect(&self) -> Result<SessionDetails> {
        self.inspect_with_summary(self.summary(false))
    }

    fn inspect_with_summary(&self, run: RunSummary) -> Result<SessionDetails> {
        let config = self.load_config()?;
        let report = self.load_report()?;
        let taxonomy = self
            .load_stage::<SynthesizeCategoriesState>(RunStage::SynthesizeCategories)?
            .map(|state| state.categories);
        let status = status_from_report(&run, report.as_ref());

        Ok(SessionDetails {
            run,
            config: SessionConfigSummary {
                dry_run: config.dry_run,
                llm_provider: llm_provider_label(config.llm_provider).to_string(),
                llm_model: config.llm_model,
            },
            status,
            report,
            taxonomy,
            available_stage_artifacts: available_stage_artifacts(&self.root_dir),
        })
    }

    fn inspect_status(&self, run: &RunSummary) -> SessionStatusSummary {
        let report = self.load_report().ok().flatten();
        status_from_report(run, report.as_ref())
    }

    fn summary(&self, is_latest: bool) -> RunSummary {
        RunSummary {
            run_id: self.manifest.run_id.clone(),
            created_unix_ms: self.manifest.created_unix_ms,
            cwd: self.manifest.cwd.clone(),
            last_completed_stage: self.manifest.last_completed_stage,
            is_latest,
        }
    }

    #[cfg(test)]
    pub(crate) fn list_runs_with_cache_root(
        base_dir: &Path,
        cache_root: &Path,
    ) -> Result<Vec<RunSummary>> {
        list_runs_from_cache_root(base_dir, cache_root)
    }
}

pub(super) fn list_runs_in(base_dir: &Path) -> Result<Vec<RunSummary>> {
    let runs_root = runs_root_in(base_dir)?;
    if !runs_root.exists() {
        return Ok(Vec::new());
    }
    list_runs_from_cache_root(base_dir, &resume_workspace_root(base_dir)?)
}

pub(super) fn list_runs_from_cache_root(
    base_dir: &Path,
    cache_root: &Path,
) -> Result<Vec<RunSummary>> {
    let runs_root = cache_root.join(super::paths::RUNS_DIR);
    if !runs_root.exists() {
        return Ok(Vec::new());
    }

    let latest_run_id = fs::read_to_string(cache_root.join(LATEST_RUN_FILE))
        .ok()
        .map(|run_id| run_id.trim().to_string());
    let mut runs = Vec::new();
    for entry in fs::read_dir(&runs_root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        let root_dir = entry.path();
        let manifest_path = root_dir.join(MANIFEST_FILE);
        if !manifest_path.exists() {
            continue;
        }

        let manifest: RunManifest = read_json(&manifest_path)?;
        if manifest.cwd != base_dir {
            continue;
        }

        let is_latest = latest_run_id
            .as_deref()
            .is_some_and(|latest| latest == manifest.run_id);
        runs.push(RunSummary {
            run_id: manifest.run_id,
            created_unix_ms: manifest.created_unix_ms,
            cwd: manifest.cwd,
            last_completed_stage: manifest.last_completed_stage,
            is_latest,
        });
    }

    runs.sort_by(|left, right| right.created_unix_ms.cmp(&left.created_unix_ms));
    Ok(runs)
}

fn available_stage_artifacts(root_dir: &Path) -> Vec<RunStage> {
    persisted_run_stages()
        .iter()
        .copied()
        .filter(|stage| {
            stage
                .file_name()
                .is_some_and(|file_name| root_dir.join(file_name).exists())
        })
        .collect()
}

fn persisted_run_stages() -> &'static [RunStage] {
    const PERSISTED_STAGES: [RunStage; 10] = [
        RunStage::DiscoverInput,
        RunStage::DiscoverOutput,
        RunStage::Dedupe,
        RunStage::FilterSize,
        RunStage::ExtractText,
        RunStage::ExtractKeywords,
        RunStage::SynthesizeCategories,
        RunStage::InspectOutput,
        RunStage::GeneratePlacements,
        RunStage::BuildPlan,
    ];

    &PERSISTED_STAGES
}

fn llm_provider_label(provider: LlmProvider) -> &'static str {
    match provider {
        LlmProvider::Openai => "openai",
        LlmProvider::Ollama => "ollama",
        LlmProvider::Gemini => "gemini",
    }
}

pub(super) fn status_from_report(
    run: &RunSummary,
    report: Option<&RunReport>,
) -> SessionStatusSummary {
    SessionStatusSummary {
        is_completed: run.last_completed_stage == Some(RunStage::Completed),
        is_incomplete: run.last_completed_stage != Some(RunStage::Completed),
        is_failed_looking: run.last_completed_stage != Some(RunStage::Completed)
            || report.is_some_and(|saved| saved.failed > 0),
    }
}
