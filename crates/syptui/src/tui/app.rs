use std::{collections::VecDeque, sync::mpsc, thread, time::Instant};

use crate::{CliArgs, config, error::Result, prefs};

use super::{
    backend::BackendEvent,
    forms::{ExtractForm, RunForm},
    model::{
        HomeAction, OperationAlert, OperationDetail, OperationOutcome, OperationState,
        OperationTab, Overlay, ProgressEntry, Screen, StageTiming,
    },
    session_view::SessionView,
    taxonomy_review::TaxonomyReviewView,
    taxonomy_tree::reset_state_for_categories,
    theme::{ThemePalette, UiThemeName},
};

const MAX_LOG_LINES: usize = 400;
const MAX_PINNED_ALERTS: usize = 20;
const LOG_FOLLOW_WINDOW: usize = 18;

pub(super) struct App {
    pub(super) screen: Screen,
    pub(super) home_index: usize,
    pub(super) run_form: RunForm,
    pub(super) extract_form: ExtractForm,
    pub(super) session_view: SessionView,
    pub(super) overlay: Option<Overlay>,
    pub(super) taxonomy_review: Option<TaxonomyReviewView>,
    pub(super) operation: super::model::OperationView,
    pub(super) logs: VecDeque<String>,
    pub(super) progress: Vec<ProgressEntry>,
    pub(super) last_report: Option<crate::report::RunReport>,
    pub(super) last_category_tree: Option<Vec<crate::papers::taxonomy::CategoryTree>>,
    pub(super) should_quit: bool,
    pub(super) backend_rx: mpsc::Receiver<BackendEvent>,
    pub(super) op_rx: mpsc::Receiver<OperationOutcome>,
    pub(super) op_tx: mpsc::Sender<OperationOutcome>,
    pub(super) debug_tui: bool,
    pub(super) theme_name: UiThemeName,
    pub(super) theme: ThemePalette,
}

impl App {
    pub(super) fn new(
        backend_rx: mpsc::Receiver<BackendEvent>,
        op_rx: mpsc::Receiver<OperationOutcome>,
        op_tx: mpsc::Sender<OperationOutcome>,
        debug_tui: bool,
        theme_name: UiThemeName,
    ) -> Result<Self> {
        let mut session_view = SessionView::default();
        session_view.refresh()?;
        let run_form = RunForm::from_config(&config::resolve_config(CliArgs {
            input: None,
            output: None,
            recursive: None,
            max_file_size_mb: None,
            page_cutoff: None,
            pdf_extract_workers: None,
            category_depth: None,
            taxonomy_mode: None,
            taxonomy_assistance: None,
            taxonomy_batch_size: None,
            reference_manifest_path: None,
            reference_top_k: None,
            use_current_folder_tree: None,
            placement_batch_size: None,
            placement_assistance: None,
            placement_mode: None,
            placement_reference_top_k: None,
            placement_candidate_top_k: None,
            placement_min_similarity: None,
            placement_min_margin: None,
            placement_min_reference_support: None,
            rebuild: None,
            apply: false,
            llm_provider: None,
            llm_model: None,
            llm_base_url: None,
            api_key: None,
            api_key_command: None,
            api_key_env: None,
            embedding_provider: None,
            embedding_model: None,
            embedding_base_url: None,
            embedding_api_key: None,
            embedding_api_key_command: None,
            embedding_api_key_env: None,
            keyword_batch_size: None,
            subcategories_suggestion_number: None,
            verbosity: 0,
            quiet: false,
        })?);

        Ok(Self {
            screen: Screen::Home,
            home_index: 0,
            run_form,
            extract_form: ExtractForm::default(),
            session_view,
            overlay: None,
            taxonomy_review: None,
            operation: super::model::OperationView::default(),
            logs: VecDeque::new(),
            progress: Vec::new(),
            last_report: None,
            last_category_tree: None,
            should_quit: false,
            backend_rx,
            op_rx,
            op_tx,
            debug_tui,
            theme_name,
            theme: theme_name.palette(),
        })
    }

