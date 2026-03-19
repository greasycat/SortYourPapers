use std::sync::mpsc::{self, Sender};

use crate::{
    error::{AppError, Result},
    report::RunReport,
    taxonomy::CategoryTree,
    terminal::{InspectReviewPrompt, TerminalBackend, Verbosity},
};

#[derive(Debug)]
pub(super) enum BackendEvent {
    StdoutLine(String),
    StderrLine(String),
    ProgressStart {
        id: u64,
        total: usize,
        label: String,
    },
    ProgressAdvance {
        id: u64,
        delta: usize,
    },
    ProgressFinish {
        id: u64,
    },
    Report(RunReport),
    CategoryTree(Vec<CategoryTree>),
    PromptInspectReview {
        categories: Vec<CategoryTree>,
        reply: mpsc::Sender<std::result::Result<InspectReviewPrompt, String>>,
    },
    PromptContinueImproving {
        reply: mpsc::Sender<std::result::Result<bool, String>>,
    },
}

#[derive(Clone)]
pub(super) struct TuiBackend {
    tx: Sender<BackendEvent>,
}

impl TuiBackend {
    pub(super) fn new(tx: Sender<BackendEvent>) -> Self {
        Self { tx }
    }

    fn send(&self, event: BackendEvent) {
        let _ = self.tx.send(event);
    }
}

impl TerminalBackend for TuiBackend {
    fn stdout_is_terminal(&self) -> bool {
        false
    }

    fn stderr_is_terminal(&self) -> bool {
        false
    }

    fn supports_progress(&self) -> bool {
        true
    }

    fn is_interactive(&self) -> bool {
        true
    }

    fn write_stdout_line(&self, line: &str) {
        self.send(BackendEvent::StdoutLine(line.to_string()));
    }

    fn write_stderr_line(&self, line: &str) {
        self.send(BackendEvent::StderrLine(line.to_string()));
    }

    fn start_progress(&self, id: u64, total: usize, label: &str) {
        self.send(BackendEvent::ProgressStart {
            id,
            total,
            label: label.to_string(),
        });
    }

    fn advance_progress(&self, id: u64, delta: usize) {
        self.send(BackendEvent::ProgressAdvance { id, delta });
    }

    fn finish_progress(&self, id: u64) {
        self.send(BackendEvent::ProgressFinish { id });
    }

    fn show_report(&self, report: &RunReport, _verbosity: Verbosity) {
        self.send(BackendEvent::Report(report.clone()));
    }

    fn show_category_tree(&self, categories: &[CategoryTree], _verbosity: Verbosity) {
        self.send(BackendEvent::CategoryTree(categories.to_vec()));
    }

    fn prompt_inspect_review_action(
        &self,
        categories: &[CategoryTree],
        _verbosity: Verbosity,
    ) -> Result<InspectReviewPrompt> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.send(BackendEvent::PromptInspectReview {
            categories: categories.to_vec(),
            reply: reply_tx,
        });
        match reply_rx.recv() {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(message)) => Err(AppError::Execution(message)),
            Err(_) => Err(AppError::Execution(
                "tui prompt closed before a taxonomy review choice was made".to_string(),
            )),
        }
    }

    fn prompt_continue_improving(&self) -> Result<bool> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.send(BackendEvent::PromptContinueImproving { reply: reply_tx });
        match reply_rx.recv() {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(message)) => Err(AppError::Execution(message)),
            Err(_) => Err(AppError::Execution(
                "tui prompt closed before an inspect-output continuation choice was made"
                    .to_string(),
            )),
        }
    }
}
