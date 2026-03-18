mod backend;
pub mod report;

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use indicatif::{ProgressState, ProgressStyle};

pub use backend::{
    BackendGuard, InspectReviewPrompt, TerminalBackend, current_backend, install_backend,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verbosity {
    verbose: bool,
    debug: bool,
    quiet: bool,
}

impl Verbosity {
    pub fn new(verbose: bool, debug: bool, quiet: bool) -> Self {
        Self {
            verbose,
            debug,
            quiet,
        }
    }

    pub fn quiet(self) -> bool {
        self.quiet
    }

    pub fn silenced(self) -> Self {
        Self {
            verbose: self.verbose,
            debug: self.debug,
            quiet: true,
        }
    }

    pub fn stage_silenced(self) -> Self {
        Self {
            verbose: false,
            debug: self.debug,
            quiet: self.quiet,
        }
    }

    pub fn verbose_enabled(self) -> bool {
        self.verbose && !self.quiet
    }

    pub fn debug_enabled(self) -> bool {
        self.debug && !self.quiet
    }

    pub fn use_color_stderr(self) -> bool {
        !self.quiet && current_backend().stderr_is_terminal()
    }

    pub fn use_color_stdout(self) -> bool {
        !self.quiet && current_backend().stdout_is_terminal()
    }

    pub fn show_stage_output(self) -> bool {
        self.verbose_enabled()
    }

    pub fn show_progress(self, total: usize, allow_single: bool) -> bool {
        self.show_progress_with_terminal(total, allow_single, current_backend().supports_progress())
    }

    fn show_progress_with_terminal(
        self,
        total: usize,
        allow_single: bool,
        is_terminal: bool,
    ) -> bool {
        !self.quiet && !self.debug && is_terminal && total > 0 && (allow_single || total > 1)
    }

    pub fn info(self, message: impl AsRef<str>) {
        if !self.quiet {
            current_backend().write_stderr_line(message.as_ref());
        }
    }

    pub fn debug(self, message: impl AsRef<str>) {
        if self.debug_enabled() {
            current_backend().write_stderr_line(message.as_ref());
        }
    }

    pub fn stage_line(self, stage: &str, message: impl AsRef<str>) {
        if !self.show_stage_output() {
            return;
        }
        let stage = self.paint(stage, "1;34");
        let bar = self.paint("----------", "1;36");
        self.info(format!("{bar} {stage} {}", message.as_ref()));
    }

    pub fn run_line(self, label: &str, message: impl AsRef<str>) {
        if !self.show_stage_output() {
            return;
        }
        let tag = self.paint(label, "1;35");
        self.info(format!("{tag} {}", message.as_ref()));
    }

    pub fn success_line(self, label: &str, message: impl AsRef<str>) {
        if !self.show_stage_output() {
            return;
        }
        let tag = self.paint(label, "1;32");
        self.info(format!("{tag} {}", message.as_ref()));
    }

    pub fn warn_line(self, label: &str, message: impl AsRef<str>) {
        let tag = self.paint(label, "1;33");
        current_backend().write_stderr_line(&format!("{tag} {}", message.as_ref()));
    }

    pub fn error_line(self, label: &str, message: impl AsRef<str>) {
        let tag = self.paint(label, "1;31");
        current_backend().write_stderr_line(&format!("{tag} {}", message.as_ref()));
    }

    pub fn debug_line(self, label: &str, message: impl AsRef<str>) {
        let tag = self.paint(label, "1;90");
        self.debug(format!("{tag} {}", message.as_ref()));
    }

    pub fn header_stdout(self, title: &str) -> String {
        if self.use_color_stdout() {
            format!("\x1b[1;37;46m {title} \x1b[0m")
        } else {
            format!("== {title} ==")
        }
    }

    pub fn good(self, text: impl AsRef<str>) -> String {
        self.paint(text.as_ref(), "1;32")
    }

    pub fn warn(self, text: impl AsRef<str>) -> String {
        self.paint(text.as_ref(), "1;33")
    }

    pub fn bad(self, text: impl AsRef<str>) -> String {
        self.paint(text.as_ref(), "1;31")
    }

    pub fn accent(self, text: impl AsRef<str>) -> String {
        self.paint(text.as_ref(), "1;36")
    }

    pub fn muted(self, text: impl AsRef<str>) -> String {
        self.paint(text.as_ref(), "90")
    }

    fn paint(self, text: &str, code: &str) -> String {
        if self.use_color_stderr() || self.use_color_stdout() {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }
}

pub struct ProgressTracker {
    id: Option<u64>,
}

impl ProgressTracker {
    pub fn new(verbosity: Verbosity, total: usize, label: &str, allow_single: bool) -> Self {
        let id = verbosity.show_progress(total, allow_single).then(|| {
            let id = next_progress_id();
            current_backend().start_progress(id, total, label);
            id
        });
        Self { id }
    }

    pub fn inc(&mut self, delta: usize) {
        if let Some(id) = self.id {
            current_backend().advance_progress(id, delta);
        }
    }

    pub fn finish(&mut self) {
        if let Some(id) = self.id.take() {
            current_backend().finish_progress(id);
        }
    }
}

pub(crate) fn progress_style() -> ProgressStyle {
    match ProgressStyle::with_template(
        "{spinner:.cyan} {msg} [{wide_bar:.cyan/blue}] {pos}/{len} [{elapsed_mm_ss}/{eta_mm_ss}]",
    ) {
        Ok(style) => style
            .with_key(
                "elapsed_mm_ss",
                |state: &ProgressState, w: &mut dyn std::fmt::Write| {
                    let _ = write!(w, "{}", format_minutes_seconds(state.elapsed()));
                },
            )
            .with_key(
                "eta_mm_ss",
                |state: &ProgressState, w: &mut dyn std::fmt::Write| {
                    let _ = write!(w, "{}", format_minutes_seconds(state.eta()));
                },
            )
            .progress_chars("=> "),
        Err(_) => ProgressStyle::default_bar(),
    }
}

fn format_minutes_seconds(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes:02}:{seconds:02}")
}

pub fn format_duration(duration: Duration) -> String {
    if duration.as_secs_f64() >= 1.0 {
        format!("{:.3}s", duration.as_secs_f64())
    } else {
        format!("{:.1}ms", duration.as_secs_f64() * 1000.0)
    }
}

pub fn terminal_is_interactive() -> bool {
    current_backend().is_interactive()
}

pub fn prompt_inspect_review_action(
    categories: &[crate::models::CategoryTree],
    verbosity: Verbosity,
) -> crate::error::Result<InspectReviewPrompt> {
    current_backend().prompt_inspect_review_action(categories, verbosity)
}

pub fn prompt_continue_improving() -> crate::error::Result<bool> {
    current_backend().prompt_continue_improving()
}

fn next_progress_id() -> u64 {
    static NEXT_PROGRESS_ID: AtomicU64 = AtomicU64::new(1);
    NEXT_PROGRESS_ID.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::Verbosity;

    #[test]
    fn stage_output_requires_verbose_mode() {
        assert!(!Verbosity::new(false, false, false).show_stage_output());
        assert!(Verbosity::new(true, false, false).show_stage_output());
        assert!(!Verbosity::new(true, false, true).show_stage_output());
    }

    #[test]
    fn progress_requires_terminal_and_non_quiet_mode() {
        let verbosity = Verbosity::new(false, false, false);
        assert!(verbosity.show_progress_with_terminal(2, false, true));
        assert!(!verbosity.show_progress_with_terminal(2, false, false));
        assert!(!Verbosity::new(false, false, true).show_progress_with_terminal(2, false, true));
        assert!(!Verbosity::new(true, true, false).show_progress_with_terminal(2, false, true));
    }

    #[test]
    fn progress_can_opt_into_single_item_work() {
        let verbosity = Verbosity::new(false, false, false);
        assert!(!verbosity.show_progress_with_terminal(1, false, true));
        assert!(verbosity.show_progress_with_terminal(1, true, true));
        assert!(!verbosity.show_progress_with_terminal(0, true, true));
    }

    #[test]
    fn stage_silenced_preserves_debug_output() {
        let verbosity = Verbosity::new(true, true, false).stage_silenced();
        assert!(!verbosity.show_stage_output());
        assert!(verbosity.debug_enabled());
    }
}
