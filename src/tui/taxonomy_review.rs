use std::{cell::RefCell, collections::BTreeSet, sync::mpsc};

use crate::{
    papers::taxonomy::CategoryTree,
    terminal::{InspectReviewPrompt, InspectReviewRequest},
};

use super::taxonomy_tree::{TaxonomySection, TaxonomyTreeState, reset_state_for_sections};

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
    History,
    IterationTaxonomy,
}

impl ReviewPane {
    const ALL: [Self; 3] = [Self::Suggestion, Self::History, Self::IterationTaxonomy];

    pub(super) fn index(self) -> usize {
        match self {
            Self::Suggestion => 0,
            Self::History => 1,
            Self::IterationTaxonomy => 2,
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

#[derive(Debug, Clone)]
pub(super) struct CutTaxonomyEntry {
    path: Vec<String>,
    entry: CategoryTree,
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
    pub(super) iteration_tree_state: RefCell<TaxonomyTreeState>,
    pub(super) marked_removals: BTreeSet<Vec<String>>,
    pub(super) marked_subtree_removals: BTreeSet<Vec<String>>,
    pub(super) phase: ReviewPhase,
    pub(super) editing: bool,
    pub(super) pending_reply: Option<PendingReviewReply>,
    pub(super) cut_entry: Option<CutTaxonomyEntry>,
}

impl TaxonomyReviewView {
    pub(super) fn new(
        categories: Vec<CategoryTree>,
        reply: mpsc::Sender<std::result::Result<InspectReviewPrompt, String>>,
    ) -> Self {
        let mut review = Self {
            accepted_categories: categories,
            candidate_categories: None,
            suggestion_buffer: String::new(),
            last_submitted_suggestion: None,
            history: Vec::new(),
            iteration_scroll: 0,
            history_scroll: 0,
            history_selection: 0,
            focused_pane: ReviewPane::Suggestion,
            iteration_tree_state: RefCell::new(TaxonomyTreeState::default()),
            marked_removals: BTreeSet::new(),
            marked_subtree_removals: BTreeSet::new(),
            phase: ReviewPhase::Drafting,
            editing: false,
            pending_reply: Some(PendingReviewReply::Inspect(reply)),
            cut_entry: None,
        };
        review.refresh_iteration_tree_state();
        review
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
        self.marked_removals.clear();
        self.marked_subtree_removals.clear();
        self.focused_pane = ReviewPane::Suggestion;
        self.phase = ReviewPhase::Drafting;
        self.editing = false;
        self.pending_reply = Some(PendingReviewReply::Inspect(reply));
        self.cut_entry = None;
        self.refresh_iteration_tree_state();
    }

    pub(super) fn register_candidate(&mut self, categories: Vec<CategoryTree>) {
        if !matches!(self.phase, ReviewPhase::WaitingForModel) {
            return;
        }

        self.candidate_categories = Some(categories.clone());
        self.iteration_scroll = 0;
        self.marked_removals.clear();
        self.marked_subtree_removals.clear();
        self.cut_entry = None;
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
        self.refresh_iteration_tree_state();
    }

    pub(super) fn set_continue_prompt(
        &mut self,
        reply: mpsc::Sender<std::result::Result<bool, String>>,
    ) {
        self.marked_removals.clear();
        self.marked_subtree_removals.clear();
        self.focused_pane = ReviewPane::IterationTaxonomy;
        self.phase = ReviewPhase::PostSuggestionDecision;
        self.editing = false;
        self.pending_reply = Some(PendingReviewReply::Continue(reply));
        self.cut_entry = None;
        self.refresh_iteration_tree_state();
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

    pub(super) fn has_marked_removals(&self) -> bool {
        !self.marked_removals.is_empty() || !self.marked_subtree_removals.is_empty()
    }

    pub(super) fn submit_review_request(&mut self) -> Option<InspectReviewRequest> {
        let request = self.build_review_request(Some(self.suggestion_buffer.trim().to_string()))?;
        let summary = request.summary();

        if summary.is_empty() {
            return None;
        }

        self.last_submitted_suggestion = Some(summary);
        self.phase = ReviewPhase::WaitingForModel;
        self.focused_pane = ReviewPane::IterationTaxonomy;
        self.iteration_scroll = 0;
        self.editing = false;
        Some(request)
    }

    pub(super) fn promote_candidate_to_accepted(&mut self) {
        if let Some(candidate) = self.candidate_categories.take() {
            self.accepted_categories = candidate;
        }
        self.suggestion_buffer.clear();
        self.iteration_scroll = 0;
        self.history_selection = 0;
        self.history_scroll = 0;
        self.marked_removals.clear();
        self.marked_subtree_removals.clear();
        self.phase = ReviewPhase::Drafting;
        self.focused_pane = ReviewPane::Suggestion;
        self.editing = false;
        self.pending_reply = None;
        self.cut_entry = None;
        self.refresh_iteration_tree_state();
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
                if delta < 0 {
                    for _ in 0..delta.unsigned_abs() {
                        let _ = self.iteration_tree_state.borrow_mut().key_up();
                    }
                } else {
                    for _ in 0..delta as usize {
                        let _ = self.iteration_tree_state.borrow_mut().key_down();
                    }
                }
                self.iteration_tree_state
                    .borrow_mut()
                    .scroll_selected_into_view();
            }
            ReviewPane::History => self.select_history_delta(delta),
        }
    }

    pub(super) fn jump_focused(&mut self, to_end: bool) {
        match self.focused_pane {
            ReviewPane::Suggestion => {}
            ReviewPane::IterationTaxonomy => {
                if to_end {
                    let _ = self.iteration_tree_state.borrow_mut().select_last();
                } else {
                    let _ = self.iteration_tree_state.borrow_mut().select_first();
                }
                self.iteration_tree_state
                    .borrow_mut()
                    .scroll_selected_into_view();
            }
            ReviewPane::History => {
                self.history_selection = if to_end { self.history.len() } else { 0 };
                self.sync_history_scroll();
                self.iteration_scroll = 0;
            }
        }
    }

    pub(super) fn toggle_focused_tree(&mut self) {
        if !matches!(self.focused_pane, ReviewPane::IterationTaxonomy) {
            return;
        }

        if self.iteration_tree_state.borrow_mut().toggle_selected() {
            self.iteration_tree_state
                .borrow_mut()
                .scroll_selected_into_view();
        }
    }

    pub(super) fn toggle_selected_removal(&mut self) {
        if !matches!(self.phase, ReviewPhase::Drafting)
            || !matches!(self.focused_pane, ReviewPane::IterationTaxonomy)
        {
            return;
        }

        let selected = self.iteration_tree_state.borrow().selected().to_vec();
        let Some(path) = self.selected_category_path(&selected) else {
            return;
        };

        self.marked_subtree_removals.remove(&path);
        if !self.marked_removals.remove(&path) {
            self.marked_removals.insert(path);
        }
    }

    pub(super) fn toggle_selected_subtree_removal(&mut self) {
        if !matches!(self.phase, ReviewPhase::Drafting)
            || !matches!(self.focused_pane, ReviewPane::IterationTaxonomy)
        {
            return;
        }

        let selected = self.iteration_tree_state.borrow().selected().to_vec();
        let Some(path) = self.selected_category_path(&selected) else {
            return;
        };

        self.marked_removals.remove(&path);
        if !self.marked_subtree_removals.remove(&path) {
            self.marked_subtree_removals.insert(path);
        }
    }

    pub(super) fn cut_selected_entry(&mut self) -> bool {
        if !self.can_rearrange_taxonomy()
            || !matches!(self.focused_pane, ReviewPane::IterationTaxonomy)
        {
            return false;
        }

        let selected = self.iteration_tree_state.borrow().selected().to_vec();
        let Some(path) = self.selected_category_path(&selected) else {
            return false;
        };
        let parent_path = path[..path.len().saturating_sub(1)].to_vec();
        let Some(entry) = self
            .editable_iteration_categories_mut()
            .and_then(|categories| remove_category_at_path(categories, &path))
        else {
            return false;
        };

        self.cut_entry = Some(CutTaxonomyEntry {
            path: path.clone(),
            entry,
        });
        self.clear_tree_edit_marks();
        self.refresh_iteration_tree_state();
        self.select_iteration_tree_path(&parent_path);
        true
    }

    pub(super) fn paste_cut_entry(&mut self) -> bool {
        if !self.can_rearrange_taxonomy()
            || !matches!(self.focused_pane, ReviewPane::IterationTaxonomy)
        {
            return false;
        }

        let Some(cut_entry) = self.cut_entry.take() else {
            return false;
        };
        let selected = self.iteration_tree_state.borrow().selected().to_vec();
        let destination_path = if selected.len() < 2 {
            Vec::new()
        } else if let Some(path) = self.selected_category_path(&selected) {
            path
        } else {
            self.cut_entry = Some(cut_entry);
            return false;
        };

        let pasted_name = cut_entry.entry.name.clone();
        let entry = cut_entry.entry;
        let Some(categories) = self.editable_iteration_categories_mut() else {
            self.cut_entry = Some(CutTaxonomyEntry {
                path: cut_entry.path,
                entry,
            });
            return false;
        };

        match insert_category_at_path(categories, &destination_path, entry) {
            Ok(()) => {
                self.clear_tree_edit_marks();
                self.refresh_iteration_tree_state();
                let mut pasted_path = destination_path;
                pasted_path.push(pasted_name);
                self.select_iteration_tree_path(&pasted_path);
                true
            }
            Err(entry) => {
                self.cut_entry = Some(CutTaxonomyEntry {
                    path: cut_entry.path,
                    entry,
                });
                false
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
        self.marked_removals.clear();
        self.marked_subtree_removals.clear();
        self.cut_entry = None;
        self.refresh_iteration_tree_state();
    }

    fn can_rearrange_taxonomy(&self) -> bool {
        self.history_selection == 0
            && matches!(
                self.phase,
                ReviewPhase::Drafting | ReviewPhase::PostSuggestionDecision
            )
    }

    fn editable_iteration_categories_mut(&mut self) -> Option<&mut Vec<CategoryTree>> {
        if self.history_selection != 0 {
            return None;
        }

        if let Some(categories) = self.candidate_categories.as_mut()
            && !categories.is_empty()
        {
            return Some(categories);
        }

        Some(&mut self.accepted_categories)
    }

    fn clear_tree_edit_marks(&mut self) {
        self.marked_removals.clear();
        self.marked_subtree_removals.clear();
    }

    fn select_iteration_tree_path(&mut self, path: &[String]) {
        let mut selected = vec![0];
        let mut categories = self.current_iteration_categories();
        for segment in path {
            let Some(index) = categories
                .iter()
                .position(|category| category.name == *segment)
            else {
                break;
            };
            selected.push(index);
            categories = &categories[index].children;
        }

        let state = self.iteration_tree_state.get_mut();
        let _ = state.select(selected);
        state.scroll_selected_into_view();
    }

    fn build_review_request(&self, suggestion: Option<String>) -> Option<InspectReviewRequest> {
        let user_suggestion = suggestion
            .map(|suggestion| suggestion.trim().to_string())
            .filter(|suggestion| !suggestion.is_empty());
        let removals = self
            .marked_removals
            .iter()
            .chain(self.marked_subtree_removals.iter())
            .map(|path| path.join(" > "))
            .collect::<Vec<_>>();

        if user_suggestion.is_none() && removals.is_empty() {
            return None;
        }

        Some(InspectReviewRequest::new(user_suggestion, removals))
    }

    fn current_iteration_categories(&self) -> &[CategoryTree] {
        if let Some(iteration) = self.selected_iteration() {
            if iteration.suggested_categories.is_empty() {
                &iteration.accepted_categories
            } else {
                &iteration.suggested_categories
            }
        } else if let Some(categories) = self
            .candidate_categories
            .as_ref()
            .filter(|categories| !categories.is_empty())
        {
            categories
        } else {
            &self.accepted_categories
        }
    }

    fn marked_paths_for_render(&self, categories: &[CategoryTree]) -> BTreeSet<Vec<String>> {
        let mut marked_paths = self.marked_removals.clone();
        for root in &self.marked_subtree_removals {
            collect_marked_subtree_paths(categories, root, &mut marked_paths);
        }
        marked_paths
    }

    fn selected_category_path(&self, selected: &[usize]) -> Option<Vec<String>> {
        if selected.len() < 2 {
            return None;
        }

        let mut categories = self.current_iteration_categories();
        let mut path = Vec::new();
        for index in &selected[1..] {
            let category = categories.get(*index)?;
            path.push(category.name.clone());
            categories = &category.children;
        }
        Some(path)
    }

    pub(super) fn iteration_summary_lines(&self) -> Vec<String> {
        if let Some(iteration) = self.selected_iteration() {
            return vec![
                format!("Iteration {}", iteration.number),
                format!("Suggestion: {}", iteration.suggestion),
            ];
        }

        let mut lines = vec!["Current Iteration".to_string()];
        if let Some(suggestion) = self.last_submitted_suggestion.as_deref()
            && !matches!(self.phase, ReviewPhase::Drafting)
        {
            lines.push(format!("Latest suggestion: {suggestion}"));
        }
        lines
    }

    pub(super) fn iteration_taxonomy_sections(&self) -> Vec<TaxonomySection> {
        if let Some(iteration) = self.selected_iteration() {
            return vec![if iteration.suggested_categories.is_empty() {
                TaxonomySection::Categories {
                    title: "Accepted Taxonomy".to_string(),
                    categories: iteration.accepted_categories.clone(),
                    marked_paths: BTreeSet::new(),
                }
            } else {
                TaxonomySection::Categories {
                    title: "Suggested Taxonomy".to_string(),
                    categories: iteration.suggested_categories.clone(),
                    marked_paths: BTreeSet::new(),
                }
            }];
        }

        vec![match &self.candidate_categories {
            Some(categories) if !categories.is_empty() => TaxonomySection::Categories {
                title: "Suggested Taxonomy".to_string(),
                categories: categories.clone(),
                marked_paths: self.marked_paths_for_render(categories),
            },
            _ => TaxonomySection::Categories {
                title: "Accepted Taxonomy".to_string(),
                categories: self.accepted_categories.clone(),
                marked_paths: self.marked_paths_for_render(&self.accepted_categories),
            },
        }]
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

    fn refresh_iteration_tree_state(&mut self) {
        let sections = self.iteration_taxonomy_sections();
        let state = self.iteration_tree_state.get_mut();
        *state = TaxonomyTreeState::default();
        reset_state_for_sections(state, &sections);
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
                lines.push("Press `d` on a taxonomy node to mark it for removal.".to_string());
                lines.push("Press `D` to mark that taxonomy and all sub-taxonomies.".to_string());
                lines.push("Press `x` to cut a taxonomy entry and `p` to paste it.".to_string());
                lines
                    .push("Accept with `a`, or press `s` to draft the next iteration.".to_string());
            }
            ReviewPhase::Drafting => {
                lines.push("Preparing the next iteration request.".to_string());
                lines.push("The current taxonomy will become the next baseline.".to_string());
                lines.push("You can still cut with `x` and paste with `p`.".to_string());
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
                lines.push(
                    "Use `x` and `p` to reorganize taxonomy entries before deciding.".to_string(),
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
                lines.push(
                    "Use `d` on a taxonomy node to request removing that section.".to_string(),
                );
                lines
                    .push("Use `D` to request removing the selected taxonomy subtree.".to_string());
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

        if let Some(cut_entry) = self.cut_entry.as_ref() {
            lines.push(String::new());
            lines.push(format!("cut entry: {}", cut_entry.path.join(" > ")));
            lines.push("Select a taxonomy entry and press `p` to paste under it.".to_string());
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
                    ("Space", "fold"),
                    ("d", "mark remove"),
                    ("D", "mark subtree"),
                    ("x", "cut entry"),
                    ("p", "paste entry"),
                    ("PgUp/PgDn", "page"),
                    ("g/G", "start/end"),
                    ("s", "edit suggestion"),
                    ("a", "accept"),
                    ("c or Esc", "cancel"),
                ],
                ReviewPhase::Drafting => &[
                    ("Tab/h/l", "change pane"),
                    ("j/k", "scroll"),
                    ("Space", "fold"),
                    ("d", "mark remove"),
                    ("D", "mark subtree"),
                    ("x", "cut entry"),
                    ("p", "paste entry"),
                    ("PgUp/PgDn", "page"),
                    ("g/G", "start/end"),
                ],
                ReviewPhase::WaitingForModel => &[
                    ("Tab/h/l", "change pane"),
                    ("j/k", "scroll"),
                    ("Space", "fold"),
                    ("PgUp/PgDn", "page"),
                    ("g/G", "start/end"),
                ],
                ReviewPhase::PostSuggestionDecision => &[
                    ("Tab/h/l", "change pane"),
                    ("j/k", "scroll"),
                    ("Space", "fold"),
                    ("x", "cut entry"),
                    ("p", "paste entry"),
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

fn collect_marked_subtree_paths(
    categories: &[CategoryTree],
    root: &[String],
    marked_paths: &mut BTreeSet<Vec<String>>,
) {
    if root.is_empty() {
        return;
    }

    let mut current_categories = categories;
    let mut path = Vec::new();
    let mut target = None;
    for segment in root {
        let Some(category) = current_categories
            .iter()
            .find(|category| category.name == *segment)
        else {
            return;
        };
        path.push(category.name.clone());
        current_categories = &category.children;
        target = Some(category);
    }

    if let Some(category) = target {
        collect_category_subtree_paths(category, &mut path, marked_paths);
    }
}

fn collect_category_subtree_paths(
    category: &CategoryTree,
    path: &mut Vec<String>,
    marked_paths: &mut BTreeSet<Vec<String>>,
) {
    marked_paths.insert(path.clone());
    for child in &category.children {
        path.push(child.name.clone());
        collect_category_subtree_paths(child, path, marked_paths);
        path.pop();
    }
}

fn remove_category_at_path(
    categories: &mut Vec<CategoryTree>,
    path: &[String],
) -> Option<CategoryTree> {
    let Some((head, tail)) = path.split_first() else {
        return None;
    };
    let index = categories
        .iter()
        .position(|category| category.name == *head)?;

    if tail.is_empty() {
        Some(categories.remove(index))
    } else {
        remove_category_at_path(&mut categories[index].children, tail)
    }
}

fn insert_category_at_path(
    categories: &mut Vec<CategoryTree>,
    parent_path: &[String],
    entry: CategoryTree,
) -> std::result::Result<(), CategoryTree> {
    let Some((head, tail)) = parent_path.split_first() else {
        categories.push(entry);
        return Ok(());
    };
    let Some(index) = categories
        .iter()
        .position(|category| category.name == *head)
    else {
        return Err(entry);
    };

    if tail.is_empty() {
        categories[index].children.push(entry);
        Ok(())
    } else {
        insert_category_at_path(&mut categories[index].children, tail, entry)
    }
}