    pub(super) fn cycle_theme(&mut self) {
        self.theme_name = self.theme_name.next();
        self.theme = self.theme_name.palette();

        if let Err(err) = prefs::save_tui_preferences(&prefs::TuiPreferences {
            theme: self.theme_name,
        }) {
            self.overlay = Some(Overlay::Notice {
                title: "Theme Persistence".to_string(),
                message: err.to_string(),
            });
        }
    }

    pub(super) fn drain_backend_events(&mut self) {
        while let Ok(event) = self.backend_rx.try_recv() {
            match event {
                BackendEvent::StdoutLine(line) | BackendEvent::StderrLine(line) => {
                    self.push_log(line);
                }
                BackendEvent::ProgressStart { id, total, label } => {
                    self.progress.push(ProgressEntry {
                        id,
                        label,
                        total,
                        current: 0,
                    });
                }
                BackendEvent::ProgressAdvance { id, delta } => {
                    if let Some(progress) = self.progress.iter_mut().find(|entry| entry.id == id) {
                        progress.current =
                            progress.current.saturating_add(delta).min(progress.total);
                    }
                }
                BackendEvent::ProgressFinish { id } => {
                    self.progress.retain(|entry| entry.id != id);
                }
                BackendEvent::StageStatus { stage, message } => {
                    self.record_stage_status(stage, message);
                }
                BackendEvent::Alert {
                    severity,
                    label,
                    message,
                } => {
                    self.operation
                        .alerts
                        .push_back(OperationAlert::new(severity, label, message));
                    while self.operation.alerts.len() > MAX_PINNED_ALERTS {
                        self.operation.alerts.pop_front();
                    }
                }
                BackendEvent::Report(report) => {
                    self.last_report = Some(report);
                }
                BackendEvent::CategoryTree(categories) => {
                    self.last_category_tree = Some(categories.clone());
                    if let Some(review) = self.taxonomy_review.as_mut() {
                        review.register_candidate(categories);
                    }
                }
                BackendEvent::PromptInspectReview { categories, reply } => {
                    self.last_category_tree = Some(categories.clone());
                    self.screen = Screen::TaxonomyReview;
                    if let Some(review) = self.taxonomy_review.as_mut() {
                        review.begin_iteration(categories, reply);
                    } else {
                        self.taxonomy_review = Some(TaxonomyReviewView::new(categories, reply));
                    }
                }
                BackendEvent::PromptContinueImproving { reply } => {
                    self.screen = Screen::TaxonomyReview;
                    if let Some(review) = self.taxonomy_review.as_mut() {
                        review.set_continue_prompt(reply);
                    }
                }
            }
        }
    }

    pub(super) fn drain_operation_events(&mut self) {
        while let Ok(outcome) = self.op_rx.try_recv() {
            self.finish_current_stage_timing();
            let origin = self.operation.origin;
            self.taxonomy_review = None;
            self.operation.title = outcome.title;
            self.operation.state = if outcome.success {
                OperationState::Success
            } else {
                OperationState::Failure
            };
            self.operation.summary = outcome.summary;
            self.operation.detail = outcome.detail;
            if let OperationDetail::Tree(categories) = &self.operation.detail {
                self.last_category_tree = Some(categories.clone());
                reset_state_for_categories(
                    &mut self.operation.taxonomy_tree_state.borrow_mut(),
                    categories,
                );
            }
            if matches!(origin, Screen::Sessions) {
                let _ = self.session_view.refresh();
            }
            self.screen = Screen::Operation;
        }
    }

    pub(super) fn apply_edit(&mut self, value: String) -> Result<()> {
        match self.screen {
            Screen::RunForm => self.run_form.apply_edit(value)?,
            Screen::ExtractForm => self.extract_form.apply_edit(value)?,
            Screen::Home | Screen::Sessions | Screen::Operation | Screen::TaxonomyReview => {}
        }
        Ok(())
    }

    pub(super) fn switch_operation_tab(&mut self, delta: i8) {
        let current = self.operation.active_tab.index();
        let target = if delta < 0 {
            current.saturating_sub(1)
        } else {
            (current + 1).min(OperationTab::ALL.len() - 1)
        };
        self.operation.active_tab = OperationTab::from_index(target);
    }

    pub(super) fn set_operation_tab(&mut self, tab: OperationTab) {
        self.operation.active_tab = tab;
    }

