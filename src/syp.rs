use std::ffi::OsString;

use clap::{Args, Parser, Subcommand};

use crate::{Cli, error::Result, tui};

#[derive(Debug, Parser)]
#[command(name = "syp", version, about = "SortYourPapers terminal interface")]
pub struct SypCli {
    #[command(subcommand)]
    pub command: Option<SypCommands>,
}

#[derive(Debug, Subcommand)]
pub enum SypCommands {
    Tui,
    Cli(ForwardCliArgs),
}

#[derive(Debug, Args)]
pub struct ForwardCliArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<OsString>,
}

/// Dispatches the `syp` frontend entrypoint.
///
/// # Errors
/// Returns an error when the selected mode fails.
pub async fn run_syp(cli: SypCli) -> Result<()> {
    match cli.command {
        None | Some(SypCommands::Tui) => tui::run().await,
        Some(SypCommands::Cli(args)) => {
            crate::entrypoints::run_cli(parse_forwarded_cli(args)).await
        }
    }
}

fn parse_forwarded_cli(args: ForwardCliArgs) -> Cli {
    let mut argv = Vec::with_capacity(args.args.len() + 1);
    argv.push(OsString::from("sortyourpapers"));
    argv.extend(args.args);
    Cli::parse_from(argv)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{SypCli, SypCommands};

    #[test]
    fn defaults_to_tui_without_subcommand() {
        let cli = SypCli::parse_from(["syp"]);
        assert!(cli.command.is_none());
    }

    #[test]
    fn parses_explicit_tui_subcommand() {
        let cli = SypCli::parse_from(["syp", "tui"]);
        assert!(matches!(cli.command, Some(SypCommands::Tui)));
    }

    #[test]
    fn parses_forwarded_cli_args() {
        let cli = SypCli::parse_from(["syp", "cli", "session", "resume", "run-123", "--apply"]);

        let Some(SypCommands::Cli(forwarded)) = cli.command else {
            panic!("expected forwarded cli command");
        };

        assert_eq!(
            forwarded
                .args
                .into_iter()
                .map(|value| value.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec!["session", "resume", "run-123", "--apply"]
        );
    }
}
