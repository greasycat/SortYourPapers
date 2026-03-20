use std::path::PathBuf;

use crate::{
    config::AppConfig,
    llm::LlmProvider,
    papers::placement::PlacementMode,
    papers::taxonomy::{CategoryTree, TaxonomyMode},
    session::commands::{
        RerunArtifact, apply_resume_overrides, completed_runs, describe_rerun_impact,
        resolve_run_selection, resolve_stage_selection, selectable_runs, validate_run_ids,
    },
    session::{RunStage, RunSummary, format_stage_description, stage_sequence},
    terminal::{Verbosity, report::render_category_tree},
};

fn sample_runs() -> Vec<RunSummary> {
    vec![
        RunSummary {
            run_id: "run-2".to_string(),
            created_unix_ms: 2,
            cwd: PathBuf::from("/tmp/two"),
            last_completed_stage: Some(RunStage::ExtractText),
            is_latest: true,
        },
        RunSummary {
            run_id: "run-3".to_string(),
            created_unix_ms: 3,
            cwd: PathBuf::from("/tmp/three"),
            last_completed_stage: Some(RunStage::Completed),
            is_latest: false,
        },
        RunSummary {
            run_id: "run-1".to_string(),
            created_unix_ms: 1,
            cwd: PathBuf::from("/tmp/one"),
            last_completed_stage: None,
            is_latest: false,
        },
    ]
}

fn strip_ansi_sgr(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut stripped = String::with_capacity(text.len());
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] == b'\x1b' && bytes.get(index + 1) == Some(&b'[') {
            index += 2;
            while index < bytes.len() && bytes[index] != b'm' {
                index += 1;
            }
            if index < bytes.len() {
                index += 1;
            }
            continue;
        }

        stripped.push(bytes[index] as char);
        index += 1;
    }

    stripped
}

#[test]
fn resolves_run_selection_by_index() {
    let runs = selectable_runs(sample_runs(), false);

    let selected = resolve_run_selection("2", &runs).expect("resolve by index");

    assert_eq!(selected, "run-1");
}

#[test]
fn resolves_run_selection_by_run_id() {
    let runs = selectable_runs(sample_runs(), false);

    let selected = resolve_run_selection("run-2", &runs).expect("resolve by id");

    assert_eq!(selected, "run-2");
}

#[test]
fn rejects_invalid_run_selection() {
    let runs = selectable_runs(sample_runs(), false);

    let err = resolve_run_selection("9", &runs).expect_err("selection should fail");

    assert!(err.to_string().contains("out of range"));
}

#[test]
fn rejects_zero_run_selection() {
    let runs = selectable_runs(sample_runs(), false);

    let err = resolve_run_selection("0", &runs).expect_err("zero selection should fail");

    assert!(err.to_string().contains("out of range"));
}

#[test]
fn resolves_rerun_stage_selection_by_index() {
    let stages = vec![
        RunStage::DiscoverInput,
        RunStage::ExtractText,
        RunStage::ExtractKeywords,
    ];

    let selected = resolve_stage_selection("2", &stages).expect("resolve stage by index");

    assert_eq!(selected, RunStage::ExtractText);
}

#[test]
fn resolves_rerun_stage_selection_by_name() {
    let stages = vec![
        RunStage::DiscoverInput,
        RunStage::ExtractText,
        RunStage::ExtractKeywords,
    ];

    let selected = resolve_stage_selection("extract-keywords", &stages)
        .expect("resolve stage by kebab-case name");

    assert_eq!(selected, RunStage::ExtractKeywords);
}

#[test]
fn rejects_zero_stage_selection() {
    let stages = vec![
        RunStage::DiscoverInput,
        RunStage::ExtractText,
        RunStage::ExtractKeywords,
    ];

    let err = resolve_stage_selection("0", &stages).expect_err("zero stage should fail");

    assert!(err.to_string().contains("out of range"));
}

#[test]
fn validate_run_ids_requires_exact_run_ids() {
    let runs = sample_runs();

    let err =
        validate_run_ids(&["9".to_string()], &runs).expect_err("numeric selection should fail");

    assert!(err.to_string().contains("run '9' was not found"));
}

#[test]
fn filters_completed_runs_from_preview_selection() {
    let runs = selectable_runs(sample_runs(), false);

    assert_eq!(runs.len(), 2);
    assert!(
        runs.iter()
            .all(|run| run.last_completed_stage != Some(RunStage::Completed))
    );
}

#[test]
fn keeps_completed_runs_in_apply_selection() {
    let runs = selectable_runs(sample_runs(), true);

    assert_eq!(runs.len(), 3);
    assert!(
        runs.iter()
            .any(|run| run.last_completed_stage == Some(RunStage::Completed))
    );
}

#[test]
fn completed_runs_filters_to_completed_only() {
    let runs = completed_runs(sample_runs());

    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].run_id, "run-3");
}

