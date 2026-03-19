use std::{
    collections::HashMap,
    io::{self, BufRead, IsTerminal, Write},
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use indicatif::{ProgressBar, ProgressState, ProgressStyle};

use crate::{
    error::{AppError, Result},
    models::{CategoryTree, RunReport},
};

use super::{Verbosity, report};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectReviewPrompt {
    Accept,
    Cancel,
    Suggest(String),
}

pub trait TerminalBackend: Send + Sync {
    fn stdout_is_terminal(&self) -> bool;
    fn stderr_is_terminal(&self) -> bool;
    fn supports_progress(&self) -> bool;
    fn is_interactive(&self) -> bool;
    fn write_stdout_line(&self, line: &str);
    fn write_stderr_line(&self, line: &str);
    fn start_progress(&self, id: u64, total: usize, label: &str);
    fn advance_progress(&self, id: u64, delta: usize);
    fn finish_progress(&self, id: u64);
    fn show_report(&self, report: &RunReport, verbosity: Verbosity);
    fn show_category_tree(&self, categories: &[CategoryTree], verbosity: Verbosity);
    fn prompt_inspect_review_action(
        &self,
        categories: &[CategoryTree],
        verbosity: Verbosity,
    ) -> Result<InspectReviewPrompt>;
    fn prompt_continue_improving(&self) -> Result<bool>;
}

pub struct BackendGuard {
    previous: Option<Arc<dyn TerminalBackend>>,
}

impl Drop for BackendGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            let mut backend = backend_cell()
                .lock()
                .expect("terminal backend lock should not be poisoned");
            *backend = previous;
        }
    }
}

pub fn install_backend(backend: Arc<dyn TerminalBackend>) -> BackendGuard {
    let mut current = backend_cell()
        .lock()
        .expect("terminal backend lock should not be poisoned");
    let previous = Arc::clone(&current);
    *current = backend;
    BackendGuard {
        previous: Some(previous),
    }
}

pub fn current_backend() -> Arc<dyn TerminalBackend> {
    Arc::clone(
        &backend_cell()
            .lock()
            .expect("terminal backend lock should not be poisoned"),
    )
}

fn backend_cell() -> &'static Mutex<Arc<dyn TerminalBackend>> {
    static BACKEND: OnceLock<Mutex<Arc<dyn TerminalBackend>>> = OnceLock::new();
    BACKEND.get_or_init(|| {
        let backend: Arc<dyn TerminalBackend> = Arc::new(PlainTerminalBackend::default());
        Mutex::new(backend)
    })
}

#[derive(Default)]
struct PlainTerminalBackend {
    progress: Mutex<HashMap<u64, ProgressBar>>,
}

impl TerminalBackend for PlainTerminalBackend {
    fn stdout_is_terminal(&self) -> bool {
        io::stdout().is_terminal()
    }

    fn stderr_is_terminal(&self) -> bool {
        io::stderr().is_terminal()
    }

    fn is_interactive(&self) -> bool {
        io::stdin().is_terminal()
    }

    fn supports_progress(&self) -> bool {
        self.stderr_is_terminal()
    }

    fn write_stdout_line(&self, line: &str) {
        println!("{line}");
    }

    fn write_stderr_line(&self, line: &str) {
        eprintln!("{line}");
    }

    fn start_progress(&self, id: u64, total: usize, label: &str) {
        let progress = ProgressBar::new(total as u64);
        progress.set_style(progress_style());
        progress.set_message(label.to_string());
        progress.enable_steady_tick(std::time::Duration::from_millis(100));
        self.progress
            .lock()
            .expect("progress lock should not be poisoned")
            .insert(id, progress);
    }

    fn advance_progress(&self, id: u64, delta: usize) {
        if let Some(progress) = self
            .progress
            .lock()
            .expect("progress lock should not be poisoned")
            .get(&id)
        {
            progress.inc(delta as u64);
        }
    }

    fn finish_progress(&self, id: u64) {
        if let Some(progress) = self
            .progress
            .lock()
            .expect("progress lock should not be poisoned")
            .remove(&id)
        {
            progress.finish_and_clear();
        }
    }

    fn show_report(&self, report: &RunReport, verbosity: Verbosity) {
        for line in report::render_report_lines(report, verbosity) {
            self.write_stdout_line(&line);
        }
    }

    fn show_category_tree(&self, categories: &[CategoryTree], verbosity: Verbosity) {
        for line in report::render_category_tree_lines(categories, verbosity) {
            self.write_stdout_line(&line);
        }
    }

