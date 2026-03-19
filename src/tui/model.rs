use std::sync::mpsc;

use ratatui::prelude::{Color, Style};

use crate::{papers::taxonomy::CategoryTree, session::RunStage, terminal::InspectReviewPrompt};

#[derive(Clone, Copy)]
pub(super) enum Screen {
    Home,
    RunForm,
    Sessions,
    Operation,
}

#[derive(Default)]
pub(super) struct OperationView {
    pub(super) title: String,
    pub(super) running: bool,
    pub(super) success: bool,
    pub(super) summary: String,
    pub(super) detail: OperationDetail,
}

#[derive(Default)]
pub(super) enum OperationDetail {
    #[default]
    None,
    Tree(Vec<CategoryTree>),
}

pub(super) struct OperationOutcome {
    pub(super) title: String,
    pub(super) success: bool,
    pub(super) summary: String,
    pub(super) detail: OperationDetail,
}

impl OperationOutcome {
    pub(super) fn success(title: &str, summary: String, detail: OperationDetail) -> Self {
        Self {
            title: title.to_string(),
            success: true,
            summary,
            detail,
        }
    }

    pub(super) fn failure(title: &str, summary: String, detail: OperationDetail) -> Self {
        Self {
            title: title.to_string(),
            success: false,
            summary,
            detail,
        }
    }
}

#[derive(Clone)]
pub(super) enum ConfirmAction {
    Quit,
    RemoveRun(String),
    ClearIncomplete,
}

pub(super) enum Overlay {
    EditField {
        label: String,
        buffer: String,
    },
    InspectPrompt {
        categories: Vec<CategoryTree>,
        input: String,
        reply: mpsc::Sender<std::result::Result<InspectReviewPrompt, String>>,
    },
    ContinuePrompt {
        reply: mpsc::Sender<std::result::Result<bool, String>>,
    },
    Confirm {
        title: String,
        message: String,
        action: ConfirmAction,
    },
    SelectRerunStage {
        run_id: String,
        apply: bool,
        stages: Vec<RunStage>,
        selected: usize,
    },
}

pub(super) struct ProgressEntry {
    pub(super) id: u64,
    pub(super) label: String,
    pub(super) total: usize,
    pub(super) current: usize,
}

impl ProgressEntry {
    pub(super) fn ratio(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.current as f64 / self.total as f64
        }
    }

    pub(super) fn label(&self) -> String {
        format!("{} {}/{}", self.label, self.current, self.total)
    }

    pub(super) fn gauge_style(&self) -> Style {
        Style::default().fg(if self.current >= self.total && self.total > 0 {
            Color::Green
        } else {
            Color::Cyan
        })
    }
}
