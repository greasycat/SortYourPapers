mod backend;

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Color, CrosstermBackend, Frame, Line, Modifier, Span, Style, Text},
    widgets::{Block, Borders, Clear, Gauge, Paragraph, Wrap},
};

use crate::{
    CliArgs, ExtractTextArgs,
    app_run::stages::stage_sequence,
    cli::{
        DEFAULT_CATEGORY_DEPTH, DEFAULT_INPUT, DEFAULT_KEYWORD_BATCH_SIZE, DEFAULT_LLM_MODEL,
        DEFAULT_LLM_PROVIDER, DEFAULT_MAX_FILE_SIZE_MB, DEFAULT_OUTPUT, DEFAULT_PAGE_CUTOFF,
        DEFAULT_PDF_EXTRACT_WORKERS, DEFAULT_PLACEMENT_BATCH_SIZE,
        DEFAULT_SUBCATEGORIES_SUGGESTION_NUMBER, DEFAULT_TAXONOMY_BATCH_SIZE,
    },
    config,
    domain::{AppConfig, CategoryTree, LlmProvider, PlacementMode, TaxonomyMode},
    error::{AppError, Result},
    papers::extract::ExtractorMode,
    pdf_extract::{extract_text_batch, reset_debug_extract_log},
    run_state::{RunStage, RunSummary, RunWorkspace},
    session::workspace::RunStage as WorkspaceRunStage,
    terminal::{self, InspectReviewPrompt, install_backend},
};

use self::backend::{BackendEvent, TuiBackend};

pub async fn run(debug_tui: bool) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let (backend_tx, backend_rx) = mpsc::channel();
    let (op_tx, op_rx) = mpsc::channel();
    let _backend_guard = install_backend(Arc::new(TuiBackend::new(backend_tx)));

    let mut app = App::new(backend_rx, op_rx, op_tx, debug_tui)?;
    let run_result = run_loop(&mut terminal, &mut app).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    run_result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        app.drain_backend_events();
        app.drain_operation_events();
        terminal.draw(|frame| app.draw(frame))?;

        if app.should_quit {
            return Ok(());
        }

        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.handle_key(key).await?;
        }
    }
}

struct App {
    screen: Screen,
    home_index: usize,
    run_form: RunForm,
    extract_form: ExtractForm,
    init_force: bool,
    session_view: SessionView,
    overlay: Option<Overlay>,
    operation: OperationView,
    logs: VecDeque<String>,
    progress: Vec<ProgressEntry>,
    last_report: Option<crate::models::RunReport>,
    last_category_tree: Option<String>,
    should_quit: bool,
    backend_rx: mpsc::Receiver<BackendEvent>,
    op_rx: mpsc::Receiver<OperationOutcome>,
    op_tx: mpsc::Sender<OperationOutcome>,
    debug_tui: bool,
}