    fn prompt_inspect_review_action(
        &self,
        _categories: &[CategoryTree],
        _verbosity: Verbosity,
    ) -> Result<InspectReviewPrompt> {
        if !self.is_interactive() {
            return Ok(InspectReviewPrompt::Accept);
        }

        let mut stdin = io::stdin().lock();
        let mut stderr = io::stderr();
        prompt_for_inspect_review_action_with_io(&mut stdin, &mut stderr)
    }

    fn prompt_continue_improving(&self) -> Result<bool> {
        if !self.is_interactive() {
            return Ok(false);
        }

        let mut stdin = io::stdin().lock();
        let mut stderr = io::stderr();
        prompt_for_continue_improving_with_io(&mut stdin, &mut stderr)
    }
}

fn progress_style() -> ProgressStyle {
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

fn prompt_for_inspect_review_action_with_io<R, W>(
    reader: &mut R,
    writer: &mut W,
) -> Result<InspectReviewPrompt>
where
    R: BufRead,
    W: Write,
{
    let mut input = String::new();
    loop {
        write!(
            writer,
            "Enter a taxonomy improvement suggestion, press Enter to continue, or type 'q' to cancel: "
        )?;
        writer.flush()?;

        input.clear();
        let bytes_read = reader.read_line(&mut input)?;
        if bytes_read == 0 {
            return Err(AppError::Execution(
                "inspect-output cancelled before a review choice was made".to_string(),
            ));
        }

        match resolve_inspect_review_action(input.trim()) {
            Ok(action) => return Ok(action),
            Err(err) => writeln!(writer, "error: {err}")?,
        }
    }
}

fn prompt_for_continue_improving_with_io<R, W>(reader: &mut R, writer: &mut W) -> Result<bool>
where
    R: BufRead,
    W: Write,
{
    let mut input = String::new();
    loop {
        write!(
            writer,
            "Continue improving this taxonomy? [y/N] (or 'q' to cancel): "
        )?;
        writer.flush()?;

        input.clear();
        let bytes_read = reader.read_line(&mut input)?;
        if bytes_read == 0 {
            return Err(AppError::Execution(
                "inspect-output cancelled before a continuation choice was made".to_string(),
            ));
        }

        match resolve_continue_improving(input.trim()) {
            Ok(InspectLoopDecision::ContinueImproving) => return Ok(true),
            Ok(InspectLoopDecision::Finish) => return Ok(false),
            Ok(InspectLoopDecision::Cancel) => {
                return Err(AppError::Execution("inspect-output cancelled".to_string()));
            }
            Err(err) => writeln!(writer, "error: {err}")?,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InspectLoopDecision {
    ContinueImproving,
    Finish,
    Cancel,
}

fn resolve_inspect_review_action(input: &str) -> Result<InspectReviewPrompt> {
    if input.is_empty() {
        return Ok(InspectReviewPrompt::Accept);
    }

    if input.eq_ignore_ascii_case("q") || input.eq_ignore_ascii_case("quit") {
        return Ok(InspectReviewPrompt::Cancel);
    }

    Ok(InspectReviewPrompt::Suggest(input.to_string()))
}

fn resolve_continue_improving(input: &str) -> Result<InspectLoopDecision> {
    if input.is_empty() || input.eq_ignore_ascii_case("n") || input.eq_ignore_ascii_case("no") {
        return Ok(InspectLoopDecision::Finish);
    }

    if input.eq_ignore_ascii_case("y") || input.eq_ignore_ascii_case("yes") {
        return Ok(InspectLoopDecision::ContinueImproving);
    }

    if input.eq_ignore_ascii_case("q") || input.eq_ignore_ascii_case("quit") {
        return Ok(InspectLoopDecision::Cancel);
    }

    Err(AppError::Execution(
        "enter 'y' to keep improving, press Enter to continue, or type 'q' to cancel".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{
        InspectReviewPrompt, prompt_for_continue_improving_with_io,
        prompt_for_inspect_review_action_with_io,
    };

    #[test]
    fn inspect_prompt_accepts_empty_input() {
        let mut input = Cursor::new(b"\n");
        let mut output = Vec::new();

        let result = prompt_for_inspect_review_action_with_io(&mut input, &mut output)
            .expect("prompt should accept empty input");

        assert_eq!(result, InspectReviewPrompt::Accept);
    }

    #[test]
    fn continue_prompt_accepts_yes() {
        let mut input = Cursor::new(b"y\n");
        let mut output = Vec::new();

        let result = prompt_for_continue_improving_with_io(&mut input, &mut output)
            .expect("prompt should accept yes");

        assert!(result);
    }
}
