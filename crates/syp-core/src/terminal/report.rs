use crate::{
    llm::LlmUsageSummary, papers::taxonomy::CategoryTree, report::RunReport, terminal::Verbosity,
};

pub fn print_report(report: &RunReport, verbosity: Verbosity) {
    super::current_backend().show_report(report, verbosity);
}

pub fn print_category_tree(categories: &[CategoryTree], verbosity: Verbosity) {
    super::current_backend().show_category_tree(categories, verbosity);
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

pub fn render_report_lines(report: &RunReport, verbosity: Verbosity) -> Vec<String> {
    let mut lines = render_report_summary_lines(report, verbosity);
    let action_lines = render_report_action_lines(report, verbosity);
    if !action_lines.is_empty() {
        lines.push(String::new());
        lines.extend(action_lines);
    }
    lines
}

pub fn render_report_summary_lines(report: &RunReport, verbosity: Verbosity) -> Vec<String> {
    let mut lines = vec![
        verbosity.header_stdout("SortYourPapers Summary"),
        format!(
            "{} {}",
            verbosity.muted("mode"),
            if report.dry_run {
                verbosity.warn("preview")
            } else {
                verbosity.good("apply")
            }
        ),
        format!(
            "{} {}",
            verbosity.muted("scanned"),
            verbosity.accent(report.scanned.to_string())
        ),
        format!(
            "{} {}",
            verbosity.muted("processed"),
            verbosity.good(report.processed.to_string())
        ),
        format!(
            "{} {}",
            verbosity.muted("skipped(size)"),
            verbosity.warn(report.skipped.to_string())
        ),
        format!(
            "{} {}",
            verbosity.muted("failed"),
            verbosity.bad(report.failed.to_string())
        ),
        format!(
            "{} {}",
            verbosity.muted("planned_actions"),
            verbosity.accent(report.actions.len().to_string())
        ),
    ];

    if report.llm_usage.has_activity() {
        lines.push(String::new());
        lines.push(verbosity.header_stdout("LLM Usage"));
        render_llm_usage_stage(
            "keywords",
            &report.llm_usage.keywords,
            verbosity,
            &mut lines,
        );
        render_llm_usage_stage(
            "taxonomy",
            &report.llm_usage.taxonomy,
            verbosity,
            &mut lines,
        );
        render_llm_usage_stage(
            "placements",
            &report.llm_usage.placements,
            verbosity,
            &mut lines,
        );
    }

    lines
}

pub fn render_report_action_lines(report: &RunReport, verbosity: Verbosity) -> Vec<String> {
    if report.actions.is_empty() {
        return Vec::new();
    }

    let mut lines = vec![verbosity.header_stdout("Planned Actions")];
    for action in &report.actions {
        lines.push(format!(
            "{} {} {} {}",
            verbosity.accent("MOVE"),
            action.source.display(),
            verbosity.muted("->"),
            action.destination.display()
        ));
    }
    lines
}

pub(crate) fn render_category_tree_lines(
    categories: &[CategoryTree],
    verbosity: Verbosity,
) -> Vec<String> {
    vec![
        String::new(),
        verbosity.header_stdout("Final Categories"),
        render_category_tree(categories),
    ]
}

fn render_llm_usage_stage(
    label: &str,
    usage: &LlmUsageSummary,
    verbosity: Verbosity,
    lines: &mut Vec<String>,
) {
    if !usage.has_activity() {
        return;
    }

    lines.push(format!(
        "{} calls={} http_attempts={} request_chars={} response_chars={} json_retries={} semantic_retries={}",
        verbosity.accent(label),
        usage.call_count,
        usage.http_attempt_count,
        usage.request_chars,
        usage.response_chars,
        usage.json_retry_count,
        usage.semantic_retry_count,
    ));

    if usage.calls_with_native_tokens > 0 {
        lines.push(format!(
            "  {} prompt={} completion={} total={} native_token_calls={}",
            verbosity.muted("tokens"),
            usage.input_tokens,
            usage.output_tokens,
            usage.total_tokens,
            usage.calls_with_native_tokens,
        ));
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
