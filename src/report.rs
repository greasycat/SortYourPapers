use crate::models::RunReport;

pub fn print_report(report: &RunReport) {
    println!("SortYourPapers run summary");
    println!("- dry_run: {}", report.dry_run);
    println!("- scanned: {}", report.scanned);
    println!("- processed: {}", report.processed);
    println!("- skipped(size): {}", report.skipped);
    println!("- failed: {}", report.failed);
    println!("- planned_actions: {}", report.actions.len());

    if !report.actions.is_empty() {
        println!("\nActions:");
        for action in &report.actions {
            println!(
                "- MOVE {} -> {}",
                action.source.display(),
                action.destination.display()
            );
        }
    }
}
