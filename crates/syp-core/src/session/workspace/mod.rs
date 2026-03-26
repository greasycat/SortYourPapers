mod cleanup;
mod paths;
mod queries;
mod store;
mod types;

pub use types::{
    ExtractTextState, FilterSizeState, RunStage, RunSummary, RunWorkspace, SessionConfigSummary,
    SessionDetails, SessionStatusSummary, StageFailure,
};

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{
        ExtractTextState, FilterSizeState, RunStage, RunWorkspace, SessionConfigSummary,
        SessionStatusSummary, StageFailure,
    };
    use crate::config::{ApiKeySource, AppConfig};
    use crate::llm::LlmProvider;
    use crate::papers::placement::{PlacementAssistance, PlacementMode};
    use crate::papers::taxonomy::{
        CategoryTree, KeywordBatchProgress, TaxonomyAssistance, TaxonomyMode,
    };
    use crate::papers::{PdfCandidate, SynthesizeCategoriesState};
    use crate::report::RunReport;

    fn sample_config() -> AppConfig {
        AppConfig {
            input: "/tmp/in".into(),
            output: "/tmp/out".into(),
            recursive: false,
            max_file_size_mb: 8,
            page_cutoff: 5,
            pdf_extract_workers: 4,
            category_depth: 2,
            taxonomy_mode: TaxonomyMode::BatchMerge,
            taxonomy_assistance: TaxonomyAssistance::LlmOnly,
            taxonomy_batch_size: 3,
            reference_manifest_path: "assets/testsets/scijudgebench-diverse.toml".into(),
            reference_top_k: 5,
            use_current_folder_tree: false,
            placement_batch_size: 25,
            placement_assistance: PlacementAssistance::LlmOnly,
            placement_mode: PlacementMode::ExistingOnly,
            placement_reference_top_k: 5,
            placement_candidate_top_k: 3,
            placement_min_similarity: 0.20,
            placement_min_margin: 0.05,
            placement_min_reference_support: 2,
            rebuild: false,
            dry_run: true,
            llm_provider: LlmProvider::Gemini,
            llm_model: "gemini-3-flash-preview".to_string(),
            llm_base_url: None,
            api_key: Some(ApiKeySource::Text("secret".to_string())),
            embedding_provider: LlmProvider::Gemini,
            embedding_model: "gemini-embedding-2-preview".to_string(),
            embedding_base_url: None,
            embedding_api_key: None,
            keyword_batch_size: 50,
            batch_start_delay_ms: 100,
            subcategories_suggestion_number: 5,
            verbose: false,
            debug: false,
            quiet: false,
        }
    }

    #[test]
    fn persists_and_recovers_latest_run() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let cfg = sample_config();

        let workspace = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create workspace");
        let reopened = RunWorkspace::open_latest_with_cache_root(dir.path(), &cache_root)
            .expect("open latest");

        assert_eq!(workspace.run_id(), reopened.run_id());
        assert_eq!(
            reopened.load_config().expect("load config").llm_model,
            "gemini-3-flash-preview"
        );
    }

    #[test]
    fn stage_round_trip_works() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let cfg = sample_config();
        let mut workspace = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create workspace");
        let state = FilterSizeState {
            accepted: vec![PdfCandidate {
                path: "/tmp/in/a.pdf".into(),
                size_bytes: 123,
            }],
            skipped: vec![PdfCandidate {
                path: "/tmp/in/b.pdf".into(),
                size_bytes: 456,
            }],
        };

        workspace
            .save_stage(RunStage::FilterSize, &state)
            .expect("save stage");

        let loaded = workspace
            .load_stage::<FilterSizeState>(RunStage::FilterSize)
            .expect("load stage")
            .expect("stage should exist");
        assert_eq!(loaded.accepted.len(), 1);
        assert_eq!(loaded.skipped.len(), 1);
        assert_eq!(workspace.last_completed_stage(), Some(RunStage::FilterSize));
    }

    #[test]
    fn missing_stage_returns_none() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let cfg = sample_config();
        let workspace = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create workspace");

        let loaded = workspace
            .load_stage::<Vec<StageFailure>>(RunStage::ExtractText)
            .expect("load stage");

        assert!(loaded.is_none());
    }

    #[test]
    fn report_round_trip_works() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let cfg = sample_config();
        let workspace = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create workspace");
        let mut report = RunReport::new(true);
        report.scanned = 3;
        report.llm_usage.keywords.call_count = 2;

        workspace.save_report(&report).expect("save report");

        let loaded = workspace
            .load_report()
            .expect("load report")
            .expect("report should exist");
        assert_eq!(loaded.scanned, 3);
        assert_eq!(loaded.llm_usage.keywords.call_count, 2);
    }

    #[test]
    fn artifact_round_trip_and_removal_work() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let cfg = sample_config();
        let workspace = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create workspace");
        let mut progress = KeywordBatchProgress::default();
        progress.usage.call_count = 2;

        workspace
            .save_artifact("taxonomy-progress.json", &progress)
            .expect("save artifact");

        let loaded = workspace
            .load_artifact::<KeywordBatchProgress>("taxonomy-progress.json")
            .expect("load artifact")
            .expect("artifact should exist");
        assert_eq!(loaded.usage.call_count, 2);

        workspace
            .remove_artifact("taxonomy-progress.json")
            .expect("remove artifact");
        assert!(
            workspace
                .load_artifact::<KeywordBatchProgress>("taxonomy-progress.json")
                .expect("reload artifact")
                .is_none()
        );
    }

    #[test]
    fn lists_runs_newest_first_and_marks_latest() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let cfg = sample_config();

        let mut first = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create first");
        first
            .mark_stage(RunStage::ExtractText)
            .expect("mark first stage");
        let second = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create second");

        let runs =
            RunWorkspace::list_runs_with_cache_root(dir.path(), &cache_root).expect("list runs");

        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].run_id, second.run_id());
        assert!(runs[0].is_latest);
        assert_eq!(runs[1].run_id, first.run_id());
        assert_eq!(runs[1].last_completed_stage, Some(RunStage::ExtractText));
        assert!(!runs[1].is_latest);
    }

    #[test]
    fn removing_latest_run_repoints_latest_pointer() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let cfg = sample_config();

        let first = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create first");
        let second = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create second");

        let removed = RunWorkspace::remove_runs_with_cache_root(
            dir.path(),
            &cache_root,
            &[second.run_id().to_string()],
        )
        .expect("remove latest");

        assert_eq!(removed, vec![second.run_id().to_string()]);
        let runs =
            RunWorkspace::list_runs_with_cache_root(dir.path(), &cache_root).expect("list runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, first.run_id());
        assert!(runs[0].is_latest);
    }

    #[test]
    fn removing_last_run_clears_latest_pointer() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let cfg = sample_config();

        let run = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create run");

        RunWorkspace::remove_runs_with_cache_root(
            dir.path(),
            &cache_root,
            &[run.run_id().to_string()],
        )
        .expect("remove only run");

        let latest_path = cache_root.join("latest_run");
        assert!(!latest_path.exists());
    }

    #[test]
    fn clear_incomplete_runs_preserves_completed_runs() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let cfg = sample_config();

        let mut incomplete = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create incomplete");
        incomplete
            .mark_stage(RunStage::ExtractText)
            .expect("mark incomplete stage");

        let mut completed = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create completed");
        completed.mark_completed().expect("mark completed");

        let removed = RunWorkspace::clear_incomplete_runs_with_cache_root(dir.path(), &cache_root)
            .expect("clear incomplete");

        assert_eq!(removed, vec![incomplete.run_id().to_string()]);
        let runs =
            RunWorkspace::list_runs_with_cache_root(dir.path(), &cache_root).expect("list runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, completed.run_id());
        assert_eq!(runs[0].last_completed_stage, Some(RunStage::Completed));
        assert!(runs[0].is_latest);
    }

    #[test]
    fn clear_all_runs_removes_completed_and_incomplete_runs() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let cfg = sample_config();

        let mut incomplete = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create incomplete");
        incomplete
            .mark_stage(RunStage::ExtractText)
            .expect("mark incomplete stage");

        let mut completed = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create completed");
        completed.mark_completed().expect("mark completed");

        let removed = RunWorkspace::clear_all_runs_with_cache_root(dir.path(), &cache_root)
            .expect("clear all");

        assert_eq!(removed.len(), 2);
        assert!(removed.contains(&incomplete.run_id().to_string()));
        assert!(removed.contains(&completed.run_id().to_string()));
        let runs =
            RunWorkspace::list_runs_with_cache_root(dir.path(), &cache_root).expect("list runs");
        assert!(runs.is_empty());
        assert!(!cache_root.join("latest_run").exists());
    }

    #[test]
    fn inspect_run_loads_saved_report_taxonomy_and_stage_artifacts() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let cfg = sample_config();
        let mut workspace = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create workspace");

        workspace
            .save_stage(
                RunStage::ExtractText,
                &ExtractTextState {
                    papers: Vec::new(),
                    failures: Vec::new(),
                },
            )
            .expect("save extract state");
        workspace
            .save_stage(
                RunStage::SynthesizeCategories,
                &SynthesizeCategoriesState {
                    categories: vec![CategoryTree {
                        name: "AI".to_string(),
                        children: vec![],
                    }],
                    partial_categories: Vec::new(),
                    reference_evidence: None,
                },
            )
            .expect("save taxonomy");
        let mut report = RunReport::new(true);
        report.scanned = 4;
        workspace.save_report(&report).expect("save report");

        let details = workspace.inspect().expect("inspect run");

        assert_eq!(
            details.config,
            SessionConfigSummary {
                dry_run: true,
                llm_provider: "gemini".to_string(),
                llm_model: "gemini-3-flash-preview".to_string(),
            }
        );
        assert_eq!(details.report.expect("report").scanned, 4);
        assert_eq!(details.taxonomy.expect("taxonomy")[0].name, "AI");
        assert_eq!(
            details.available_stage_artifacts,
            vec![RunStage::ExtractText, RunStage::SynthesizeCategories]
        );
    }

    #[test]
    fn inspect_run_marks_incomplete_runs_as_failed_looking() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let cfg = sample_config();
        let mut workspace = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create workspace");
        workspace
            .mark_stage(RunStage::ExtractText)
            .expect("mark stage");

        let details = workspace.inspect().expect("inspect run");

        assert_eq!(
            details.status,
            SessionStatusSummary {
                is_completed: false,
                is_incomplete: true,
                is_failed_looking: true,
            }
        );
    }

    #[test]
    fn inspect_run_marks_completed_runs_with_failed_report_as_failed_looking() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let cfg = sample_config();
        let mut workspace = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create workspace");
        workspace.mark_completed().expect("mark completed");
        let mut report = RunReport::new(true);
        report.failed = 2;
        workspace.save_report(&report).expect("save report");

        let details = workspace.inspect().expect("inspect run");

        assert_eq!(
            details.status,
            SessionStatusSummary {
                is_completed: true,
                is_incomplete: false,
                is_failed_looking: true,
            }
        );
    }
}
