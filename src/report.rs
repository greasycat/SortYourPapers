use crate::{logging::Verbosity, models::RunReport};

pub fn print_report(report: &RunReport, verbosity: Verbosity) {
    println!("{}", verbosity.header_stdout("SortYourPapers Summary"));
    println!(
        "{} {}",
        verbosity.muted("mode"),
        if report.dry_run {
            verbosity.warn("preview")
        } else {
            verbosity.good("apply")
        }
    );
    println!(
        "{} {}",
        verbosity.muted("scanned"),
        verbosity.accent(report.scanned.to_string())
    );
    println!(
        "{} {}",
        verbosity.muted("processed"),
        verbosity.good(report.processed.to_string())
    );
    println!(
        "{} {}",
        verbosity.muted("skipped(size)"),
        verbosity.warn(report.skipped.to_string())
    );
    println!(
        "{} {}",
        verbosity.muted("failed"),
        verbosity.bad(report.failed.to_string())
    );
    println!(
        "{} {}",
        verbosity.muted("planned_actions"),
        verbosity.accent(report.actions.len().to_string())
    );

    if !report.actions.is_empty() {
        println!();
        println!("{}", verbosity.header_stdout("Planned Actions"));
        for action in &report.actions {
            println!(
                "{} {} {} {}",
                verbosity.accent("MOVE"),
                action.source.display(),
                verbosity.muted("->"),
                action.destination.display()
            );
        }
    }
}
