use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "syptui", version, about = "SortYourPapers terminal interface")]
struct SyptuiCli {
    #[arg(long)]
    debug_tui: bool,
}

#[tokio::main]
async fn main() {
    let cli = SyptuiCli::parse();

    if let Err(err) = syptui::tui::run(cli.debug_tui).await {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}
