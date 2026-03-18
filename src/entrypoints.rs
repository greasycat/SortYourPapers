use crate::{
    Cli, Commands, SessionCommands,
    error::{AppError, Result},
};

/// Dispatches the existing clap-based CLI surface.
///
/// # Errors
/// Returns an error when any selected command fails.
pub async fn run_cli(cli: Cli) -> Result<()> {
    if let Some(command) = cli.command {
        match command {
            Commands::Init(args) => {
                let path = crate::init_config(args.force)?;
                println!("Wrote default config to {}", path.display());
            }
            Commands::ExtractText(args) => crate::run_extract_text(args).await?,
            Commands::Session(args) => match args.command {
                SessionCommands::Resume(args) => {
                    crate::resume_run(args.run_id, args.apply, args.verbosity, args.quiet)
                        .await
                        .map(|_| ())?;
                }
                SessionCommands::Rerun(args) => {
                    crate::rerun_run(
                        args.run_id,
                        args.stage,
                        args.apply,
                        args.verbosity,
                        args.quiet,
                    )
                    .await
                    .map(|_| ())?;
                }
                SessionCommands::Review(args) => crate::review_session(args.run_id)?,
                SessionCommands::List => crate::list_sessions()?,
                SessionCommands::Remove(args) => crate::remove_sessions(args.run_ids)?,
                SessionCommands::Clear => crate::clear_sessions()?,
            },
        }
        return Ok(());
    }

    crate::run_with_args(cli.run).await.map(|_| ())
}

pub fn print_error_with_hints(err: &AppError) {
    eprintln!("error: {err}");

    if let AppError::MissingConfig(missing_key) = err
        && !missing_key.to_ascii_lowercase().contains("api_key")
    {
        if let Some(path) = crate::config::xdg_config_path() {
            eprintln!(
                "hint: run `sortyourpapers init` to create a default config at {}",
                path.display()
            );
        } else {
            eprintln!("hint: run `sortyourpapers init` to create a default XDG config");
        }
    }
}
