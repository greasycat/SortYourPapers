use clap::Parser;
use sortyourpapers::config::{Cli, Commands};
use sortyourpapers::error::AppError;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if let Some(command) = cli.command {
        match command {
            Commands::Init(args) => match sortyourpapers::init_config(args.force) {
                Ok(path) => {
                    println!("Wrote default config to {}", path.display());
                }
                Err(err) => {
                    print_error_with_hints(&err);
                    std::process::exit(1);
                }
            },
            Commands::ExtractText(args) => match sortyourpapers::run_extract_text(args).await {
                Ok(()) => {}
                Err(err) => {
                    print_error_with_hints(&err);
                    std::process::exit(1);
                }
            },
            Commands::Resume(args) => {
                match sortyourpapers::resume_run(
                    args.run_id,
                    args.apply,
                    args.verbosity,
                    args.quiet,
                )
                .await
                {
                    Ok(_) => {}
                    Err(err) => {
                        print_error_with_hints(&err);
                        std::process::exit(1);
                    }
                }
            }
        }
        return;
    }

    if let Err(err) = sortyourpapers::run_with_args(cli.run).await {
        print_error_with_hints(&err);
        std::process::exit(1);
    }
}

fn print_error_with_hints(err: &AppError) {
    eprintln!("error: {err}");

    if let AppError::MissingConfig(missing_key) = err
        && !missing_key.to_ascii_lowercase().contains("api_key")
    {
        if let Some(path) = sortyourpapers::config::xdg_config_path() {
            eprintln!(
                "hint: run `sortyourpapers init` to create a default config at {}",
                path.display()
            );
        } else {
            eprintln!("hint: run `sortyourpapers init` to create a default XDG config");
        }
    }
}