#[test]
fn resume_apply_override_turns_off_dry_run() {
    let mut config = sample_config();

    apply_resume_overrides(&mut config, true, 0, false);

    assert!(!config.dry_run);
}

#[test]
fn resume_overrides_apply_verbosity_and_quiet() {
    let mut config = sample_config();

    apply_resume_overrides(&mut config, false, 2, true);

    assert!(config.dry_run);
    assert!(config.verbose);
    assert!(config.debug);
    assert!(config.quiet);
}

#[test]
fn stage_sequence_omits_optional_stages_by_default() {
    let stages = stage_sequence(false, false);

    assert_eq!(
        stages,
        vec![
            RunStage::DiscoverInput,
            RunStage::Dedupe,
            RunStage::FilterSize,
            RunStage::ExtractText,
            RunStage::ExtractKeywords,
            RunStage::SynthesizeCategories,
            RunStage::InspectOutput,
            RunStage::GeneratePlacements,
            RunStage::BuildPlan,
            RunStage::ExecutePlan,
        ]
    );
}

#[test]
fn stage_sequence_includes_optional_stages_when_needed() {
    let stages = stage_sequence(true, true);

    assert_eq!(
        stages,
        vec![
            RunStage::DiscoverInput,
            RunStage::DiscoverOutput,
            RunStage::Dedupe,
            RunStage::FilterSize,
            RunStage::ExtractText,
            RunStage::BuildLlmClient,
            RunStage::ExtractKeywords,
            RunStage::SynthesizeCategories,
            RunStage::InspectOutput,
            RunStage::GeneratePlacements,
            RunStage::BuildPlan,
            RunStage::ExecutePlan,
        ]
    );
}

#[test]
fn stage_description_formatter_preserves_plain_text_without_color() {
    let verbosity = Verbosity::new(false, false, false);
    let formatted = format_stage_description(verbosity, "Extract keywords");

    assert_eq!(strip_ansi_sgr(&formatted), "Extract keywords");
}

#[test]
fn category_tree_renderer_uses_ascii_tree_layout() {
    let rendered = render_category_tree(&[CategoryTree {
        name: "AI".to_string(),
        children: vec![CategoryTree {
            name: "Vision".to_string(),
            children: vec![CategoryTree {
                name: "Segmentation".to_string(),
                children: vec![],
            }],
        }],
    }]);

    assert!(rendered.contains("AI"));
    assert!(rendered.contains("\\-- Vision"));
    assert!(rendered.contains("\\-- Segmentation"));
}

#[test]
fn rerun_impact_describes_reset_scope_for_early_stage_restart() {
    let impact =
        describe_rerun_impact(&sample_config(), RunStage::ExtractText).expect("describe impact");

    assert_eq!(
        impact.previous_last_completed_stage,
        Some(RunStage::FilterSize)
    );
    assert!(impact.cleared_stage_files.contains(&RunStage::ExtractText));
    assert!(impact.cleared_stage_files.contains(&RunStage::BuildPlan));
    assert!(
        impact
            .cleared_artifacts
            .contains(&RerunArtifact::KeywordBatchProgress)
    );
    assert!(
        impact
            .cleared_artifacts
            .contains(&RerunArtifact::TaxonomyBatchProgress)
    );
    assert!(
        impact
            .cleared_artifacts
            .contains(&RerunArtifact::PlacementBatchProgress)
    );
    assert!(
        impact
            .report_reset_sections
            .contains(&"scan and extraction counters")
    );
}

#[test]
fn rerun_impact_keeps_placement_progress_when_restarting_at_build_plan() {
    let impact =
        describe_rerun_impact(&sample_config(), RunStage::BuildPlan).expect("describe impact");

    assert_eq!(
        impact.previous_last_completed_stage,
        Some(RunStage::GeneratePlacements)
    );
    assert_eq!(impact.cleared_stage_files, vec![RunStage::BuildPlan]);
    assert!(impact.cleared_artifacts.is_empty());
    assert_eq!(impact.report_reset_sections, vec!["planned actions"]);
}

fn sample_config() -> AppConfig {
    AppConfig {
        input: PathBuf::from("/tmp/in"),
        output: PathBuf::from("/tmp/out"),
        recursive: false,
        max_file_size_mb: 16,
        page_cutoff: 1,
        pdf_extract_workers: 8,
        category_depth: 2,
        taxonomy_mode: TaxonomyMode::BatchMerge,
        taxonomy_batch_size: 4,
        use_current_folder_tree: false,
        placement_batch_size: 10,
        placement_mode: PlacementMode::ExistingOnly,
        rebuild: false,
        dry_run: true,
        llm_provider: LlmProvider::Gemini,
        llm_model: "gemini-3-flash-preview".to_string(),
        llm_base_url: None,
        api_key: None,
        keyword_batch_size: 20,
        batch_start_delay_ms: 100,
        subcategories_suggestion_number: 5,
        verbose: false,
        debug: false,
        quiet: false,
    }
}
