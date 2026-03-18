use crate::{
    logging::Verbosity,
    models::{CategoryTree, LlmUsageSummary, RunReport},
};

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

    if report.llm_usage.has_activity() {
        println!();
        println!("{}", verbosity.header_stdout("LLM Usage"));
        print_llm_usage_stage("keywords", &report.llm_usage.keywords, verbosity);
        print_llm_usage_stage("taxonomy", &report.llm_usage.taxonomy, verbosity);
        print_llm_usage_stage("placements", &report.llm_usage.placements, verbosity);
    }
}

pub fn print_category_tree(categories: &[CategoryTree], verbosity: Verbosity) {
    println!();
    println!("{}", verbosity.header_stdout("Final Categories"));
    println!("{}", render_category_tree(categories));
}

#[must_use]
pub fn render_category_tree(categories: &[CategoryTree]) -> String {
    if categories.is_empty() {
        return "<empty>".to_string();
    }

    let mut lines = Vec::new();
    for category in categories {
        lines.push(category.name.clone());
        render_category_children(&category.children, String::new(), &mut lines);
    }
    lines.join("\n")
}

fn print_llm_usage_stage(label: &str, usage: &LlmUsageSummary, verbosity: Verbosity) {
    if !usage.has_activity() {
        return;
    }

    println!(
        "{} calls={} http_attempts={} request_chars={} response_chars={} json_retries={} semantic_retries={}",
        verbosity.accent(label),
        usage.call_count,
        usage.http_attempt_count,
        usage.request_chars,
        usage.response_chars,
        usage.json_retry_count,
        usage.semantic_retry_count,
    );

    if usage.calls_with_native_tokens > 0 {
        println!(
            "  {} prompt={} completion={} total={} native_token_calls={}",
            verbosity.muted("tokens"),
            usage.input_tokens,
            usage.output_tokens,
            usage.total_tokens,
            usage.calls_with_native_tokens,
        );
    }
}

fn render_category_children(children: &[CategoryTree], prefix: String, lines: &mut Vec<String>) {
    for (index, child) in children.iter().enumerate() {
        let is_last = index + 1 == children.len();
        let branch = if is_last { "\\-- " } else { "|-- " };
        lines.push(format!("{prefix}{branch}{}", child.name));
        let next_prefix = if is_last {
            format!("{prefix}    ")
        } else {
            format!("{prefix}|   ")
        };
        render_category_children(&child.children, next_prefix, lines);
    }
}
