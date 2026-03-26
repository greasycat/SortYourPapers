use crate::{
    config,
    config::AppConfig,
    error::Result,
    inputs::RunOverrides,
    report::RunReport,
    session::{RunWorkspace, run_with_workspace},
    terminal::Verbosity,
};

use super::path_resolution::absolutize_config;

/// Resolves CLI arguments into an application config and runs the main workflow.
///
/// # Errors
/// Returns an error when config resolution fails or the run itself fails.
pub async fn run_with_args(overrides: RunOverrides) -> Result<RunReport> {
    let config = config::resolve_config(overrides)?;
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