    pub(super) fn scroll_active_operation_tab(&mut self, delta: isize) {
        let active_tab = self.operation.active_tab;
        if matches!(active_tab, OperationTab::Taxonomy)
            && matches!(
                self.operation.detail,
                OperationDetail::Tree(_) | OperationDetail::None
            )
        {
            if delta < 0 {
                for _ in 0..delta.unsigned_abs() {
                    let _ = self.operation.taxonomy_tree_state.borrow_mut().key_up();
                }
            } else {
                for _ in 0..delta as usize {
                    let _ = self.operation.taxonomy_tree_state.borrow_mut().key_down();
                }
            }
            self.operation
                .taxonomy_tree_state
                .borrow_mut()
                .scroll_selected_into_view();
            return;
        }

        let next = match active_tab {
            OperationTab::Summary => return,
            OperationTab::Logs => self.operation.log_scroll as isize + delta,
            OperationTab::Taxonomy => self.operation.taxonomy_scroll as isize + delta,
            OperationTab::Report => self.operation.report_scroll as isize + delta,
        }
        .clamp(0, u16::MAX as isize) as u16;

        match active_tab {
            OperationTab::Summary => {}
            OperationTab::Logs => self.operation.log_scroll = next,
            OperationTab::Taxonomy => self.operation.taxonomy_scroll = next,
            OperationTab::Report => self.operation.report_scroll = next,
        }
    }

    pub(super) fn jump_active_operation_tab(&mut self, to_end: bool) {
        let active_tab = self.operation.active_tab;
        if matches!(active_tab, OperationTab::Taxonomy)
            && matches!(
                self.operation.detail,
                OperationDetail::Tree(_) | OperationDetail::None
            )
        {
            if to_end {
                let _ = self
                    .operation
                    .taxonomy_tree_state
                    .borrow_mut()
                    .select_last();
            } else {
                let _ = self
                    .operation
                    .taxonomy_tree_state
                    .borrow_mut()
                    .select_first();
            }
            self.operation
                .taxonomy_tree_state
                .borrow_mut()
                .scroll_selected_into_view();
            return;
        }
        let target = if to_end { u16::MAX as usize } else { 0 }.min(u16::MAX as usize) as u16;

        match active_tab {
            OperationTab::Summary => {}
            OperationTab::Logs => self.operation.log_scroll = target,
            OperationTab::Taxonomy => self.operation.taxonomy_scroll = target,
            OperationTab::Report => self.operation.report_scroll = target,
        }
    }

    pub(super) fn toggle_active_operation_taxonomy(&mut self) {
        if !matches!(self.operation.active_tab, OperationTab::Taxonomy) {
            return;
        }
        if !matches!(
            self.operation.detail,
            OperationDetail::Tree(_) | OperationDetail::None
        ) {
            return;
        }

        if self
            .operation
            .taxonomy_tree_state
            .borrow_mut()
            .toggle_selected()
        {
            self.operation
                .taxonomy_tree_state
                .borrow_mut()
                .scroll_selected_into_view();
        }
    }

    pub(super) fn operation_log_lines(&self) -> Vec<String> {
        self.logs.iter().cloned().collect()
    }

    pub(super) fn operation_report_lines(&self) -> Vec<String> {
        self.last_report
            .as_ref()
            .map(|report| {
                crate::terminal::report::render_report_action_lines(
                    report,
                    crate::terminal::Verbosity::new(false, false, false),
                )
            })
            .unwrap_or_default()
    }

    pub(super) fn operation_report_summary_lines(&self) -> Vec<String> {
        self.last_report
            .as_ref()
            .map(|report| {
                let mut lines = crate::terminal::report::render_report_summary_lines(
                    report,
                    crate::terminal::Verbosity::new(false, false, false),
                );
                if !lines.is_empty() {
                    lines.remove(0);
                }
                lines
            })
            .unwrap_or_default()
    }

    pub(super) fn operation_taxonomy_lines(&self) -> Vec<String> {
        match &self.operation.detail {
            OperationDetail::Tree(_) => Vec::new(),
            OperationDetail::Text { lines, .. } => lines.clone(),
            OperationDetail::None => self
                .last_category_tree
                .as_ref()
                .map(|_| Vec::new())
                .unwrap_or_default(),
        }
    }

