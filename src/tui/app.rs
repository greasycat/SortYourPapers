use std::{collections::VecDeque, sync::mpsc, thread};

use crate::error::Result;

use super::{
    backend::BackendEvent,
    forms::RunForm,
    model::{OperationDetail, OperationOutcome, OperationState, Overlay, ProgressEntry, Screen},
    session_view::SessionView,
};

pub(super) struct App {
    pub(super) screen: Screen,
    pub(super) home_index: usize,
    pub(super) run_form: RunForm,
    pub(super) session_view: SessionView,
    pub(super) overlay: Option<Overlay>,
    pub(super) operation: super::model::OperationView,
    pub(super) logs: VecDeque<String>,
    pub(super) progress: Vec<ProgressEntry>,
    pub(super) last_report: Option<crate::report::RunReport>,
    pub(super) last_category_tree: Option<String>,
    pub(super) should_quit: bool,
    pub(super) backend_rx: mpsc::Receiver<BackendEvent>,
    pub(super) op_rx: mpsc::Receiver<OperationOutcome>,
    pub(super) op_tx: mpsc::Sender<OperationOutcome>,
    pub(super) debug_tui: bool,
}

impl App {
    pub(super) fn new(
        backend_rx: mpsc::Receiver<BackendEvent>,
        op_rx: mpsc::Receiver<OperationOutcome>,
        op_tx: mpsc::Sender<OperationOutcome>,
        debug_tui: bool,
    ) -> Result<Self> {
        let mut session_view = SessionView::default();
        session_view.refresh()?;

        Ok(Self {
            screen: Screen::Home,
            home_index: 0,
            run_form: RunForm::default(),
            session_view,
            overlay: None,
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
        })
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
                BackendEvent::Report(report) => {
                    self.last_report = Some(report);
                }
                BackendEvent::CategoryTree(categories) => {
                    self.last_category_tree =
                        Some(crate::terminal::report::render_category_tree(&categories));
                }
                BackendEvent::PromptInspectReview { categories, reply } => {
                    self.last_category_tree =
                        Some(crate::terminal::report::render_category_tree(&categories));
                    self.overlay = Some(Overlay::InspectPrompt {
                        categories,
                        input: String::new(),
                        reply,
                    });
                }
                BackendEvent::PromptContinueImproving { reply } => {
                    self.overlay = Some(Overlay::ContinuePrompt { reply });
                }
            }
        }
    }

    pub(super) fn drain_operation_events(&mut self) {
        while let Ok(outcome) = self.op_rx.try_recv() {
            self.operation.title = outcome.title;
            self.operation.state = if outcome.success {
                OperationState::Success
            } else {
                OperationState::Failure
            };
            self.operation.summary = outcome.summary;
            self.operation.detail = outcome.detail;
            if let OperationDetail::Tree(categories) = &self.operation.detail {
                self.last_category_tree =
                    Some(crate::terminal::report::render_category_tree(categories));
            }
            self.screen = Screen::Operation;
        }
    }

    pub(super) fn apply_edit(&mut self, value: String) -> Result<()> {
        match self.screen {
            Screen::RunForm => self.run_form.apply_edit(value)?,
            Screen::Home | Screen::Sessions | Screen::Operation => {}
        }
        Ok(())
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
        self.screen = Screen::Operation;
        self.overlay = None;
        self.operation = super::model::OperationView {
            title: title.to_string(),
            state: OperationState::Running,
            summary: "running".to_string(),
            detail: OperationDetail::None,
        };
        self.logs.clear();
        self.progress.clear();
        self.last_report = None;
        self.last_category_tree = None;
    }

    fn push_log(&mut self, line: String) {
        self.logs.push_back(line);
        while self.logs.len() > 400 {
            self.logs.pop_front();
        }
    }
}
