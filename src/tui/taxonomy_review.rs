use std::sync::mpsc;

use crate::{papers::taxonomy::CategoryTree, terminal::InspectReviewPrompt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReviewPhase {
    Drafting,
    WaitingForModel,
    PostSuggestionDecision,
}

impl ReviewPhase {
    pub(super) fn label(self, has_pending_reply: bool) -> &'static str {
        match self {
            Self::Drafting if has_pending_reply => "Drafting",
            Self::Drafting => "Preparing Next Iteration",
            Self::WaitingForModel => "Waiting For Model",
            Self::PostSuggestionDecision => "Review Candidate",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReviewPane {
    Suggestion,
    IterationTaxonomy,
    History,
}

impl ReviewPane {
    const ALL: [Self; 3] = [Self::Suggestion, Self::IterationTaxonomy, Self::History];

    pub(super) fn index(self) -> usize {
        match self {
            Self::Suggestion => 0,
            Self::IterationTaxonomy => 1,
            Self::History => 2,
        }
    }

    fn from_index(index: usize) -> Self {
        Self::ALL[index.min(Self::ALL.len() - 1)]
    }
}

#[derive(Debug, Clone)]
pub(super) struct ReviewIteration {
    pub(super) number: usize,
    pub(super) suggestion: String,
    pub(super) accepted_categories: Vec<CategoryTree>,
    pub(super) suggested_categories: Vec<CategoryTree>,
}

pub(super) enum PendingReviewReply {
    Inspect(mpsc::Sender<std::result::Result<InspectReviewPrompt, String>>),
    Continue(mpsc::Sender<std::result::Result<bool, String>>),
}

pub(super) struct TaxonomyReviewView {
    pub(super) accepted_categories: Vec<CategoryTree>,
    pub(super) candidate_categories: Option<Vec<CategoryTree>>,
    pub(super) suggestion_buffer: String,
    pub(super) last_submitted_suggestion: Option<String>,
    pub(super) history: Vec<ReviewIteration>,
    pub(super) iteration_scroll: u16,
    pub(super) history_scroll: u16,
    pub(super) history_selection: usize,
    pub(super) focused_pane: ReviewPane,
    pub(super) phase: ReviewPhase,
    pub(super) editing: bool,
    pub(super) pending_reply: Option<PendingReviewReply>,
}

impl TaxonomyReviewView {
    pub(super) fn new(
        categories: Vec<CategoryTree>,
        reply: mpsc::Sender<std::result::Result<InspectReviewPrompt, String>>,
    ) -> Self {
        Self {
            accepted_categories: categories,
            candidate_categories: None,
            suggestion_buffer: String::new(),
            last_submitted_suggestion: None,
            history: Vec::new(),
            iteration_scroll: 0,
            history_scroll: 0,
            history_selection: 0,
            focused_pane: ReviewPane::Suggestion,
            phase: ReviewPhase::Drafting,
            editing: false,
            pending_reply: Some(PendingReviewReply::Inspect(reply)),
        }
    }

    pub(super) fn begin_iteration(
        &mut self,
        categories: Vec<CategoryTree>,
        reply: mpsc::Sender<std::result::Result<InspectReviewPrompt, String>>,
    ) {
        self.accepted_categories = categories;
        self.candidate_categories = None;
        self.suggestion_buffer.clear();
        self.iteration_scroll = 0;
        self.history_selection = 0;
        self.history_scroll = 0;
        self.focused_pane = ReviewPane::Suggestion;
        self.phase = ReviewPhase::Drafting;
        self.editing = false;
        self.pending_reply = Some(PendingReviewReply::Inspect(reply));
    }

    pub(super) fn register_candidate(&mut self, categories: Vec<CategoryTree>) {
        if !matches!(self.phase, ReviewPhase::WaitingForModel) {
            return;
        }

        self.candidate_categories = Some(categories.clone());
        self.iteration_scroll = 0;
        if let Some(suggestion) = self.last_submitted_suggestion.clone() {
            self.history.push(ReviewIteration {
                number: self.history.len() + 1,
                suggestion,
                accepted_categories: self.accepted_categories.clone(),
                suggested_categories: categories,
            });
            self.history_selection = self.history.len();
            self.sync_history_scroll();
        }
    }

    pub(super) fn set_continue_prompt(
        &mut self,
        reply: mpsc::Sender<std::result::Result<bool, String>>,
    ) {
        self.focused_pane = ReviewPane::IterationTaxonomy;
        self.phase = ReviewPhase::PostSuggestionDecision;
        self.editing = false;
        self.pending_reply = Some(PendingReviewReply::Continue(reply));
    }

    pub(super) fn start_editing(&mut self) {
        if matches!(self.phase, ReviewPhase::Drafting) {
            self.focused_pane = ReviewPane::Suggestion;
            self.editing = true;
        }
    }

    pub(super) fn stop_editing(&mut self) {
        self.editing = false;
    }

    pub(super) fn append_input(&mut self, character: char) {
        self.suggestion_buffer.push(character);
    }

    pub(super) fn pop_input(&mut self) {
        self.suggestion_buffer.pop();
    }

    pub(super) fn submit_suggestion(&mut self) -> Option<String> {
        let suggestion = self.suggestion_buffer.trim().to_string();
        if suggestion.is_empty() {
            return None;
        }

        self.last_submitted_suggestion = Some(suggestion.clone());
        self.phase = ReviewPhase::WaitingForModel;
        self.focused_pane = ReviewPane::IterationTaxonomy;
        self.iteration_scroll = 0;
        self.editing = false;
        Some(suggestion)
    }

    pub(super) fn promote_candidate_to_accepted(&mut self) {
        if let Some(candidate) = self.candidate_categories.take() {
            self.accepted_categories = candidate;
        }
        self.suggestion_buffer.clear();
        self.iteration_scroll = 0;
        self.history_selection = 0;
        self.history_scroll = 0;
        self.phase = ReviewPhase::Drafting;
        self.focused_pane = ReviewPane::Suggestion;
        self.editing = false;
        self.pending_reply = None;
    }

    pub(super) fn has_pending_inspect_prompt(&self) -> bool {
        matches!(self.pending_reply, Some(PendingReviewReply::Inspect(_)))
    }

    pub(super) fn take_inspect_reply(
        &mut self,
    ) -> Option<mpsc::Sender<std::result::Result<InspectReviewPrompt, String>>> {
        match self.pending_reply.take() {
            Some(PendingReviewReply::Inspect(reply)) => Some(reply),
            other => {
                self.pending_reply = other;
                None
            }
        }
    }

    pub(super) fn take_continue_reply(
        &mut self,
    ) -> Option<mpsc::Sender<std::result::Result<bool, String>>> {
        match self.pending_reply.take() {
            Some(PendingReviewReply::Continue(reply)) => Some(reply),
            other => {
                self.pending_reply = other;
                None
            }
        }
    }

    pub(super) fn phase_label(&self) -> &'static str {
        self.phase.label(self.pending_reply.is_some())
    }

    pub(super) fn focused_scroll(&self) -> Option<u16> {
        match self.focused_pane {
            ReviewPane::Suggestion => None,
            ReviewPane::IterationTaxonomy => Some(self.iteration_scroll),
            ReviewPane::History => Some(self.history_scroll),
        }
    }

    pub(super) fn scroll_focused(&mut self, delta: isize) {
        match self.focused_pane {
            ReviewPane::Suggestion => {}
            ReviewPane::IterationTaxonomy => {
                let max_offset = self.iteration_taxonomy_lines().len().saturating_sub(1);
                self.iteration_scroll = (self.iteration_scroll as isize + delta)
                    .clamp(0, max_offset.min(u16::MAX as usize) as isize)
                    as u16;
            }
            ReviewPane::History => self.select_history_delta(delta),
        }
    }

    pub(super) fn jump_focused(&mut self, to_end: bool) {
        match self.focused_pane {
            ReviewPane::Suggestion => {}
            ReviewPane::IterationTaxonomy => {
                let target = self.iteration_taxonomy_lines().len().saturating_sub(1);
                self.iteration_scroll = if to_end {
                    target.min(u16::MAX as usize) as u16
                } else {
                    0
                };
            }
            ReviewPane::History => {
                self.history_selection = if to_end { self.history.len() } else { 0 };
                self.sync_history_scroll();
                self.iteration_scroll = 0;
            }
        }
    }

    pub(super) fn focus_next(&mut self) {
        let next = self.focused_pane.index() + 1;
        self.focused_pane = ReviewPane::from_index(next);
    }

    pub(super) fn focus_previous(&mut self) {
        let current = self.focused_pane.index();
        self.focused_pane = ReviewPane::from_index(current.saturating_sub(1));
    }

    fn category_lines(categories: &[CategoryTree]) -> Vec<String> {
        crate::terminal::report::render_category_tree(categories)
            .lines()
            .map(ToOwned::to_owned)
            .collect()
    }

    fn selected_iteration(&self) -> Option<&ReviewIteration> {
        self.history_selection
            .checked_sub(1)
            .and_then(|index| self.history.get(index))
    }

    fn sync_history_scroll(&mut self) {
        self.history_scroll = self
            .history_selection
            .saturating_sub(1)
            .min(u16::MAX as usize) as u16;
    }

    fn select_history_delta(&mut self, delta: isize) {
        let max_index = self.history.len();
        self.history_selection = (self.history_selection as isize + delta)
            .clamp(0, max_index.min(isize::MAX as usize) as isize)
            as usize;
        self.sync_history_scroll();
        self.iteration_scroll = 0;
    }

    fn current_suggested_lines(&self) -> Vec<String> {
        self.candidate_categories
            .as_ref()
            .map(|categories| Self::category_lines(categories))
            .unwrap_or_else(|| match self.phase {
                ReviewPhase::Drafting => {
                    vec!["No suggested taxonomy yet for the current iteration.".to_string()]
                }
                ReviewPhase::WaitingForModel => {
                    vec!["Waiting for the suggested taxonomy from the model.".to_string()]
                }
                ReviewPhase::PostSuggestionDecision => {
                    vec!["Suggested taxonomy unavailable for this iteration.".to_string()]
                }
            })
    }

    pub(super) fn iteration_taxonomy_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(iteration) = self.selected_iteration() {
            lines.push(format!("Iteration {}", iteration.number));
            lines.push(format!("Suggestion: {}", iteration.suggestion));
            lines.push(String::new());
            lines.push("Accepted Taxonomy".to_string());
            lines.extend(Self::category_lines(&iteration.accepted_categories));
            lines.push(String::new());
            lines.push("Suggested Taxonomy".to_string());
            lines.extend(Self::category_lines(&iteration.suggested_categories));
            return lines;
        }

        lines.push("Current Iteration".to_string());
        if let Some(suggestion) = self.last_submitted_suggestion.as_deref()
            && !matches!(self.phase, ReviewPhase::Drafting)
        {
            lines.push(format!("Latest suggestion: {suggestion}"));
        }
        lines.push(String::new());
        lines.push("Accepted Taxonomy".to_string());
        lines.extend(Self::category_lines(&self.accepted_categories));
        lines.push(String::new());
        lines.push("Suggested Taxonomy".to_string());
        lines.extend(self.current_suggested_lines());
        lines
    }

    pub(super) fn history_lines(&self) -> Vec<String> {
        let mut lines = vec![if self.history_selection == 0 {
            "> Current".to_string()
        } else {
            "  Current".to_string()
        }];
        for (index, iteration) in self.history.iter().enumerate() {
            let marker = if self.history_selection == index + 1 {
                ">"
            } else {
                " "
            };
            lines.push(format!(
                "{marker} Iteration {}: {}",
                iteration.number, iteration.suggestion
            ));
        }
        lines
    }

    pub(super) fn status_lines(&self) -> Vec<String> {
        let iteration_count = self.history.len();
        let mut lines = vec![
            format!("phase: {}", self.phase_label()),
            format!("iterations: {iteration_count}"),
        ];

        match self.phase {
            ReviewPhase::Drafting if self.has_pending_inspect_prompt() => {
                lines.push("Compare the current taxonomy and draft a focused change.".to_string());
                lines
                    .push("Accept with `a`, or press `s` to draft the next iteration.".to_string());
            }
            ReviewPhase::Drafting => {
                lines.push("Preparing the next iteration request.".to_string());
                lines.push("The current taxonomy will become the next baseline.".to_string());
            }
            ReviewPhase::WaitingForModel => {
                lines.push("The suggestion has been sent to the model.".to_string());
                lines.push(
                    "Wait for the suggested taxonomy to populate the iteration panel.".to_string(),
                );
            }
            ReviewPhase::PostSuggestionDecision => {
                lines.push(
                    "Review the selected iteration in the iteration taxonomy panel.".to_string(),
                );
                lines.push("Accept with `a`, iterate with `i`, or cancel with `c`.".to_string());
            }
        }

        lines
    }

    pub(super) fn suggestion_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        match self.phase {
            ReviewPhase::Drafting if self.editing => {
                lines.push(
                    "Editing suggestion. Press Enter to submit or Esc to stop editing.".to_string(),
                );
            }
            ReviewPhase::Drafting => {
                lines.push("No suggestion submitted for this iteration yet.".to_string());
                lines.push(
                    "Press `s` to edit a suggestion or `a` to accept the current taxonomy."
                        .to_string(),
                );
            }
            ReviewPhase::WaitingForModel => {
                lines.push("Submitted suggestion:".to_string());
                lines.push(
                    self.last_submitted_suggestion
                        .clone()
                        .unwrap_or_else(|| "<missing>".to_string()),
                );
                lines.push(String::new());
                lines.push("Waiting for an updated taxonomy...".to_string());
            }
            ReviewPhase::PostSuggestionDecision => {
                lines.push("Current suggestion:".to_string());
                lines.push(
                    self.last_submitted_suggestion
                        .clone()
                        .unwrap_or_else(|| "<missing>".to_string()),
                );
                lines.push(String::new());
                lines.push(
                    "Use History to switch iterations and Iteration Taxonomy to inspect them."
                        .to_string(),
                );
            }
        }

        if self.editing {
            lines.push(String::new());
            lines.push(format!("draft: {}", self.suggestion_buffer));
        }

        lines
    }

    pub(super) fn shortcut_actions(&self) -> &'static [(&'static str, &'static str)] {
        if self.editing {
            &[
                ("Enter", "submit suggestion"),
                ("Backspace", "edit"),
                ("Esc", "stop editing"),
            ]
        } else {
            match self.phase {
                ReviewPhase::Drafting if self.has_pending_inspect_prompt() => &[
                    ("Tab/h/l", "change pane"),
                    ("j/k", "scroll"),
                    ("PgUp/PgDn", "page"),
                    ("g/G", "start/end"),
                    ("s", "edit suggestion"),
                    ("a", "accept"),
                    ("c or Esc", "cancel"),
                ],
                ReviewPhase::Drafting => &[
                    ("Tab/h/l", "change pane"),
                    ("j/k", "scroll"),
                    ("PgUp/PgDn", "page"),
                    ("g/G", "start/end"),
                ],
                ReviewPhase::WaitingForModel => &[
                    ("Tab/h/l", "change pane"),
                    ("j/k", "scroll"),
                    ("PgUp/PgDn", "page"),
                    ("g/G", "start/end"),
                ],
                ReviewPhase::PostSuggestionDecision => &[
                    ("Tab/h/l", "change pane"),
                    ("j/k", "scroll"),
                    ("PgUp/PgDn", "page"),
                    ("g/G", "start/end"),
                    ("a", "accept candidate"),
                    ("i", "iterate again"),
                    ("c or Esc", "cancel"),
                ],
            }
        }
    }
}