impl App {
    fn new(
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
            extract_form: ExtractForm::default(),
            init_force: false,
            session_view,
            overlay: None,
            operation: OperationView::default(),
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

    fn drain_backend_events(&mut self) {
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

    fn drain_operation_events(&mut self) {
        while let Ok(outcome) = self.op_rx.try_recv() {
            self.operation.title = outcome.title;
            self.operation.running = false;
            self.operation.summary = outcome.summary;
            self.operation.success = outcome.success;
            self.operation.detail = outcome.detail;
            if let OperationDetail::Tree(categories) = &self.operation.detail {
                self.last_category_tree =
                    Some(crate::terminal::report::render_category_tree(categories));
            }
            self.screen = Screen::Operation;
            if matches!(self.operation.detail, OperationDetail::None)
                && !matches!(self.screen, Screen::Operation)
            {
                self.screen = Screen::Operation;
            }
        }
    }

    async fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        if self.handle_overlay_key(key).await? {
            return Ok(());
        }

        match self.screen {
            Screen::Home => self.handle_home_key(key).await,
            Screen::RunForm => self.handle_run_form_key(key).await,
            Screen::Sessions => self.handle_sessions_key(key).await,
            Screen::ExtractForm => self.handle_extract_form_key(key).await,
            Screen::Init => self.handle_init_key(key).await,
            Screen::Operation => self.handle_operation_key(key),
        }
    }

    async fn handle_home_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Down | KeyCode::Char('j') => {
                self.home_index = (self.home_index + 1).min(HOME_ITEMS.len() - 1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.home_index = self.home_index.saturating_sub(1);
            }
            KeyCode::Enter => {
                self.screen = match self.home_index {
                    0 => Screen::RunForm,
                    1 => {
                        self.session_view.refresh()?;
                        Screen::Sessions
                    }
                    2 => Screen::ExtractForm,
                    3 => Screen::Init,
                    _ => {
                        self.should_quit = true;
                        Screen::Home
                    }
                };
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_sessions_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => self.screen = Screen::Home,
            KeyCode::Char('g') => self.session_view.refresh()?,
            KeyCode::Down | KeyCode::Char('j') => self.session_view.move_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.session_view.move_selection(-1),
            KeyCode::Char('p') => {
                if let Some(run_id) = self.session_view.selected_run_id() {
                    self.start_async_operation("Resume Session", move |tx| async move {
                        match crate::resume_run(Some(run_id.clone()), false, 0, false).await {
                            Ok(_) => tx.send(OperationOutcome::success(
                                "Resume Session",
                                format!("resumed {run_id} in preview mode"),
                                OperationDetail::None,
                            )),
                            Err(err) => tx.send(OperationOutcome::failure(
                                "Resume Session",
                                err.to_string(),
                                OperationDetail::None,
                            )),
                        }
                        .ok();
                    });
                }
            }
            KeyCode::Char('a') => {
                if let Some(run_id) = self.session_view.selected_run_id() {
                    self.start_async_operation("Resume Session", move |tx| async move {
                        match crate::resume_run(Some(run_id.clone()), true, 0, false).await {
                            Ok(_) => tx.send(OperationOutcome::success(
                                "Resume Session",
                                format!("resumed {run_id} in apply mode"),
                                OperationDetail::None,
                            )),
                            Err(err) => tx.send(OperationOutcome::failure(
                                "Resume Session",
                                err.to_string(),
                                OperationDetail::None,
                            )),
                        }
                        .ok();
                    });
                }
            }
            KeyCode::Char('r') => self.open_rerun_overlay(false)?,
            KeyCode::Char('x') => self.open_rerun_overlay(true)?,
            KeyCode::Char('v') => {
                if let Some(run_id) = self.session_view.selected_run_id() {
                    self.start_blocking_operation("Review Session", move || {
                        let workspace = RunWorkspace::open(&run_id)?;
                        if workspace.last_completed_stage() != Some(WorkspaceRunStage::Completed) {
                            return Err(AppError::Execution(format!(
                                "run '{run_id}' is not completed"
                            )));
                        }
                        let categories = workspace
                            .load_stage::<crate::models::SynthesizeCategoriesState>(
                                WorkspaceRunStage::SynthesizeCategories,
                            )?
                            .ok_or_else(|| {
                                AppError::Execution(format!(
                                    "run '{run_id}' has no saved synthesized categories"
                                ))
                            })?;
                        Ok(OperationOutcome::success(
                            "Review Session",
                            format!("loaded taxonomy for {run_id}"),
                            OperationDetail::Tree(categories.categories),
                        ))
                    });
                }
            }
            KeyCode::Char('d') => {
                if let Some(run_id) = self.session_view.selected_run_id() {
                    self.overlay = Some(Overlay::Confirm {
                        title: "Remove Session".to_string(),
                        message: format!("Remove saved session {run_id}?"),
                        action: ConfirmAction::RemoveRun(run_id),
                    });
                }
            }
            KeyCode::Char('c') => {
                self.overlay = Some(Overlay::Confirm {
                    title: "Clear Incomplete Sessions".to_string(),
                    message: "Clear all incomplete saved sessions for this workspace?".to_string(),
                    action: ConfirmAction::ClearIncomplete,
                });
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_run_form_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => self.screen = Screen::Home,
            KeyCode::Down | KeyCode::Char('j') => {
                self.run_form.selected =
                    (self.run_form.selected + 1).min(RUN_FIELD_LABELS.len() - 1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.run_form.selected = self.run_form.selected.saturating_sub(1);
            }
            KeyCode::Left => self.run_form.cycle_selected(-1),
            KeyCode::Right => self.run_form.cycle_selected(1),
            KeyCode::Char(' ') => self.run_form.toggle_selected(),
            KeyCode::Char('r') => {
                let config = self.run_form.build_config()?;
                let use_debug_tui = self.debug_tui;
                let op_tx = self.op_tx.clone();
                self.start_async_operation("Run Papers", move |_tx| async move {
                    let outcome = match if use_debug_tui {
                        crate::app::run_debug_tui(config).await
                    } else {
                        crate::run(config).await
                    } {
                        Ok(_) => OperationOutcome::success(
                            "Run Papers",
                            "run completed".to_string(),
                            OperationDetail::None,
                        ),
                        Err(err) => OperationOutcome::failure(
                            "Run Papers",
                            err.to_string(),
                            OperationDetail::None,
                        ),
                    };
                    let _ = op_tx.send(outcome);
                });
            }
            KeyCode::Enter => {
                if self.run_form.editable(self.run_form.selected) {
                    self.overlay = Some(Overlay::EditField {
                        label: RUN_FIELD_LABELS[self.run_form.selected].to_string(),
                        buffer: self.run_form.value(self.run_form.selected),
                    });
                } else {
                    self.run_form.toggle_selected();
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_extract_form_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => self.screen = Screen::Home,
            KeyCode::Down | KeyCode::Char('j') => {
                self.extract_form.selected =
                    (self.extract_form.selected + 1).min(EXTRACT_FIELD_LABELS.len() - 1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.extract_form.selected = self.extract_form.selected.saturating_sub(1);
            }
            KeyCode::Left => self.extract_form.cycle_selected(-1),
            KeyCode::Right => self.extract_form.cycle_selected(1),
            KeyCode::Char('r') => {
                let args = self.extract_form.build_args()?;
                let op_tx = self.op_tx.clone();
                self.start_async_operation("Extract Text", move |_tx| async move {
                    let outcome = match collect_extract_preview(args).await {
                        Ok(result) => OperationOutcome::success(
                            "Extract Text",
                            format!(
                                "processed {} file(s), {} failure(s)",
                                result.papers.len(),
                                result.failures.len()
                            ),
                            OperationDetail::Text(render_extract_result_lines(&result)),
                        ),
                        Err(err) => OperationOutcome::failure(
                            "Extract Text",
                            err.to_string(),
                            OperationDetail::None,
                        ),
                    };
                    let _ = op_tx.send(outcome);
                });
            }
            KeyCode::Enter => {
                if self.extract_form.selected == 2 || self.extract_form.selected == 4 {
                    self.extract_form.cycle_selected(1);
                } else {
                    self.overlay = Some(Overlay::EditField {
                        label: EXTRACT_FIELD_LABELS[self.extract_form.selected].to_string(),
                        buffer: self.extract_form.value(self.extract_form.selected),
                    });
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_init_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => self.screen = Screen::Home,
            KeyCode::Char(' ') => self.init_force = !self.init_force,
            KeyCode::Enter | KeyCode::Char('r') => {
                let force = self.init_force;
                self.start_blocking_operation("Init Config", move || {
                    let path = crate::init_config(force)?;
                    Ok(OperationOutcome::success(
                        "Init Config",
                        format!("wrote config to {}", path.display()),
                        OperationDetail::None,
                    ))
                });
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_operation_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('q') => {
                if !self.operation.running {
                    self.should_quit = true;
                }
            }
            KeyCode::Esc | KeyCode::Char('b') => {
                if !self.operation.running {
                    self.screen = Screen::Home;
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_overlay_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(mut overlay) = self.overlay.take() else {
            return Ok(false);
        };

        let handled = match &mut overlay {
            Overlay::EditField { buffer, .. } => {
                match key.code {
                    KeyCode::Esc => {}
                    KeyCode::Enter => {
                        self.apply_edit(buffer.clone())?;
                        return Ok(true);
                    }
                    KeyCode::Backspace => {
                        buffer.pop();
                        self.overlay = Some(overlay);
                        return Ok(true);
                    }
                    KeyCode::Char(c) => {
                        if !key.modifiers.contains(KeyModifiers::CONTROL) {
                            buffer.push(c);
                            self.overlay = Some(overlay);
                        }
                        return Ok(true);
                    }
                    _ => {
                        self.overlay = Some(overlay);
                        return Ok(true);
                    }
                }
                false
            }
            Overlay::InspectPrompt { input, reply, .. } => {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => {
                        let _ = reply.send(Err("inspect-output cancelled".to_string()));
                    }
                    KeyCode::Enter => {
                        let response = if input.trim().is_empty() {
                            InspectReviewPrompt::Accept
                        } else {
                            InspectReviewPrompt::Suggest(input.trim().to_string())
                        };
                        let _ = reply.send(Ok(response));
                    }
                    KeyCode::Backspace => {
                        input.pop();
                        self.overlay = Some(overlay);
                        return Ok(true);
                    }
                    KeyCode::Char(c) => {
                        if !key.modifiers.contains(KeyModifiers::CONTROL) {
                            input.push(c);
                        }
                        self.overlay = Some(overlay);
                        return Ok(true);
                    }
                    _ => {
                        self.overlay = Some(overlay);
                        return Ok(true);
                    }
                }
                false
            }
            Overlay::ContinuePrompt { reply } => {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        let _ = reply.send(Ok(true));
                    }
                    KeyCode::Enter | KeyCode::Char('n') | KeyCode::Char('N') => {
                        let _ = reply.send(Ok(false));
                    }
                    KeyCode::Esc | KeyCode::Char('q') => {
                        let _ = reply.send(Err("inspect-output cancelled".to_string()));
                    }
                    _ => {
                        self.overlay = Some(overlay);
                        return Ok(true);
                    }
                }
                false
            }
            Overlay::Confirm { action, .. } => {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Enter => {
                        self.confirm_action(action.clone())?;
                    }
                    KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => {}
                    _ => {
                        self.overlay = Some(overlay);
                        return Ok(true);
                    }
                }
                false
            }
            Overlay::SelectRerunStage {
                stages,
                selected,
                run_id,
                apply,
            } => {
                match key.code {
                    KeyCode::Down | KeyCode::Char('j') => {
                        *selected = (*selected + 1).min(stages.len().saturating_sub(1));
                        self.overlay = Some(overlay);
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        *selected = selected.saturating_sub(1);
                        self.overlay = Some(overlay);
                    }
                    KeyCode::Enter => {
                        if let Some(stage) = stages.get(*selected).copied() {
                            let run_id = run_id.clone();
                            let apply = *apply;
                            self.start_async_operation("Rerun Session", move |tx| async move {
                                match crate::rerun_run(
                                    Some(run_id.clone()),
                                    Some(stage),
                                    apply,
                                    0,
                                    false,
                                )
                                .await
                                {
                                    Ok(_) => tx.send(OperationOutcome::success(
                                        "Rerun Session",
                                        format!(
                                            "reran {run_id} from {} in {} mode",
                                            rerun_stage_name(stage),
                                            if apply { "apply" } else { "preview" }
                                        ),
                                        OperationDetail::None,
                                    )),
                                    Err(err) => tx.send(OperationOutcome::failure(
                                        "Rerun Session",
                                        err.to_string(),
                                        OperationDetail::None,
                                    )),
                                }
                                .ok();
                            });
                        }
                    }
                    KeyCode::Esc | KeyCode::Char('q') => {}
                    _ => {
                        self.overlay = Some(overlay);
                        return Ok(true);
                    }
                }
                return Ok(true);
            }
        };

        Ok(handled)
    }

    fn apply_edit(&mut self, value: String) -> Result<()> {
        match self.screen {
            Screen::RunForm => self.run_form.apply_edit(value)?,
            Screen::ExtractForm => self.extract_form.apply_edit(value)?,
            Screen::Home | Screen::Sessions | Screen::Init | Screen::Operation => {}
        }
        Ok(())
    }

    fn confirm_action(&mut self, action: ConfirmAction) -> Result<()> {
        match action {
            ConfirmAction::RemoveRun(run_id) => {
                self.start_blocking_operation("Remove Session", move || {
                    let removed = RunWorkspace::remove_runs(&[run_id.clone()])?;
                    let summary = if removed.is_empty() {
                        "no sessions removed".to_string()
                    } else {
                        format!("removed {}", removed.join(", "))
                    };
                    Ok(OperationOutcome::success(
                        "Remove Session",
                        summary,
                        OperationDetail::None,
                    ))
                });
            }
            ConfirmAction::ClearIncomplete => {
                self.start_blocking_operation("Clear Incomplete Sessions", move || {
                    let removed = RunWorkspace::clear_incomplete_runs()?;
                    Ok(OperationOutcome::success(
                        "Clear Incomplete Sessions",
                        format!("cleared {} incomplete session(s)", removed.len()),
                        OperationDetail::None,
                    ))
                });
            }
        }
        Ok(())
    }

    fn open_rerun_overlay(&mut self, apply: bool) -> Result<()> {
        let Some(run_id) = self.session_view.selected_run_id() else {
            return Ok(());
        };
        let workspace = RunWorkspace::open(&run_id)?;
        let config = workspace.load_config()?;
        let stages = stage_sequence(config.rebuild && config.output.exists(), true);
        self.overlay = Some(Overlay::SelectRerunStage {
            run_id,
            apply,
            stages,
            selected: 0,
        });
        Ok(())
    }

    fn start_async_operation<Fut, F>(&mut self, title: &str, build: F)
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

    fn start_blocking_operation<F>(&mut self, title: &str, work: F)
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
        self.operation = OperationView {
            title: title.to_string(),
            running: true,
            success: false,
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

    fn draw(&self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(10),
                Constraint::Length(11),
            ])
            .split(frame.area());

        self.draw_header(frame, chunks[0]);
        match self.screen {
            Screen::Home => self.draw_home(frame, chunks[1]),
            Screen::RunForm => self.run_form.draw(frame, chunks[1]),
            Screen::Sessions => self.session_view.draw(frame, chunks[1]),
            Screen::ExtractForm => self.extract_form.draw(frame, chunks[1]),
            Screen::Init => self.draw_init(frame, chunks[1]),
            Screen::Operation => self.draw_operation(frame, chunks[1]),
        }
        self.draw_footer(frame, chunks[2]);

        if let Some(overlay) = &self.overlay {
            self.draw_overlay(frame, overlay);
        }
    }

    fn draw_header(&self, frame: &mut Frame, area: Rect) {
        let title = match self.screen {
            Screen::Home => "Home",
            Screen::RunForm => "Run Configuration",
            Screen::Sessions => "Sessions",
            Screen::ExtractForm => "Extract Text",
            Screen::Init => "Init Config",
            Screen::Operation => &self.operation.title,
        };
        let status = if self.operation.running {
            "busy"
        } else if self.operation.success {
            "ready"
        } else {
            "idle"
        };
        let header = Paragraph::new(Line::from(vec![
            Span::styled(
                " SortYourPapers ",
                Style::default().fg(Color::Black).bg(Color::Cyan),
            ),
            Span::raw(format!(" {title}")),
            Span::raw(" "),
            Span::styled(
                format!("[{status}]"),
                Style::default().fg(if self.operation.running {
                    Color::Yellow
                } else {
                    Color::Green
                }),
            ),
        ]))
        .block(Block::default().borders(Borders::ALL));
        frame.render_widget(header, area);
    }

    fn draw_home(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
            .split(area);

        let menu_lines = HOME_ITEMS
            .iter()
            .enumerate()
            .map(|(index, item)| {
                if index == self.home_index {
                    Line::from(Span::styled(
                        format!("> {item}"),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Line::from(format!("  {item}"))
                }
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(menu_lines)
                .block(Block::default().title("Actions").borders(Borders::ALL)),
            chunks[0],
        );

        let help = Paragraph::new(Text::from(vec![
            Line::from("`syp` is the interactive terminal frontend."),
            Line::from(""),
            Line::from("Run: configure the full sorting workflow."),
            Line::from("Sessions: resume, rerun, review, remove, or clear saved runs."),
            Line::from("Extract Text: preview manual extraction output."),
            Line::from("Init Config: write the default XDG config file."),
            Line::from(""),
            Line::from("Keys: ↑/↓ move, Enter open, q quit."),
        ]))
        .wrap(Wrap { trim: false })
        .block(Block::default().title("Overview").borders(Borders::ALL));
        frame.render_widget(help, chunks[1]);
    }

    fn draw_init(&self, frame: &mut Frame, area: Rect) {
        let body = Paragraph::new(Text::from(vec![
            Line::from("Create or overwrite the default XDG config file."),
            Line::from(""),
            Line::from(format!(
                "force overwrite: {}",
                if self.init_force { "yes" } else { "no" }
            )),
            Line::from(""),
            Line::from("Keys: space toggle, Enter run, Esc back."),
        ]))
        .wrap(Wrap { trim: false })
        .block(Block::default().title("Init Config").borders(Borders::ALL));
        frame.render_widget(body, area);
    }

    fn draw_operation(&self, frame: &mut Frame, area: Rect) {
        let status_height = operation_status_height(area.height, self.progress.len());
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(status_height),
                Constraint::Percentage(45),
                Constraint::Percentage(55),
            ])
            .split(area);

        self.draw_operation_status(frame, chunks[0]);

        let detail_lines = self.operation_detail_lines();
        frame.render_widget(
            Paragraph::new(detail_lines)
                .wrap(Wrap { trim: false })
                .block(Block::default().title("Details").borders(Borders::ALL)),
            chunks[1],
        );

        let log_lines = self
            .logs
            .iter()
            .rev()
            .take(18)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|line| Line::from(line.clone()))
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(log_lines)
                .wrap(Wrap { trim: false })
                .block(Block::default().title("Logs").borders(Borders::ALL)),
            chunks[2],
        );
    }

    fn operation_detail_lines(&self) -> Text<'static> {
        if let OperationDetail::Text(lines) = &self.operation.detail {
            return Text::from(lines.iter().cloned().map(Line::from).collect::<Vec<_>>());
        }

        if let OperationDetail::Tree(categories) = &self.operation.detail {
            return Text::from(
                crate::terminal::report::render_category_tree(categories)
                    .lines()
                    .map(|line| Line::from(line.to_string()))
                    .collect::<Vec<_>>(),
            );
        }

        if let Some(report) = &self.last_report {
            return Text::from(
                crate::terminal::report::render_report_lines(
                    report,
                    terminal::Verbosity::new(false, false, false),
                )
                .into_iter()
                .map(Line::from)
                .collect::<Vec<_>>(),
            );
        }

        if let Some(tree) = &self.last_category_tree {
            return Text::from(
                tree.lines()
                    .map(|line| Line::from(line.to_string()))
                    .collect::<Vec<_>>(),
            );
        }

        Text::from(vec![Line::from(self.operation.summary.clone())])
    }

    fn draw_operation_status(&self, frame: &mut Frame, area: Rect) {
        if self.progress.is_empty() {
            frame.render_widget(
                Paragraph::new(vec![Line::from(self.operation.summary.clone())])
                    .wrap(Wrap { trim: false })
                    .block(Block::default().title("Status").borders(Borders::ALL)),
                area,
            );
            return;
        }

        let block = Block::default().title("Status").borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.height == 0 || inner.width == 0 {
            return;
        }

        let visible = usize::from(inner.height).min(self.progress.len());
        let progress_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(vec![Constraint::Length(1); visible])
            .split(inner);

        for (progress, chunk) in self
            .progress
            .iter()
            .take(visible)
            .zip(progress_chunks.iter().copied())
        {
            frame.render_widget(
                Gauge::default()
                    .ratio(progress.ratio())
                    .label(progress.label())
                    .gauge_style(progress.gauge_style())
                    .use_unicode(true),
                chunk,
            );
        }
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        let help = match self.screen {
            Screen::Home => "↑/↓ move  Enter open  q quit",
            Screen::RunForm => "↑/↓ select  Enter edit/run  ←/→ cycle  space toggle  Esc back",
            Screen::Sessions => {
                "↑/↓ select  p preview  a apply  r rerun  x rerun-apply  v review  d delete  c clear  g refresh  Esc back"
            }
            Screen::ExtractForm => "↑/↓ select  Enter edit/run  ←/→ cycle  Esc back",
            Screen::Init => "space toggle  Enter run  Esc back",
            Screen::Operation => "b/Esc back when idle  q quit when idle",
        };
        frame.render_widget(
            Paragraph::new(help).block(Block::default().borders(Borders::ALL)),
            area,
        );
    }

    fn draw_overlay(&self, frame: &mut Frame, overlay: &Overlay) {
        let area = centered_rect(70, 60, frame.area());
        frame.render_widget(Clear, area);

        match overlay {
            Overlay::EditField { label, buffer } => {
                let widget = Paragraph::new(Text::from(vec![
                    Line::from(format!("Editing {label}")),
                    Line::from(""),
                    Line::from(buffer.clone()),
                    Line::from(""),
                    Line::from("Enter save  Esc cancel"),
                ]))
                .wrap(Wrap { trim: false })
                .block(Block::default().title("Edit Field").borders(Borders::ALL));
                frame.render_widget(widget, area);
            }
            Overlay::InspectPrompt {
                categories, input, ..
            } => {
                let tree = crate::terminal::report::render_category_tree(categories);
                let widget = Paragraph::new(Text::from(vec![
                    Line::from("Review the current taxonomy."),
                    Line::from(""),
                    Line::from(tree),
                    Line::from(""),
                    Line::from("Enter suggestion text, press Enter to accept current taxonomy, or q to cancel."),
                    Line::from(""),
                    Line::from(format!("Suggestion: {input}")),
                ]))
                .wrap(Wrap { trim: false })
                .block(Block::default().title("Inspect Taxonomy").borders(Borders::ALL));
                frame.render_widget(widget, area);
            }
            Overlay::ContinuePrompt { .. } => {
                let widget = Paragraph::new(Text::from(vec![
                    Line::from("Continue improving this taxonomy?"),
                    Line::from(""),
                    Line::from("y continue"),
                    Line::from("Enter or n finish"),
                    Line::from("q cancel"),
                ]))
                .block(
                    Block::default()
                        .title("Continue Improving")
                        .borders(Borders::ALL),
                );
                frame.render_widget(widget, area);
            }
            Overlay::Confirm { title, message, .. } => {
                let widget = Paragraph::new(Text::from(vec![
                    Line::from(message.clone()),
                    Line::from(""),
                    Line::from("Enter or y confirm"),
                    Line::from("Esc, n, or q cancel"),
                ]))
                .wrap(Wrap { trim: false })
                .block(Block::default().title(title.clone()).borders(Borders::ALL));
                frame.render_widget(widget, area);
            }
            Overlay::SelectRerunStage {
                stages, selected, ..
            } => {
                let lines = stages
                    .iter()
                    .enumerate()
                    .map(|(index, stage)| {
                        let line = format!("{} {}", rerun_stage_name(*stage), stage.description());
                        if index == *selected {
                            Line::from(Span::styled(
                                format!("> {line}"),
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD),
                            ))
                        } else {
                            Line::from(format!("  {line}"))
                        }
                    })
                    .collect::<Vec<_>>();
                frame.render_widget(
                    Paragraph::new(lines).wrap(Wrap { trim: false }).block(
                        Block::default()
                            .title("Select Rerun Stage")
                            .borders(Borders::ALL),
                    ),
                    area,
                );
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Screen {
    Home,
    RunForm,
    Sessions,
    ExtractForm,
    Init,
    Operation,
}

#[derive(Default)]
struct OperationView {
    title: String,
    running: bool,
    success: bool,
    summary: String,
    detail: OperationDetail,
}

#[derive(Default)]
enum OperationDetail {
    #[default]
    None,
    Text(Vec<String>),
    Tree(Vec<CategoryTree>),
}

struct OperationOutcome {
    title: String,
    success: bool,
    summary: String,
    detail: OperationDetail,
}

impl OperationOutcome {
    fn success(title: &str, summary: String, detail: OperationDetail) -> Self {
        Self {
            title: title.to_string(),
            success: true,
            summary,
            detail,
        }
    }

    fn failure(title: &str, summary: String, detail: OperationDetail) -> Self {
        Self {
            title: title.to_string(),
            success: false,
            summary,
            detail,
        }
    }
}

#[derive(Clone)]
enum ConfirmAction {
    RemoveRun(String),
    ClearIncomplete,
}

enum Overlay {
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

struct ProgressEntry {
    id: u64,
    label: String,
    total: usize,
    current: usize,
}

impl ProgressEntry {
    fn ratio(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.current as f64 / self.total as f64
        }
    }

    fn label(&self) -> String {
        format!("{} {}/{}", self.label, self.current, self.total)
    }

    fn gauge_style(&self) -> Style {
        Style::default().fg(if self.current >= self.total && self.total > 0 {
            Color::Green
        } else {
            Color::Cyan
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum UiVerbosity {
    Normal,
    Verbose,
    Debug,
}

impl UiVerbosity {
    fn count(self) -> u8 {
        match self {
            Self::Normal => 0,
            Self::Verbose => 1,
            Self::Debug => 2,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Normal => Self::Verbose,
            Self::Verbose => Self::Debug,
            Self::Debug => Self::Normal,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Normal => Self::Debug,
            Self::Verbose => Self::Normal,
            Self::Debug => Self::Verbose,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Verbose => "verbose",
            Self::Debug => "debug",
        }
    }
}

struct RunForm {
    selected: usize,
    input: String,
    output: String,
    recursive: bool,
    max_file_size_mb: String,
    page_cutoff: String,
    pdf_extract_workers: String,
    category_depth: String,
    taxonomy_mode: TaxonomyMode,
    taxonomy_batch_size: String,
    placement_batch_size: String,
    placement_mode: PlacementMode,
    rebuild: bool,
    apply: bool,
    llm_provider: LlmProvider,
    llm_model: String,
    llm_base_url: String,
    api_key: String,
    keyword_batch_size: String,
    subcategories_suggestion_number: String,
    verbosity: UiVerbosity,
    quiet: bool,
}

impl Default for RunForm {
    fn default() -> Self {
        Self {
            selected: 0,
            input: DEFAULT_INPUT.to_string(),
            output: DEFAULT_OUTPUT.to_string(),
            recursive: false,
            max_file_size_mb: DEFAULT_MAX_FILE_SIZE_MB.to_string(),
            page_cutoff: DEFAULT_PAGE_CUTOFF.to_string(),
            pdf_extract_workers: DEFAULT_PDF_EXTRACT_WORKERS.to_string(),
            category_depth: DEFAULT_CATEGORY_DEPTH.to_string(),
            taxonomy_mode: TaxonomyMode::BatchMerge,
            taxonomy_batch_size: DEFAULT_TAXONOMY_BATCH_SIZE.to_string(),
            placement_batch_size: DEFAULT_PLACEMENT_BATCH_SIZE.to_string(),
            placement_mode: PlacementMode::ExistingOnly,
            rebuild: false,
            apply: false,
            llm_provider: DEFAULT_LLM_PROVIDER,
            llm_model: DEFAULT_LLM_MODEL.to_string(),
            llm_base_url: String::new(),
            api_key: String::new(),
            keyword_batch_size: DEFAULT_KEYWORD_BATCH_SIZE.to_string(),
            subcategories_suggestion_number: DEFAULT_SUBCATEGORIES_SUGGESTION_NUMBER.to_string(),
            verbosity: UiVerbosity::Normal,
            quiet: false,
        }
    }
}

impl RunForm {
    fn draw(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area);

        let lines = RUN_FIELD_LABELS
            .iter()
            .enumerate()
            .map(|(index, label)| {
                let line = format!("{label}: {}", self.value(index));
                if index == self.selected {
                    Line::from(Span::styled(
                        format!("> {line}"),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Line::from(format!("  {line}"))
                }
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(Block::default().title("Run Fields").borders(Borders::ALL)),
            chunks[0],
        );

        let help = Paragraph::new(Text::from(vec![
            Line::from("Enter edits text/number/path fields."),
            Line::from("Left/Right cycles enum fields."),
            Line::from("Space toggles booleans."),
            Line::from(""),
            Line::from(format!(
                "mode: {}",
                if self.apply { "apply" } else { "preview" }
            )),
            Line::from(format!("verbosity: {}", self.verbosity.label())),
            Line::from(""),
            Line::from("Press r to start the run."),
        ]))
        .wrap(Wrap { trim: false })
        .block(Block::default().title("Help").borders(Borders::ALL));
        frame.render_widget(help, chunks[1]);
    }

    fn build_config(&self) -> Result<AppConfig> {
        let cli = CliArgs {
            input: Some(PathBuf::from(self.input.clone())),
            output: Some(PathBuf::from(self.output.clone())),
            recursive: Some(self.recursive),
            max_file_size_mb: Some(parse_u64("max_file_size_mb", &self.max_file_size_mb)?),
            page_cutoff: Some(parse_u8("page_cutoff", &self.page_cutoff)?),
            pdf_extract_workers: Some(parse_usize(
                "pdf_extract_workers",
                &self.pdf_extract_workers,
            )?),
            category_depth: Some(parse_u8("category_depth", &self.category_depth)?),
            taxonomy_mode: Some(self.taxonomy_mode),
            taxonomy_batch_size: Some(parse_usize(
                "taxonomy_batch_size",
                &self.taxonomy_batch_size,
            )?),
            placement_batch_size: Some(parse_usize(
                "placement_batch_size",
                &self.placement_batch_size,
            )?),
            placement_mode: Some(self.placement_mode),
            rebuild: Some(self.rebuild),
            apply: self.apply,
            llm_provider: Some(self.llm_provider),
            llm_model: Some(self.llm_model.clone()),
            llm_base_url: empty_string_to_option(&self.llm_base_url),
            api_key: empty_string_to_option(&self.api_key),
            keyword_batch_size: Some(parse_usize("keyword_batch_size", &self.keyword_batch_size)?),
            subcategories_suggestion_number: Some(parse_usize(
                "subcategories_suggestion_number",
                &self.subcategories_suggestion_number,
            )?),
            verbosity: self.verbosity.count(),
            quiet: self.quiet,
        };
        config::resolve_config(cli)
    }

    fn editable(&self, index: usize) -> bool {
        !matches!(index, 2 | 7 | 10 | 11 | 12 | 13 | 19 | 20)
    }

    fn toggle_selected(&mut self) {
        match self.selected {
            2 => self.recursive = !self.recursive,
            11 => self.rebuild = !self.rebuild,
            12 => self.apply = !self.apply,
            20 => self.quiet = !self.quiet,
            _ => self.cycle_selected(1),
        }
    }

    fn cycle_selected(&mut self, direction: i8) {
        match self.selected {
            7 => self.taxonomy_mode = cycle_taxonomy_mode(self.taxonomy_mode, direction),
            10 => self.placement_mode = cycle_placement_mode(self.placement_mode, direction),
            13 => self.llm_provider = cycle_provider(self.llm_provider, direction),
            19 => {
                self.verbosity = if direction >= 0 {
                    self.verbosity.next()
                } else {
                    self.verbosity.previous()
                }
            }
            _ => {}
        }
    }

    fn apply_edit(&mut self, value: String) -> Result<()> {
        match self.selected {
            0 => self.input = value,
            1 => self.output = value,
            3 => self.max_file_size_mb = value,
            4 => self.page_cutoff = value,
            5 => self.pdf_extract_workers = value,
            6 => self.category_depth = value,
            8 => self.taxonomy_batch_size = value,
            9 => self.placement_batch_size = value,
            14 => self.llm_model = value,
            15 => self.llm_base_url = value,
            16 => self.api_key = value,
            17 => self.keyword_batch_size = value,
            18 => self.subcategories_suggestion_number = value,
            _ => {}
        }
        Ok(())
    }

    fn value(&self, index: usize) -> String {
        match index {
            0 => self.input.clone(),
            1 => self.output.clone(),
            2 => bool_label(self.recursive).to_string(),
            3 => self.max_file_size_mb.clone(),
            4 => self.page_cutoff.clone(),
            5 => self.pdf_extract_workers.clone(),
            6 => self.category_depth.clone(),
            7 => taxonomy_mode_label(self.taxonomy_mode).to_string(),
            8 => self.taxonomy_batch_size.clone(),
            9 => self.placement_batch_size.clone(),
            10 => placement_mode_label(self.placement_mode).to_string(),
            11 => bool_label(self.rebuild).to_string(),
            12 => bool_label(self.apply).to_string(),
            13 => provider_label(self.llm_provider).to_string(),
            14 => self.llm_model.clone(),
            15 => self.llm_base_url.clone(),
            16 => masked_value(&self.api_key),
            17 => self.keyword_batch_size.clone(),
            18 => self.subcategories_suggestion_number.clone(),
            19 => self.verbosity.label().to_string(),
            20 => bool_label(self.quiet).to_string(),
            _ => String::new(),
        }
    }
}

struct ExtractForm {
    selected: usize,
    files: String,
    page_cutoff: String,
    extractor: ExtractorMode,
    pdf_extract_workers: String,
    verbosity: UiVerbosity,
}

impl Default for ExtractForm {
    fn default() -> Self {
        Self {
            selected: 0,
            files: String::new(),
            page_cutoff: DEFAULT_PAGE_CUTOFF.to_string(),
            extractor: ExtractorMode::Auto,
            pdf_extract_workers: DEFAULT_PDF_EXTRACT_WORKERS.to_string(),
            verbosity: UiVerbosity::Normal,
        }
    }
}

impl ExtractForm {
    fn draw(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area);

        let lines = EXTRACT_FIELD_LABELS
            .iter()
            .enumerate()
            .map(|(index, label)| {
                let line = format!("{label}: {}", self.value(index));
                if index == self.selected {
                    Line::from(Span::styled(
                        format!("> {line}"),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Line::from(format!("  {line}"))
                }
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }).block(
                Block::default()
                    .title("Extract Fields")
                    .borders(Borders::ALL),
            ),
            chunks[0],
        );

        let help = Paragraph::new(Text::from(vec![
            Line::from("Files may be separated by commas or new lines."),
            Line::from("Enter edits text fields."),
            Line::from("Left/Right cycles extractor and verbosity."),
            Line::from(""),
            Line::from("Press r to run extraction."),
        ]))
        .wrap(Wrap { trim: false })
        .block(Block::default().title("Help").borders(Borders::ALL));
        frame.render_widget(help, chunks[1]);
    }

    fn apply_edit(&mut self, value: String) -> Result<()> {
        match self.selected {
            0 => self.files = value,
            1 => self.page_cutoff = value,
            3 => self.pdf_extract_workers = value,
            _ => {}
        }
        Ok(())
    }

    fn build_args(&self) -> Result<ExtractTextArgs> {
        let files = self
            .files
            .split(|c| c == ',' || c == '\n')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        if files.is_empty() {
            return Err(AppError::Validation(
                "provide at least one PDF path".to_string(),
            ));
        }
        Ok(ExtractTextArgs {
            files,
            page_cutoff: parse_u8("page_cutoff", &self.page_cutoff)?,
            extractor: self.extractor,
            pdf_extract_workers: parse_usize("pdf_extract_workers", &self.pdf_extract_workers)?,
            verbosity: self.verbosity.count(),
        })
    }

    fn cycle_selected(&mut self, direction: i8) {
        match self.selected {
            2 => self.extractor = cycle_extractor(self.extractor, direction),
            4 => {
                self.verbosity = if direction >= 0 {
                    self.verbosity.next()
                } else {
                    self.verbosity.previous()
                };
            }
            _ => {}
        }
    }

    fn value(&self, index: usize) -> String {
        match index {
            0 => self.files.clone(),
            1 => self.page_cutoff.clone(),
            2 => extractor_label(self.extractor).to_string(),
            3 => self.pdf_extract_workers.clone(),
            4 => self.verbosity.label().to_string(),
            _ => String::new(),
        }
    }
}

#[derive(Default)]
struct SessionView {
    runs: Vec<RunSummary>,
    selected: usize,
}

impl SessionView {
    fn refresh(&mut self) -> Result<()> {
        self.runs = RunWorkspace::list_runs()?;
        if self.selected >= self.runs.len() {
            self.selected = self.runs.len().saturating_sub(1);
        }
        Ok(())
    }

    fn move_selection(&mut self, delta: isize) {
        if self.runs.is_empty() {
            self.selected = 0;
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.clamp(0, self.runs.len().saturating_sub(1) as isize) as usize;
    }

    fn selected_run_id(&self) -> Option<String> {
        self.runs.get(self.selected).map(|run| run.run_id.clone())
    }

    fn draw(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area);

        let lines = if self.runs.is_empty() {
            vec![Line::from("No saved sessions found")]
        } else {
            self.runs
                .iter()
                .enumerate()
                .map(|(index, run)| {
                    let line = format_run_summary(index, run);
                    if index == self.selected {
                        Line::from(Span::styled(
                            format!("> {line}"),
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ))
                    } else {
                        Line::from(format!("  {line}"))
                    }
                })
                .collect::<Vec<_>>()
        };
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(Block::default().title("Saved Runs").borders(Borders::ALL)),
            chunks[0],
        );

        let detail_lines = if let Some(run) = self.runs.get(self.selected) {
            vec![
                Line::from(format!("run_id: {}", run.run_id)),
                Line::from(format!("cwd: {}", run.cwd.display())),
                Line::from(format!("created_unix_ms: {}", run.created_unix_ms)),
                Line::from(format!(
                    "last stage: {}",
                    run.last_completed_stage
                        .map_or_else(|| "NotStarted".to_string(), |stage| format!("{stage:?}"))
                )),
                Line::from(format!("latest: {}", bool_label(run.is_latest))),
                Line::from(""),
                Line::from("p resume preview"),
                Line::from("a resume apply"),
                Line::from("r rerun preview"),
                Line::from("x rerun apply"),
                Line::from("v review taxonomy"),
                Line::from("d delete selected"),
                Line::from("c clear incomplete"),
            ]
        } else {
            vec![Line::from("No run selected")]
        };
        frame.render_widget(
            Paragraph::new(detail_lines)
                .wrap(Wrap { trim: false })
                .block(Block::default().title("Details").borders(Borders::ALL)),
            chunks[1],
        );
    }
}

struct ExtractPreview {
    papers: Vec<crate::models::PaperText>,
    failures: Vec<(PathBuf, String)>,
}

async fn collect_extract_preview(args: ExtractTextArgs) -> Result<ExtractPreview> {
    if args.page_cutoff == 0 {
        return Err(AppError::Validation(
            "page_cutoff must be greater than 0".to_string(),
        ));
    }
    if args.pdf_extract_workers == 0 {
        return Err(AppError::Validation(
            "pdf_extract_workers must be greater than 0".to_string(),
        ));
    }

    let verbose = args.verbosity > 0;
    let debug = args.verbosity > 1;
    reset_debug_extract_log(debug)?;

    let candidates = args
        .files
        .iter()
        .map(|path| crate::models::PdfCandidate {
            path: path.clone(),
            size_bytes: 0,
        })
        .collect::<Vec<_>>();
    let verbosity = terminal::Verbosity::new(verbose, debug, false);
    let (papers, failures) = extract_text_batch(
        &candidates,
        args.page_cutoff,
        args.extractor,
        debug,
        args.pdf_extract_workers,
        verbosity,
    )
    .await;
    Ok(ExtractPreview { papers, failures })
}

fn render_extract_result_lines(result: &ExtractPreview) -> Vec<String> {
    let mut lines = Vec::new();
    for paper in &result.papers {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(format!("=== {} ===", paper.path.display()));
        lines.push(format!("file_id: {}", paper.file_id));
        lines.push(format!("pages_read: {}", paper.pages_read));
        lines.push(String::new());
        lines.push("--- raw ---".to_string());
        lines.push(paper.extracted_text.clone());
        if !paper.llm_ready_text.is_empty() {
            lines.push(String::new());
            lines.push("--- llm-ready ---".to_string());
            lines.push(paper.llm_ready_text.clone());
        }
    }

    for (path, err) in &result.failures {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(format!("[extract-failed] {}: {err}", path.display()));
    }
    if lines.is_empty() {
        lines.push("No extract output".to_string());
    }
    lines
}

const HOME_ITEMS: [&str; 5] = [
    "Run Papers",
    "Sessions",
    "Extract Text",
    "Init Config",
    "Quit",
];

const RUN_FIELD_LABELS: [&str; 21] = [
    "input",
    "output",
    "recursive",
    "max_file_size_mb",
    "page_cutoff",
    "pdf_extract_workers",
    "category_depth",
    "taxonomy_mode",
    "taxonomy_batch_size",
    "placement_batch_size",
    "placement_mode",
    "rebuild",
    "apply",
    "llm_provider",
    "llm_model",
    "llm_base_url",
    "api_key",
    "keyword_batch_size",
    "subcategories_suggestion_number",
    "verbosity",
    "quiet",
];

const EXTRACT_FIELD_LABELS: [&str; 5] = [
    "files",
    "page_cutoff",
    "extractor",
    "pdf_extract_workers",
    "verbosity",
];

fn empty_string_to_option(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn bool_label(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn masked_value(value: &str) -> String {
    if value.is_empty() {
        String::new()
    } else {
        "*".repeat(value.len().min(8))
    }
}

fn provider_label(value: LlmProvider) -> &'static str {
    match value {
        LlmProvider::Openai => "openai",
        LlmProvider::Ollama => "ollama",
        LlmProvider::Gemini => "gemini",
    }
}

fn taxonomy_mode_label(value: TaxonomyMode) -> &'static str {
    match value {
        TaxonomyMode::Global => "global",
        TaxonomyMode::BatchMerge => "batch-merge",
    }
}

fn placement_mode_label(value: PlacementMode) -> &'static str {
    match value {
        PlacementMode::ExistingOnly => "existing-only",
        PlacementMode::AllowNew => "allow-new",
    }
}

fn extractor_label(value: ExtractorMode) -> &'static str {
    match value {
        ExtractorMode::Auto => "auto",
        ExtractorMode::PdfOxide => "pdf-oxide",
        ExtractorMode::Pdftotext => "pdftotext",
    }
}

fn cycle_provider(value: LlmProvider, direction: i8) -> LlmProvider {
    let all = [
        LlmProvider::Openai,
        LlmProvider::Ollama,
        LlmProvider::Gemini,
    ];
    cycle_enum(value, &all, direction)
}

fn cycle_taxonomy_mode(value: TaxonomyMode, direction: i8) -> TaxonomyMode {
    let all = [TaxonomyMode::Global, TaxonomyMode::BatchMerge];
    cycle_enum(value, &all, direction)
}

fn cycle_placement_mode(value: PlacementMode, direction: i8) -> PlacementMode {
    let all = [PlacementMode::ExistingOnly, PlacementMode::AllowNew];
    cycle_enum(value, &all, direction)
}

fn cycle_extractor(value: ExtractorMode, direction: i8) -> ExtractorMode {
    let all = [
        ExtractorMode::Auto,
        ExtractorMode::PdfOxide,
        ExtractorMode::Pdftotext,
    ];
    cycle_enum(value, &all, direction)
}

fn cycle_enum<T>(value: T, values: &[T], direction: i8) -> T
where
    T: Copy + PartialEq,
{
    let index = values
        .iter()
        .position(|candidate| *candidate == value)
        .unwrap_or(0);
    let next = if direction >= 0 {
        (index + 1) % values.len()
    } else if index == 0 {
        values.len() - 1
    } else {
        index - 1
    };
    values[next]
}

fn parse_u64(name: &str, value: &str) -> Result<u64> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|err| AppError::Validation(format!("invalid {name}: {err}")))
}

fn parse_usize(name: &str, value: &str) -> Result<usize> {
    value
        .trim()
        .parse::<usize>()
        .map_err(|err| AppError::Validation(format!("invalid {name}: {err}")))
}

fn parse_u8(name: &str, value: &str) -> Result<u8> {
    value
        .trim()
        .parse::<u8>()
        .map_err(|err| AppError::Validation(format!("invalid {name}: {err}")))
}

fn format_run_summary(index: usize, run: &RunSummary) -> String {
    let stage = run
        .last_completed_stage
        .map_or_else(|| "NotStarted".to_string(), |stage| format!("{stage:?}"));
    let latest = if run.is_latest { " latest" } else { "" };
    format!(
        "{}. {} | stage={} | cwd={} | created_unix_ms={}{}",
        index + 1,
        run.run_id,
        stage,
        run.cwd.display(),
        run.created_unix_ms,
        latest
    )
}

fn rerun_stage_name(stage: RunStage) -> &'static str {
    match stage {
        RunStage::DiscoverInput => "discover-input",
        RunStage::DiscoverOutput => "discover-output",
        RunStage::Dedupe => "dedupe",
        RunStage::FilterSize => "filter-size",
        RunStage::ExtractText => "extract-text",
        RunStage::BuildLlmClient => "build-llm-client",
        RunStage::ExtractKeywords => "extract-keywords",
        RunStage::SynthesizeCategories => "synthesize-categories",
        RunStage::InspectOutput => "inspect-output",
        RunStage::GeneratePlacements => "generate-placements",
        RunStage::BuildPlan => "build-plan",
        RunStage::ExecutePlan => "execute-plan",
        RunStage::Completed => "completed",
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn operation_status_height(area_height: u16, progress_count: usize) -> u16 {
    let preferred = if progress_count == 0 {
        5
    } else {
        progress_count as u16 + 2
    };
    let max_height = area_height.saturating_sub(6).max(3);
    preferred.clamp(3, max_height)
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::mpsc};

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    use super::{
        App, BackendEvent, ExtractForm, OperationDetail, OperationView, Overlay, PlacementMode,
        ProgressEntry, RunForm, Screen, SessionView, TaxonomyMode, UiVerbosity,
    };

    fn test_app() -> App {
        let (_backend_tx, backend_rx) = mpsc::channel();
        let (op_tx, op_rx) = mpsc::channel();
        App {
            screen: Screen::Operation,
            home_index: 0,
            run_form: RunForm::default(),
            extract_form: ExtractForm::default(),
            init_force: false,
            session_view: SessionView::default(),
            overlay: None,
            operation: OperationView {
                title: "Operation".to_string(),
                running: false,
                success: true,
                summary: "waiting for work".to_string(),
                detail: OperationDetail::None,
            },
            logs: VecDeque::new(),
            progress: Vec::new(),
            last_report: None,
            last_category_tree: None,
            should_quit: false,
            backend_rx,
            op_rx,
            op_tx,
            debug_tui: false,
        }
    }

    fn render_lines(app: &App, width: u16, height: u16) -> Vec<String> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        terminal
            .draw(|frame| app.draw(frame))
            .expect("test frame should render");
        let buffer = terminal.backend().buffer();
        let area = buffer.area();

        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn run_form_non_editable_fields_match_toggle_and_enum_fields() {
        let form = RunForm::default();

        assert!(!form.editable(2));
        assert!(!form.editable(7));
        assert!(!form.editable(10));
        assert!(!form.editable(11));
        assert!(!form.editable(12));
        assert!(!form.editable(13));
        assert!(!form.editable(19));
        assert!(!form.editable(20));
        assert!(form.editable(14));
        assert!(form.editable(18));
    }

    #[test]
    fn run_form_toggle_and_cycle_target_the_expected_fields() {
        let mut form = RunForm::default();

        form.selected = 7;
        form.toggle_selected();
        assert_eq!(form.taxonomy_mode, TaxonomyMode::Global);

        form.selected = 10;
        form.toggle_selected();
        assert_eq!(form.placement_mode, PlacementMode::AllowNew);

        form.selected = 19;
        form.toggle_selected();
        assert!(matches!(form.verbosity, UiVerbosity::Verbose));

        form.selected = 11;
        form.toggle_selected();
        assert!(form.rebuild);

        form.selected = 20;
        form.toggle_selected();
        assert!(form.quiet);
    }

    #[test]
    fn progress_events_add_advance_and_remove_entries() {
        let mut app = test_app();

        app.progress.push(ProgressEntry {
            id: 7,
            label: "taxonomy".to_string(),
            total: 3,
            current: 0,
        });
        app.progress.push(ProgressEntry {
            id: 9,
            label: "stale".to_string(),
            total: 1,
            current: 1,
        });

        let (backend_tx, backend_rx) = mpsc::channel();
        app.backend_rx = backend_rx;

        backend_tx
            .send(BackendEvent::ProgressStart {
                id: 12,
                total: 4,
                label: "keyword batches".to_string(),
            })
            .expect("progress start should send");
        backend_tx
            .send(BackendEvent::ProgressAdvance { id: 12, delta: 3 })
            .expect("progress advance should send");
        backend_tx
            .send(BackendEvent::ProgressAdvance { id: 12, delta: 3 })
            .expect("progress advance should send");
        backend_tx
            .send(BackendEvent::ProgressFinish { id: 9 })
            .expect("progress finish should send");

        app.drain_backend_events();

        assert_eq!(app.progress.len(), 2);
        let active = app
            .progress
            .iter()
            .find(|entry| entry.id == 12)
            .expect("new progress entry should exist");
        assert_eq!(active.current, 4);
        assert_eq!(active.total, 4);
        assert!(app.progress.iter().all(|entry| entry.id != 9));
    }

    #[test]
    fn operation_screen_shows_summary_when_no_progress_is_active() {
        let app = test_app();

        let lines = render_lines(&app, 80, 24);

        assert!(lines.iter().any(|line| line.contains("waiting for work")));
    }

    #[test]
    fn operation_screen_renders_progress_gauges_with_labels_and_counts() {
        let mut app = test_app();
        app.progress = vec![
            ProgressEntry {
                id: 1,
                label: "preprocessing".to_string(),
                total: 10,
                current: 3,
            },
            ProgressEntry {
                id: 2,
                label: "keyword batches".to_string(),
                total: 4,
                current: 4,
            },
        ];

        let lines = render_lines(&app, 100, 24);

        assert!(lines.iter().any(|line| line.contains("preprocessing 3/10")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("keyword batches 4/4"))
        );
    }

    #[test]
    fn edit_overlay_enter_commits_value_without_reopening_editor() {
        let mut app = test_app();
        app.screen = Screen::RunForm;
        app.run_form.selected = 0;
        app.overlay = Some(Overlay::EditField {
            label: "Input".to_string(),
            buffer: "papers".to_string(),
        });

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)))
            .expect("enter should commit edit");

        assert_eq!(app.run_form.input, "papers");
        assert!(app.overlay.is_none());
    }
}
