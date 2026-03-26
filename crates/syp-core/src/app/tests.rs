use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use tempfile::tempdir;

use super::{apply_debug_taxonomy_suggestion, seed_debug_stages, simulate_debug_taxonomy_review};
use crate::{
    config::AppConfig,
    llm::LlmProvider,
    papers::placement::{PlacementAssistance, PlacementMode},
    papers::taxonomy::{CategoryTree, TaxonomyAssistance, TaxonomyMode},
    session::{RunStage, RunWorkspace},
    terminal::{
        AlertSeverity, InspectReviewPrompt, InspectReviewRequest, TerminalBackend, Verbosity,
        install_backend,
    },
};

#[test]
fn seed_debug_stages_populates_preview_report_and_build_plan() {
    let dir = tempdir().expect("tempdir");
    let cache_root = dir.path().join("cache");
    let config = AppConfig {
        input: dir.path().join("input"),
        output: dir.path().join("output"),
        recursive: true,
        max_file_size_mb: 64,
        page_cutoff: 10,
        pdf_extract_workers: 2,
        category_depth: 2,
        taxonomy_mode: TaxonomyMode::BatchMerge,
        taxonomy_assistance: TaxonomyAssistance::LlmOnly,
        taxonomy_batch_size: 2,
        reference_manifest_path: dir.path().join("references.toml"),
        reference_top_k: 5,
        use_current_folder_tree: false,
        placement_batch_size: 2,
        placement_assistance: PlacementAssistance::LlmOnly,
        placement_mode: PlacementMode::AllowNew,
        placement_reference_top_k: 5,
        placement_candidate_top_k: 3,
        placement_min_similarity: 0.20,
        placement_min_margin: 0.05,
        placement_min_reference_support: 2,
        rebuild: false,
        dry_run: true,
        llm_provider: LlmProvider::Gemini,
        llm_model: "debug-model".to_string(),
        llm_base_url: None,
        api_key: None,
        embedding_provider: LlmProvider::Gemini,
        embedding_model: "gemini-embedding-2-preview".to_string(),
        embedding_base_url: None,
        embedding_api_key: None,
        keyword_batch_size: 2,
        batch_start_delay_ms: 0,
        subcategories_suggestion_number: 4,
        verbose: false,
        debug: false,
        quiet: false,
    };
    let mut workspace =
        RunWorkspace::create_with_cache_root_for_tests(dir.path(), &cache_root, &config)
            .expect("create workspace");

    let debug_run = seed_debug_stages(&mut workspace, &config).expect("seed debug stages");
    let saved_actions = workspace
        .load_stage::<Vec<crate::report::PlanAction>>(RunStage::BuildPlan)
        .expect("load build plan")
        .expect("saved build plan");
    let saved_review = workspace
        .load_stage::<serde_json::Value>(RunStage::InspectOutput)
        .expect("load inspect output");

    assert!(debug_run.report.dry_run);
    assert_eq!(debug_run.report.scanned, 4);
    assert_eq!(debug_run.report.processed, 3);
    assert_eq!(debug_run.report.skipped, 1);
    assert_eq!(debug_run.report.actions.len(), 3);
    assert_eq!(saved_actions.len(), 3);
    assert!(saved_review.is_none());
}

#[test]
fn debug_taxonomy_suggestion_updates_first_root_label() {
    let categories = vec![
        CategoryTree {
            name: "debug".to_string(),
            children: vec![],
        },
        CategoryTree {
            name: "notes".to_string(),
            children: vec![],
        },
    ];

    let updated = apply_debug_taxonomy_suggestion(
        &categories,
        &InspectReviewRequest::from_user_suggestion("merge workflow".to_string()),
    );

    assert_eq!(updated[0].name, "debug (merge workflow)");
    assert_eq!(updated[1].name, "notes");
}

#[test]
fn debug_taxonomy_review_uses_prompt_loop_and_returns_reviewed_categories() {
    let backend = Arc::new(DebugReviewBackend::new(
        vec![
            InspectReviewPrompt::Suggest(InspectReviewRequest::from_user_suggestion(
                "merge workflow".to_string(),
            )),
            InspectReviewPrompt::Accept,
        ],
        vec![true],
    ));
    let _guard = install_backend(backend.clone());
    let categories = vec![CategoryTree {
        name: "debug".to_string(),
        children: vec![],
    }];

    let reviewed = simulate_debug_taxonomy_review(&categories, Verbosity::new(false, false, false))
        .expect("debug review should complete");

    assert_eq!(reviewed[0].name, "debug (merge workflow)");
    assert_eq!(*backend.inspect_calls.lock().expect("inspect calls"), 2);
    assert_eq!(*backend.continue_calls.lock().expect("continue calls"), 1);
    assert_eq!(backend.tree_renders.lock().expect("tree renders").len(), 2);
}

struct DebugReviewBackend {
    inspect_replies: Mutex<VecDeque<InspectReviewPrompt>>,
    continue_replies: Mutex<VecDeque<bool>>,
    tree_renders: Mutex<Vec<Vec<CategoryTree>>>,
    inspect_calls: Mutex<usize>,
    continue_calls: Mutex<usize>,
}

impl DebugReviewBackend {
    fn new(inspect_replies: Vec<InspectReviewPrompt>, continue_replies: Vec<bool>) -> Self {
        Self {
            inspect_replies: Mutex::new(inspect_replies.into()),
            continue_replies: Mutex::new(continue_replies.into()),
            tree_renders: Mutex::new(Vec::new()),
            inspect_calls: Mutex::new(0),
            continue_calls: Mutex::new(0),
        }
    }
}

impl TerminalBackend for DebugReviewBackend {
    fn stdout_is_terminal(&self) -> bool {
        false
    }

    fn stderr_is_terminal(&self) -> bool {
        false
    }

    fn supports_progress(&self) -> bool {
        false
    }

    fn is_interactive(&self) -> bool {
        true
    }

    fn write_stdout_line(&self, _line: &str) {}

    fn write_stderr_line(&self, _line: &str) {}

    fn start_progress(&self, _id: u64, _total: usize, _label: &str) {}

    fn advance_progress(&self, _id: u64, _delta: usize) {}

    fn finish_progress(&self, _id: u64) {}

    fn show_report(&self, _report: &crate::report::RunReport, _verbosity: Verbosity) {}

    fn show_category_tree(&self, categories: &[CategoryTree], _verbosity: Verbosity) {
        self.tree_renders
            .lock()
            .expect("tree renders")
            .push(categories.to_vec());
    }

    fn update_stage_status(&self, _stage: &str, _message: &str) {}

    fn record_alert(&self, _severity: AlertSeverity, _label: &str, _message: &str) {}

    fn prompt_inspect_review_action(
        &self,
        _categories: &[CategoryTree],
        _verbosity: Verbosity,
    ) -> crate::error::Result<InspectReviewPrompt> {
        *self.inspect_calls.lock().expect("inspect calls") += 1;
        self.inspect_replies
            .lock()
            .expect("inspect replies")
            .pop_front()
            .ok_or_else(|| {
                crate::error::AppError::Execution("missing debug inspect reply".to_string())
            })
    }

    fn prompt_continue_improving(&self) -> crate::error::Result<bool> {
        *self.continue_calls.lock().expect("continue calls") += 1;
        self.continue_replies
            .lock()
            .expect("continue replies")
            .pop_front()
            .ok_or_else(|| {
                crate::error::AppError::Execution("missing debug continue reply".to_string())
            })
    }
}
