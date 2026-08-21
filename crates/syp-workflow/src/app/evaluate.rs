//! Scoring a finished run against a curated test set.
//!
//! Turns "does the taxonomy look right?" into numbers by joining a run's plan
//! to the reference labels the curated set already carries. No LLM is involved:
//! the run has happened, and this only compares two partitions of the same
//! papers.

use std::{collections::BTreeMap, path::PathBuf};

use syp_library::{
    eval::{ClusterAssignment, ClusteringMetrics, score},
    testsets::load_manifest_from_path,
};

use crate::{
    defaults::DEFAULT_REFERENCE_MANIFEST_PATH,
    error::{AppError, Result},
    report::RunReport,
    session::RunWorkspace,
    terminal::Verbosity,
};

/// Which reference label a run is scored against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelField {
    /// Broad field, such as `Computer Science`.
    Category,
    /// arXiv subcategory, such as `cs.LG`.
    Subcategory,
}

impl LabelField {
    fn name(self) -> &'static str {
        match self {
            Self::Category => "category",
            Self::Subcategory => "subcategory",
        }
    }
}

/// One run scored at both label granularities.
#[derive(Debug, Clone)]
pub struct RunEvaluation {
    pub run_id: String,
    pub scored_papers: usize,
    /// Papers in the plan that the manifest does not cover.
    pub unlabelled_papers: usize,
    pub by_label: Vec<(LabelField, ClusteringMetrics)>,
}

impl RunEvaluation {
    #[must_use]
    pub fn summary_lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "run {} | scored {} paper(s) | {} without a reference label",
            self.run_id, self.scored_papers, self.unlabelled_papers
        )];
        for (field, metrics) in &self.by_label {
            lines.push(String::new());
            lines.push(format!("against {}:", field.name()));
            lines.extend(
                metrics
                    .summary_lines()
                    .into_iter()
                    .map(|line| format!("  {line}")),
            );
        }
        lines
    }
}

/// Scores a saved run against a curated manifest.
///
/// # Errors
/// Returns an error when the run or manifest cannot be read, or when the run's
/// plan and the manifest share no papers.
pub fn evaluate_run(
    run_id: Option<String>,
    manifest_path: Option<PathBuf>,
    verbosity: Verbosity,
) -> Result<RunEvaluation> {
    let workspace = match run_id {
        Some(run_id) => RunWorkspace::open(&run_id)?,
        None => RunWorkspace::open_latest()?,
    };
    evaluate_workspace(&workspace, manifest_path, verbosity)
}

/// Scores one opened run against a curated manifest.
///
/// # Errors
/// Returns an error when the run or manifest cannot be read, or when the run's
/// plan and the manifest share no papers.
pub(crate) fn evaluate_workspace(
    workspace: &RunWorkspace,
    manifest_path: Option<PathBuf>,
    verbosity: Verbosity,
) -> Result<RunEvaluation> {
    let report = workspace.load_report()?.ok_or_else(|| {
        AppError::Execution(format!("run {} has no saved report", workspace.run_id()))
    })?;
    let config = workspace.load_config()?;

    let manifest_path =
        manifest_path.unwrap_or_else(|| PathBuf::from(DEFAULT_REFERENCE_MANIFEST_PATH));
    let manifest = load_manifest_from_path(&manifest_path)?;
    let labels = manifest
        .papers
        .iter()
        .map(|paper| {
            (
                paper.paper_id.clone(),
                (paper.category.clone(), paper.subcategory.clone()),
            )
        })
        .collect::<BTreeMap<_, _>>();

    verbosity.stage_line(
        "eval",
        format!(
            "scoring run {} against {} label(s) from {}",
            workspace.run_id(),
            labels.len(),
            manifest_path.display()
        ),
    );

    let mut category = Vec::new();
    let mut subcategory = Vec::new();
    let mut unlabelled = 0;

    for (paper_id, folder) in planned_folders(&report, &config.output) {
        match labels.get(&paper_id) {
            Some((broad, fine)) => {
                category.push(ClusterAssignment {
                    item: paper_id.clone(),
                    predicted: folder.clone(),
                    truth: broad.clone(),
                });
                subcategory.push(ClusterAssignment {
                    item: paper_id,
                    predicted: folder,
                    truth: fine.clone(),
                });
            }
            None => unlabelled += 1,
        }
    }

    let scored_papers = subcategory.len();
    let mut by_label = Vec::new();
    if let Some(metrics) = score(&category) {
        by_label.push((LabelField::Category, metrics));
    }
    if let Some(metrics) = score(&subcategory) {
        by_label.push((LabelField::Subcategory, metrics));
    }

    if by_label.is_empty() {
        return Err(AppError::Execution(format!(
            "run {} placed no papers that appear in {}",
            workspace.run_id(),
            manifest_path.display()
        )));
    }

    Ok(RunEvaluation {
        run_id: workspace.run_id().to_string(),
        scored_papers,
        unlabelled_papers: unlabelled,
        by_label,
    })
}

