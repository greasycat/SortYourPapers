use std::time::Duration;

use crate::{
    config::AppConfig,
    session::{RunStage, RunWorkspace},
    terminal::{Verbosity, format_duration},
};

pub(crate) struct StagePlan {
    stages: Vec<RunStage>,
}

impl StagePlan {
    pub(crate) fn new(config: &AppConfig, include_llm_client: bool) -> Self {
        Self {
            stages: stage_sequence(config.rebuild && config.output.exists(), include_llm_client),
        }
    }

    pub(crate) fn announce(&self, verbosity: Verbosity, stage: RunStage) {
        if verbosity.quiet() || verbosity.verbose_enabled() {
            return;
        }
        let Some(index) = self.stages.iter().position(|candidate| *candidate == stage) else {
            return;
        };
        verbosity.info(format!(
            "[{}/{}] {}",
            index + 1,
            self.stages.len(),
            format_stage_description(verbosity, stage.description())
        ));
    }
}

pub(crate) fn format_stage_description(verbosity: Verbosity, description: &str) -> String {
    let Some((verb, remainder)) = description.split_once(' ') else {
        return verbosity.accent(description);
    };
    format!("{} {}", verbosity.accent(verb), remainder)
}

pub(crate) fn stage_sequence(
    include_discover_output: bool,
    include_llm_client: bool,
) -> Vec<RunStage> {
    let mut stages = vec![
        RunStage::DiscoverInput,
        RunStage::Dedupe,
        RunStage::FilterSize,
        RunStage::ExtractText,
    ];
    if include_discover_output {
        stages.insert(1, RunStage::DiscoverOutput);
    }
    if include_llm_client {
        stages.push(RunStage::BuildLlmClient);
    }
    stages.extend([
        RunStage::ExtractKeywords,
        RunStage::SynthesizeCategories,
        RunStage::InspectOutput,
        RunStage::GeneratePlacements,
        RunStage::BuildPlan,
        RunStage::ExecutePlan,
    ]);
    stages
}

pub(crate) fn log_stage(verbosity: Verbosity, stage: &str, message: String) {
    verbosity.stage_line(stage, message);
}

pub(crate) fn log_resume(verbosity: Verbosity, stage: &str, workspace: &RunWorkspace) {
    verbosity.debug_line(
        "RESUME",
        format!(
            "stage={} state_dir={}",
            verbosity.accent(stage),
            workspace.root_dir().display()
        ),
    );
}

pub(crate) fn log_timing(verbosity: Verbosity, stage: &str, elapsed: Duration) {
    if verbosity.verbose_enabled() {
        verbosity.debug_line(
            "TIMING",
            format!(
                "stage={} elapsed={}",
                verbosity.accent(stage),
                format_duration(elapsed)
            ),
        );
    }
}
