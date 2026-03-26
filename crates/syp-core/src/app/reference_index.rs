use std::path::PathBuf;

use crate::{
    config::AppConfig, error::Result, papers::taxonomy::index_reference_manifest as build_report,
    terminal::Verbosity,
};

use super::path_resolution::absolutize_config;

pub async fn index_reference_manifest(
    config: AppConfig,
    manifest_path: Option<PathBuf>,
    force: bool,
) -> Result<()> {
    let config = absolutize_config(config)?;
    let verbosity = Verbosity::new(config.verbose, config.debug, config.quiet);
    let report = build_report(&config, manifest_path, force, verbosity).await?;
    println!(
        "{} reference index for {} using {}:{} at {} ({} paper(s))",
        if report.skipped { "Reused" } else { "Updated" },
        report.set_id,
        report.provider,
        report.model,
        report.db_path.display(),
        report.papers_indexed
    );
    Ok(())
}
