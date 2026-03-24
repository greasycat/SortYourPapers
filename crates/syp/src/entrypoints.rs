use syp_core::{
    app, config,
    error::{AppError, Result},
    session,
};

use crate::{Cli, Commands, ReferenceCommands, SessionCommands};

/// Dispatches the existing clap-based CLI surface.
///
/// # Errors
/// Returns an error when any selected command fails.
pub async fn run_cli(cli: Cli) -> Result<()> {
    if let Some(command) = cli.command {
        match command {
            Commands::Init(args) => {
                let path = config::init_xdg_config(args.force)?;
                println!("Wrote default config to {}", path.display());
            }
            Commands::ExtractText(args) => app::run_extract_text(args.into_request()).await?,
            Commands::Reference(args) => match args.command {
                ReferenceCommands::Index(args) => {
                    let config = config::resolve_config(Default::default())?;
                    app::index_reference_manifest(config, args.manifest, args.force).await?;
                }
            },
            Commands::Session(args) => match args.command {
                SessionCommands::Resume(args) => {
                    session::resume_run(args.run_id, args.apply, args.verbosity, args.quiet)
                        .await
                        .map(|_| ())?;
                }
                SessionCommands::Rerun(args) => {
                    session::rerun_run(
                        args.run_id,
                        args.stage,
                        args.apply,
                        args.verbosity,
                        args.quiet,
                    )
                    .await
                    .map(|_| ())?;
                }
                SessionCommands::Review(args) => session::review_session(args.run_id)?,
                SessionCommands::List => session::list_sessions()?,
                SessionCommands::Remove(args) => session::remove_sessions(args.run_ids)?,
                SessionCommands::Clear => session::clear_sessions()?,
            },
        }
        return Ok(());
    }

    app::run_with_args(cli.run.into_run_overrides())
        .await
        .map(|_| ())
}

pub fn print_error_with_hints(err: &AppError) {
    eprintln!("error: {err}");

    if let AppError::MissingConfig(missing_key) = err
        && !missing_key.to_ascii_lowercase().contains("api_key")
    {
        if let Some(path) = config::xdg_config_path() {
            eprintln!(
                "hint: run `syp init` to create a default config at {}",
                path.display()
            );
        } else {
            eprintln!("hint: run `syp init` to create a default XDG config");
        }
    }
}