/// Maps each planned move to (paper id, destination folder).
///
/// The paper id is the file stem, which is how the curated set names the PDFs
/// it materializes. The folder is the destination's parent relative to the
/// output root, so the run's own output path never leaks into cluster names.
fn planned_folders(report: &RunReport, output_root: &std::path::Path) -> Vec<(String, String)> {
    report
        .actions
        .iter()
        .filter_map(|action| {
            let paper_id = action.destination.file_stem()?.to_str()?.to_string();
            let parent = action.destination.parent()?;
            let folder = parent
                .strip_prefix(output_root)
                .unwrap_or(parent)
                .to_string_lossy()
                .to_string();
            let folder = if folder.is_empty() {
                ".".to_string()
            } else {
                folder
            };
            Some((paper_id, folder))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use syp_library::report::{FileAction, PlanAction};

    use super::*;

    fn action(destination: &str) -> PlanAction {
        PlanAction {
            source: PathBuf::from("/inbox/x.pdf"),
            destination: PathBuf::from(destination),
            action: FileAction::Move,
        }
    }

    #[test]
    fn planned_folders_use_paths_relative_to_the_output_root() {
        let report = RunReport {
            actions: vec![
                action("/library/Computer Science/Vision/arxiv-1409.1556.pdf"),
                action("/library/Physics/arxiv-9905013.pdf"),
            ],
            ..RunReport::new(true)
        };

        let folders = planned_folders(&report, Path::new("/library"));

        assert_eq!(
            folders,
            vec![
                (
                    "arxiv-1409.1556".to_string(),
                    "Computer Science/Vision".to_string()
                ),
                ("arxiv-9905013".to_string(), "Physics".to_string()),
            ]
        );
    }

    #[test]
    fn papers_placed_at_the_output_root_form_one_folder() {
        let report = RunReport {
            actions: vec![action("/library/arxiv-1.pdf")],
            ..RunReport::new(true)
        };

        let folders = planned_folders(&report, Path::new("/library"));

        assert_eq!(folders, vec![("arxiv-1".to_string(), ".".to_string())]);
    }
}

#[cfg(test)]
mod workspace_tests {
    use std::path::{Path, PathBuf};

    use syp_library::report::{FileAction, PlanAction};
    use tempfile::tempdir;

    use super::*;
    use crate::{config::AppConfig, session::RunWorkspace};

    const MANIFEST: &str = "../../assets/testsets/scijudgebench-diverse.toml";

    /// A config good enough to open a workspace; only `output` is read here.
    fn sample_eval_config() -> AppConfig {
        crate::config::resolve_config(crate::inputs::RunOverrides::default())
            .expect("default config should resolve")
    }

    fn shipped_manifest() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(MANIFEST)
    }

    fn placed(output: &Path, folder: &str, paper_id: &str) -> PlanAction {
        PlanAction {
            source: PathBuf::from(format!("/inbox/{paper_id}.pdf")),
            destination: output.join(folder).join(format!("{paper_id}.pdf")),
            action: FileAction::Move,
        }
    }

    /// Builds a run whose plan files real curated papers, then scores it the
    /// way `syp eval` does.
    #[test]
    fn a_run_that_files_papers_by_field_scores_well_against_the_curated_labels() {
        let temp = tempdir().expect("tempdir");
        let cache_root = temp.path().join("cache");
        let output = temp.path().join("library");
        let mut config = sample_eval_config();
        config.output = output.clone();

        let workspace =
            RunWorkspace::create_with_cache_root_for_tests(temp.path(), &cache_root, &config)
                .expect("workspace");

        let mut report = RunReport::new(true);
        // Real paper ids from the shipped curated set, filed by broad field.
        report.actions = vec![
            placed(&output, "Computer Science", "arxiv-1412.6980"),
            placed(&output, "Computer Science", "arxiv-1409.1556"),
            placed(&output, "Physics", "arxiv-hep-th-9711200"),
        ];
        workspace.save_report(&report).expect("save report");

        let evaluation = evaluate_workspace(
            &workspace,
            Some(shipped_manifest()),
            Verbosity::new(false, false, true),
        );

        let evaluation = evaluation.expect("evaluation");
        assert!(evaluation.scored_papers >= 2, "{evaluation:?}");
        let (_, category) = evaluation
            .by_label
            .iter()
            .find(|(field, _)| *field == LabelField::Category)
            .expect("category metrics");
        assert!(
            category.homogeneity > 0.0,
            "filing by field should explain some label structure: {category:?}"
        );
    }

    #[test]
    fn a_plan_with_no_curated_papers_is_reported_rather_than_scored() {
        let temp = tempdir().expect("tempdir");
        let cache_root = temp.path().join("cache");
        let output = temp.path().join("library");
        let mut config = sample_eval_config();
        config.output = output.clone();
        let workspace =
            RunWorkspace::create_with_cache_root_for_tests(temp.path(), &cache_root, &config)
                .expect("workspace");

        let mut report = RunReport::new(true);
        report.actions = vec![placed(&output, "Somewhere", "not-a-curated-paper")];
        workspace.save_report(&report).expect("save report");

        let err = evaluate_workspace(
            &workspace,
            Some(shipped_manifest()),
            Verbosity::new(false, false, true),
        )
        .expect_err("nothing to score");

        assert!(err.to_string().contains("no papers"), "{err}");
    }
}
