use std::{env, ffi::OsStr, ffi::OsString};

use clap::Parser;
use sortyourpapers::{Cli, SypCli, print_error_with_hints, run_cli, run_syp};

#[tokio::main]
async fn main() {
    let argv = env::args_os().collect::<Vec<_>>();

    if argv.get(1).is_some_and(|arg| arg == OsStr::new("cli")) {
        let mut forwarded = Vec::with_capacity(argv.len().saturating_sub(1));
        forwarded.push(OsString::from("sortyourpapers"));
        forwarded.extend(argv.into_iter().skip(2));

        if let Err(err) = run_cli(Cli::parse_from(forwarded)).await {
            print_error_with_hints(&err);
            std::process::exit(1);
        }
        return;
    }

    let cli = SypCli::parse_from(argv);

    if let Err(err) = run_syp(cli).await {
        print_error_with_hints(&err);
        std::process::exit(1);
    }
}
