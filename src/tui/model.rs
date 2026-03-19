use std::{collections::VecDeque, sync::mpsc};

use ratatui::prelude::{Color, Style};

use crate::{
    papers::taxonomy::CategoryTree,
    session::RunStage,
    terminal::{AlertSeverity, InspectReviewPrompt},
};

#[derive(Clone, Copy)]
pub(super) enum Screen {
    Home,
    RunForm,
    Sessions,
    Operation,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum OperationTab {
    Summary,
    Logs,
    Taxonomy,
    Report,
}

impl OperationTab {
    pub(super) const ALL: [Self; 4] = [Self::Summary, Self::Logs, Self::Taxonomy, Self::Report];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Summary => "Summary",
            Self::Logs => "Logs",
            Self::Taxonomy => "Taxonomy",
            Self::Report => "Report",
        }
    }

    pub(super) fn index(self) -> usize {
        match self {
            Self::Summary => 0,
            Self::Logs => 1,
            Self::Taxonomy => 2,
            Self::Report => 3,
        }
    }

    pub(super) fn from_index(index: usize) -> Self {
        Self::ALL[index.min(Self::ALL.len() - 1)]
    }
}

#[derive(Default)]
pub(super) enum OperationState {
    #[default]
    Idle,
    Running,
    Success,
    Failure,
}

impl OperationState {
    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }

    pub(super) fn color(&self) -> Color {
        match self {
            Self::Idle => Color::Blue,
            Self::Running => Color::Yellow,
            Self::Success => Color::Green,
            Self::Failure => Color::Red,
        }
    }
}

#[derive(Clone)]
pub(super) struct OperationAlert {
    pub(super) severity: AlertSeverity,
    pub(super) label: String,
    pub(super) message: String,
}

impl OperationAlert {
    pub(super) fn new(severity: AlertSeverity, label: String, message: String) -> Self {
        Self {
            severity,
            label,
            message,
        }
    }

    pub(super) fn line(&self) -> String {
        format!("{} {}", self.label, self.message)
    }

    pub(super) fn color(&self) -> Color {
        match self.severity {
            AlertSeverity::Warning => Color::Yellow,
            AlertSeverity::Error => Color::Red,
        }
    }
}

pub(super) struct OperationView {
    pub(super) title: String,
    pub(super) state: OperationState,
    pub(super) summary: String,
    pub(super) detail: OperationDetail,
    pub(super) active_tab: OperationTab,
    pub(super) log_scroll: u16,
    pub(super) taxonomy_scroll: u16,
    pub(super) report_scroll: u16,
    pub(super) alerts: VecDeque<OperationAlert>,
    pub(super) stage_label: String,
    pub(super) stage_message: String,
    pub(super) origin: Screen,
}

impl Default for OperationView {
    fn default() -> Self {
        Self {
            title: String::new(),
            state: OperationState::Idle,
            summary: String::new(),
            detail: OperationDetail::None,
            active_tab: OperationTab::Summary,
            log_scroll: 0,
            taxonomy_scroll: 0,
            report_scroll: 0,
            alerts: VecDeque::new(),
            stage_label: String::new(),
            stage_message: String::new(),
            origin: Screen::Home,
        }
    }
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
    Notice {
        title: String,
        message: String,
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
