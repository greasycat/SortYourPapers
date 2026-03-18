use std::{
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use clap::Parser;
use sortyourpapers::{
    config::{self, CliArgs},
    error::AppError,
    models::{LlmProvider, PlacementMode, TaxonomyMode},
};

#[derive(Debug, Parser)]
#[command(
    name = "compare-taxonomy-modes",
    about = "Time batch-merge and global taxonomy synthesis modes on the same paper set"
)]
struct CompareArgs {
    #[arg(short = 'i', long)]
    input: Option<PathBuf>,

    #[arg(long)]
    output_root: Option<PathBuf>,

    #[arg(short = 'r', long, num_args = 0..=1, default_missing_value = "true")]
    recursive: Option<bool>,

    #[arg(short = 's', long)]
    max_file_size_mb: Option<u64>,

    #[arg(short = 'p', long)]
    page_cutoff: Option<u8>,

    #[arg(long, default_value_t = 4)]
    pdf_extract_workers: usize,

    #[arg(short = 'd', long)]
    category_depth: Option<u8>,

    #[arg(short = 'M', long)]
    placement_mode: Option<PlacementMode>,

    #[arg(short = 'R', long, num_args = 0..=1, default_missing_value = "true")]
    rebuild: Option<bool>,

    #[arg(short = 'P', long)]
    llm_provider: Option<LlmProvider>,

    #[arg(short = 'm', long)]
    llm_model: Option<String>,

    #[arg(short = 'u', long)]
    llm_base_url: Option<String>,

    #[arg(short = 'k', long)]
    api_key: Option<String>,

    #[arg(long)]
    keyword_batch_size: Option<usize>,

    #[arg(long)]
    subcategories_suggestion_number: Option<usize>,

    #[arg(long, default_value_t = 3)]
    taxonomy_batch_size: usize,

    #[arg(long, default_value_t = 25)]
    placement_batch_size: usize,

    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count)]
    verbosity: u8,
}

struct ModeTiming {
    elapsed_secs: f64,
}

#[tokio::main]
async fn main() {
    let args = CompareArgs::parse();
    if let Err(err) = run_compare(args).await {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

async fn run_compare(args: CompareArgs) -> Result<(), AppError> {
    let output_root = args
        .output_root
        .unwrap_or_else(default_output_root)
        .join(unique_run_label());

    let base_cli = CliArgs {
        input: args.input,
        output: None,
        recursive: args.recursive,
        max_file_size_mb: args.max_file_size_mb,
        page_cutoff: args.page_cutoff,
        pdf_extract_workers: Some(args.pdf_extract_workers),
        category_depth: args.category_depth,
        taxonomy_mode: None,
        taxonomy_batch_size: Some(args.taxonomy_batch_size),
        placement_batch_size: Some(args.placement_batch_size),
        placement_mode: args.placement_mode,
        rebuild: args.rebuild,
        apply: false,
        llm_provider: args.llm_provider,
        llm_model: args.llm_model,
        llm_base_url: args.llm_base_url,
        api_key: args.api_key,
        keyword_batch_size: args.keyword_batch_size,
        subcategories_suggestion_number: args.subcategories_suggestion_number,
        verbosity: args.verbosity,
        quiet: args.verbosity == 0,
    };

    println!("Comparing taxonomy modes");
    println!("output_root: {}", output_root.display());
    println!("mode: preview");
    println!("pdf_extract_workers: {}", args.pdf_extract_workers);
    println!("taxonomy_batch_size: {}", args.taxonomy_batch_size);
    println!("placement_batch_size: {}", args.placement_batch_size);
    println!();

    let batch_timing = run_mode(
        base_cli.clone(),
        &output_root,
        TaxonomyMode::BatchMerge,
        "batch-merge",
    )
    .await?;
    println!();
    let global_timing = run_mode(base_cli, &output_root, TaxonomyMode::Global, "global").await?;

    println!();
    println!("Comparison");
    println!("- batch-merge: {:.3}s", batch_timing.elapsed_secs);
    println!("- global: {:.3}s", global_timing.elapsed_secs);

    let delta = batch_timing.elapsed_secs - global_timing.elapsed_secs;
    if delta.abs() < f64::EPSILON {
        println!("- delta: effectively identical");
    } else if delta < 0.0 {
        println!("- delta: batch-merge faster by {:.3}s", delta.abs());
    } else {
        println!("- delta: global faster by {:.3}s", delta);
    }

    if global_timing.elapsed_secs > 0.0 {
        println!(
            "- ratio: batch-merge/global = {:.3}",
            batch_timing.elapsed_secs / global_timing.elapsed_secs
        );
    }

    Ok(())
}

async fn run_mode(
    base_cli: CliArgs,
    output_root: &Path,
    mode: TaxonomyMode,
    label: &str,
) -> Result<ModeTiming, AppError> {
    let cli = CliArgs {
        output: Some(output_root.join(label)),
        taxonomy_mode: Some(mode),
        ..base_cli
    };
    let config = config::resolve_config(cli)?;

    println!("Running `{label}`");
    println!("- input: {}", config.input.display());
    println!("- output: {}", config.output.display());
    println!("- taxonomy_mode: {:?}", config.taxonomy_mode);

    let started = Instant::now();
    sortyourpapers::run(config).await?;
    let elapsed_secs = started.elapsed().as_secs_f64();

    println!("- elapsed: {:.3}s", elapsed_secs);

    Ok(ModeTiming { elapsed_secs })
}

fn default_output_root() -> PathBuf {
    std::env::temp_dir().join("sortyourpapers-compare")
}

fn unique_run_label() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("run-{}-{}", std::process::id(), millis)
}
