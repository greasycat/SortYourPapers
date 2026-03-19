pub(crate) mod stages;

use std::{
    env,
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::{
    cli::{CliArgs, ExtractTextArgs},
    config,
    error::{AppError, Result},
    logging::Verbosity,
    models::{
        AppConfig, CategoryTree, KeywordSet, KeywordStageState, PaperText, PdfCandidate,
        PlacementDecision, PreliminaryCategoryPair, RunReport, SynthesizeCategoriesState,
    },
    pdf_extract::{extract_text_batch, reset_debug_extract_log},
    run_state::{ExtractTextState, FilterSizeState, RunStage, RunWorkspace},
};

pub(crate) use stages::run_with_workspace;

/// Resolves CLI arguments into an application config and runs the main workflow.
///
/// # Errors
/// Returns an error when config resolution fails or the run itself fails.
pub async fn run_with_args(cli: CliArgs) -> Result<RunReport> {
    let config = config::resolve_config(cli)?;
    run(config).await
}

/// Runs the main PDF organization workflow using a fully resolved config.
///
/// # Errors
/// Returns an error when workspace setup fails or any pipeline stage fails.
pub async fn run(config: AppConfig) -> Result<RunReport> {
    let config = absolutize_config(config)?;
    let mut workspace = RunWorkspace::create(&config)?;
    let verbosity = Verbosity::new(config.verbose, config.debug, config.quiet);
    verbosity.run_line(
        "RUN",
        format!(
            "run_id={} state_dir={}",
            verbosity.accent(workspace.run_id()),
            verbosity.muted(workspace.root_dir().display().to_string())
        ),
    );
    run_with_workspace(config, &mut workspace).await
}

pub(crate) async fn run_debug_tui(config: AppConfig) -> Result<RunReport> {
    let mut config = absolutize_config(config)?;
    config.rebuild = false;
    config.dry_run = true;

    let mut workspace = RunWorkspace::create(&config)?;
    seed_debug_stages(&mut workspace, &config)?;

    run_with_workspace(config, &mut workspace).await
}

fn seed_debug_stages(workspace: &mut RunWorkspace, config: &AppConfig) -> Result<()> {
    let candidates = vec![
        PdfCandidate {
            path: config.input.join("debug-paper-01.pdf"),
            size_bytes: 1_048_576,
        },
        PdfCandidate {
            path: config.input.join("debug-paper-02.pdf"),
            size_bytes: 512_000,
        },
        PdfCandidate {
            path: config.input.join("debug-paper-03.pdf"),
            size_bytes: 640_000,
        },
    ];

    let skipped = vec![PdfCandidate {
        path: config.input.join("debug-paper-skipped.pdf"),
        size_bytes: 99_999_999,
    }];

    let papers = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let file_id = format!("debug-paper-{index:02}");
            PaperText {
                file_id,
                path: candidate.path.clone(),
                extracted_text: "Debug extracted text from mocked PDF content".to_string(),
                llm_ready_text: "Mocked LLM-ready text for debug workflow".to_string(),
                pages_read: 5,
            }
        })
        .collect::<Vec<_>>();

    let keyword_sets = papers
        .iter()
        .map(|paper| KeywordSet {
            file_id: paper.file_id.clone(),
            keywords: vec![
                "debug".to_string(),
                "workflow".to_string(),
                "ratatui".to_string(),
            ],
        })
        .collect::<Vec<_>>();

    let preliminary_pairs = papers
        .iter()
        .map(|paper| PreliminaryCategoryPair {
            file_id: paper.file_id.clone(),
            preliminary_categories_k_depth: "debug,workflow".to_string(),
        })
        .collect::<Vec<_>>();

    let categories = vec![
        CategoryTree {
            name: "debug".to_string(),
            children: vec![CategoryTree {
                name: "workflow".to_string(),
                children: vec![],
            }],
        },
        CategoryTree {
            name: "notes".to_string(),
            children: vec![],
        },
    ];

    let placements = papers
        .iter()
        .enumerate()
        .map(|(index, paper)| PlacementDecision {
            file_id: paper.file_id.clone(),
            target_rel_path: if index == 0 {
                "debug/workflow".to_string()
            } else {
                "notes".to_string()
            },
        })
        .collect::<Vec<_>>();

    workspace.save_stage(RunStage::DiscoverInput, &candidates)?;
    workspace.save_stage(RunStage::Dedupe, &candidates)?;
    workspace.save_stage(
        RunStage::FilterSize,
        &FilterSizeState {
            accepted: candidates,
            skipped,
        },
    )?;
    workspace.save_stage(
        RunStage::ExtractText,
        &ExtractTextState {
            papers,
            failures: Vec::new(),
        },
    )?;
    workspace.save_stage(
        RunStage::ExtractKeywords,
        &KeywordStageState {
            keyword_sets,
            preliminary_pairs,
        },
    )?;
    workspace.save_stage(
        RunStage::SynthesizeCategories,
        &SynthesizeCategoriesState {
            categories: categories.clone(),
            partial_categories: vec![categories.clone()],
        },
    )?;
    workspace.save_stage(
        RunStage::InspectOutput,
        &InspectableDebugState {
            categories: categories.clone(),
        },
    )?;
    workspace.save_stage(RunStage::GeneratePlacements, &placements)?;

    Ok(())
}

#[derive(Debug, Serialize)]
struct InspectableDebugState {
    categories: Vec<CategoryTree>,
}

/// Extracts and prints text for the provided PDFs without running the full workflow.
///
/// # Errors
/// Returns an error when arguments are invalid, extraction fails, or any file
/// cannot be processed.
pub async fn run_extract_text(args: ExtractTextArgs) -> Result<()> {
    if args.page_cutoff == 0 {
        return Err(AppError::Validation(
            "page_cutoff must be greater than 0".to_string(),
        ));
    }
    if args.pdf_extract_workers == 0 {
        return Err(AppError::Validation(
            "pdf_extract_workers must be greater than 0".to_string(),
        ));
    }

    let verbose = args.verbosity > 0;
    let debug = args.verbosity > 1;
    let verbosity = Verbosity::new(verbose, debug, false);
    reset_debug_extract_log(debug)?;

    let candidates = args
        .files
        .iter()
        .map(|path| PdfCandidate {
            path: path.clone(),
            size_bytes: 0,
        })
        .collect::<Vec<_>>();
    let (papers, failures) = extract_text_batch(
        &candidates,
        args.page_cutoff,
        args.extractor,
        debug,
        args.pdf_extract_workers,
        verbosity,
    )
    .await;

    let failure_count = failures.len();
    for (index, paper) in papers.iter().enumerate() {
        if index > 0 {
            println!();
        }
        println!("=== {} ===", paper.path.display());
        println!("file_id: {}", paper.file_id);
        println!("pages_read: {}", paper.pages_read);
        println!();
        println!("--- raw ---");
        println!("{}", paper.extracted_text);
        if verbose {
            println!();
            println!("--- llm-ready ---");
            println!("{}", paper.llm_ready_text);
        }
    }

    for (path, err) in failures {
        if !papers.is_empty() {
            println!();
        }
        eprintln!("[extract-failed] {}: {}", path.display(), err);
    }

    if failure_count > 0 {
        return Err(AppError::Execution(format!(
            "manual extraction failed for {failure_count} file(s)"
        )));
    }

    Ok(())
}

fn absolutize_config(mut config: AppConfig) -> Result<AppConfig> {
    let cwd = env::current_dir()?;
    config.input = absolutize_path(&cwd, &config.input);
    config.output = absolutize_path(&cwd, &config.output);
    Ok(config)
}

fn absolutize_path(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}