    pub(super) fn start_async_operation<Fut, F>(&mut self, title: &str, build: F)
    where
        Fut: std::future::Future<Output = ()> + 'static,
        F: FnOnce(mpsc::Sender<OperationOutcome>) -> Fut + Send + 'static,
    {
        self.prepare_operation(title);
        let tx = self.op_tx.clone();
        thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tui worker runtime should build");
            runtime.block_on(build(tx));
        });
    }

    pub(super) fn start_blocking_operation<F>(&mut self, title: &str, work: F)
    where
        F: FnOnce() -> Result<OperationOutcome> + Send + 'static,
    {
        self.prepare_operation(title);
        let tx = self.op_tx.clone();
        let title = title.to_string();
        thread::spawn(move || {
            let outcome = match work() {
                Ok(outcome) => outcome,
                Err(err) => {
                    OperationOutcome::failure(&title, err.to_string(), OperationDetail::None)
                }
            };
            let _ = tx.send(outcome);
        });
    }

    fn prepare_operation(&mut self, title: &str) {
        let origin = self.screen;
        self.screen = Screen::Operation;
        self.overlay = None;
        self.taxonomy_review = None;
        self.operation = super::model::OperationView {
            title: title.to_string(),
            state: OperationState::Running,
            summary: "running".to_string(),
            detail: OperationDetail::None,
            active_tab: OperationTab::Summary,
            log_scroll: 0,
            taxonomy_scroll: 0,
            taxonomy_tree_state: std::cell::RefCell::new(
                super::taxonomy_tree::TaxonomyTreeState::default(),
            ),
            report_scroll: 0,
            alerts: VecDeque::new(),
            stage_label: String::new(),
            stage_message: String::new(),
            stage_started_at: None,
            stage_timings: Vec::new(),
            origin,
        };
        self.logs.clear();
        self.progress.clear();
        self.last_report = None;
        self.last_category_tree = None;
    }

    fn push_log(&mut self, line: String) {
        let previous_bottom = self.logs.len().saturating_sub(LOG_FOLLOW_WINDOW);
        self.logs.push_back(line);
        while self.logs.len() > MAX_LOG_LINES {
            self.logs.pop_front();
        }
        if matches!(self.operation.active_tab, OperationTab::Logs)
            && self.operation.log_scroll as usize >= previous_bottom.saturating_sub(1)
        {
            self.operation.log_scroll = self
                .logs
                .len()
                .saturating_sub(LOG_FOLLOW_WINDOW)
                .min(u16::MAX as usize) as u16;
        }
    }

    pub(super) fn finish_taxonomy_review(&mut self) {
        self.taxonomy_review = None;
        self.screen = Screen::Operation;
    }

    fn record_stage_status(&mut self, stage: String, message: String) {
        if self.operation.stage_label != stage {
            self.finish_current_stage_timing();
            self.operation.stage_started_at = Some(Instant::now());
            self.operation.stage_label = stage;
        } else if self.operation.stage_started_at.is_none() {
            self.operation.stage_started_at = Some(Instant::now());
        }

        self.operation.stage_message = message;
    }

    fn finish_current_stage_timing(&mut self) {
        let Some(started_at) = self.operation.stage_started_at.take() else {
            return;
        };
        if self.operation.stage_label.is_empty() {
            return;
        }

        self.operation.stage_timings.push(StageTiming {
            stage: self.operation.stage_label.clone(),
            elapsed: started_at.elapsed(),
        });
    }

    pub(super) fn home_actions(&self) -> Vec<HomeAction> {
        let mut actions = vec![
            HomeAction::RunPapers,
            HomeAction::ExtractText,
            HomeAction::Sessions,
        ];
        actions.push(HomeAction::Quit);
        actions
    }

    pub(super) fn selected_home_action(&self) -> HomeAction {
        let actions = self.home_actions();
        actions
            .get(self.home_index.min(actions.len().saturating_sub(1)))
            .copied()
            .unwrap_or(HomeAction::RunPapers)
    }

    pub(super) fn clamp_home_index(&mut self) {
        let max_index = self.home_actions().len().saturating_sub(1);
        self.home_index = self.home_index.min(max_index);
    }
}
