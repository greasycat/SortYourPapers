use clap::Parser;
use sortyourpapers::{Cli, print_error_with_hints, run_cli};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if let Err(err) = run_cli(cli).await {
        print_error_with_hints(&err);
        std::process::exit(1);
    }
}
