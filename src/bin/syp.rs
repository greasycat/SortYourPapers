use clap::Parser;
use sortyourpapers::{SypCli, print_error_with_hints, run_syp};

#[tokio::main]
async fn main() {
    let cli = SypCli::parse();

    if let Err(err) = run_syp(cli).await {
        print_error_with_hints(&err);
        std::process::exit(1);
    }
}
