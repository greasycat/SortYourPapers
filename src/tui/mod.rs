mod app;
mod backend;
mod forms;
mod input;
mod model;
mod render;
mod session_view;

use std::{
    sync::{Arc, mpsc},
    time::Duration,
};

use crossterm::{
    cursor::SetCursorStyle,
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, prelude::CrosstermBackend};

use crate::{error::Result, terminal::install_backend};

use self::{app::App, backend::TuiBackend};

#[cfg(test)]
use self::{
    backend::BackendEvent,
    forms::{RunForm, UiVerbosity, ValidationSeverity},
    model::{OperationDetail, OperationState, OperationView, Overlay, ProgressEntry, Screen},
    session_view::SessionView,
};

pub async fn run(debug_tui: bool) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, SetCursorStyle::BlinkingBar)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let (backend_tx, backend_rx) = mpsc::channel();
    let (op_tx, op_rx) = mpsc::channel();
    let _backend_guard = install_backend(Arc::new(TuiBackend::new(backend_tx)));

    let mut app = App::new(backend_rx, op_rx, op_tx, debug_tui)?;
    let run_result = run_loop(&mut terminal, &mut app).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        SetCursorStyle::DefaultUserShape
    )?;
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

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::mpsc};

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend, layout::Position};
    use tempfile::tempdir;

    use crate::{
        papers::placement::PlacementMode, papers::taxonomy::TaxonomyMode, terminal::AlertSeverity,
        report::{FileAction, PlanAction, RunReport},
        session::{RunStage, RunSummary, SessionConfigSummary, SessionDetails, SessionStatusSummary},
    };

    use super::{
        App, BackendEvent, OperationDetail, OperationState, OperationView, Overlay, ProgressEntry,
        RunForm, Screen, SessionView, UiVerbosity, ValidationSeverity, model::OperationTab,
    };

    fn test_app() -> App {
        let (_backend_tx, backend_rx) = mpsc::channel();
        let (op_tx, op_rx) = mpsc::channel();
        App {
            screen: Screen::Operation,
            home_index: 0,
            run_form: RunForm::default(),
            session_view: SessionView::default(),
            overlay: None,
            operation: OperationView {
                title: "Operation".to_string(),
                state: OperationState::Idle,
                summary: "waiting for work".to_string(),
                detail: OperationDetail::None,
                ..OperationView::default()
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

    fn render_app(app: &App, width: u16, height: u16) -> Terminal<TestBackend> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        terminal
            .draw(|frame| app.draw(frame))
            .expect("test frame should render");
        terminal
    }

    fn test_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
    }

    fn sample_session_status(
        is_completed: bool,
        is_incomplete: bool,
        is_failed_looking: bool,
    ) -> SessionStatusSummary {
        SessionStatusSummary {
            is_completed,
            is_incomplete,
            is_failed_looking,
        }
    }

    fn sample_session_run(run_id: &str, stage: Option<RunStage>) -> RunSummary {
        RunSummary {
            run_id: run_id.to_string(),
            created_unix_ms: 120_000,
            cwd: "/tmp/project".into(),
            last_completed_stage: stage,
            is_latest: run_id == "run-complete",
        }
    }

    fn sample_session_details(run: RunSummary) -> SessionDetails {
        let mut report = RunReport::new(true);
        report.actions = (0..20)
            .map(|index| PlanAction {
                source: format!("/tmp/in/paper-{index:02}.pdf").into(),
                destination: format!("/tmp/out/topic-{index:02}.pdf").into(),
                action: FileAction::Move,
            })
            .collect();

        SessionDetails {
            run,
            config: SessionConfigSummary {
                dry_run: true,
                llm_provider: "gemini".to_string(),
                llm_model: "gemini-3-flash-preview".to_string(),
            },
            status: sample_session_status(true, false, false),
            report: Some(report),
            taxonomy: Some(vec![crate::papers::taxonomy::CategoryTree {
                name: "AI".to_string(),
                children: vec![crate::papers::taxonomy::CategoryTree {
                    name: "Vision".to_string(),
                    children: vec![],
                }],
            }]),
            available_stage_artifacts: vec![
                RunStage::ExtractText,
                RunStage::SynthesizeCategories,
                RunStage::BuildPlan,
            ],
        }
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
    fn run_form_navigation_skips_hidden_output_fields() {
        let mut form = RunForm::default();

        form.selected = 16;
        form.select_next();
        assert_eq!(form.selected, 11);

        form.select_next();
        assert_eq!(form.selected, 12);

        form.select_previous();
        assert_eq!(form.selected, 11);
    }

    #[test]
    fn run_form_column_navigation_moves_across_matching_rows() {
        let mut form = RunForm::default();

        form.selected = 4;
        form.move_column_right();
        assert_eq!(form.selected, 18);

        form.move_column_right();
        assert_eq!(form.selected, 11);

        form.move_column_left();
        assert_eq!(form.selected, 18);

        form.move_column_left();
        assert_eq!(form.selected, 4);
    }

    #[test]
    fn run_form_column_navigation_clamps_to_shorter_columns() {
        let mut form = RunForm::default();

        form.selected = 20;
        form.move_column_left();
        assert_eq!(form.selected, 10);

        form.move_column_right();
        assert_eq!(form.selected, 19);
    }

    #[test]
    fn run_form_renders_workspace_with_preview_and_selected_field_panels() {
        let mut app = test_app();
        app.screen = Screen::RunForm;

        let lines = render_lines(&app, 140, 40);

        assert!(lines.iter().any(|line| line.contains("Paths & Scope")));
        assert!(lines.iter().any(|line| line.contains("Extraction")));
        assert!(lines.iter().any(|line| line.contains("Taxonomy")));
        assert!(lines.iter().any(|line| line.contains("Placement")));
        assert!(lines.iter().any(|line| line.contains("LLM & API")));
        assert!(lines.iter().any(|line| line.contains("Run")));
        assert!(lines.iter().any(|line| line.contains("Run Setup")));
        assert!(lines.iter().any(|line| line.contains("Launch Preview")));
        assert!(lines.iter().any(|line| line.contains("Selected Field")));
        assert!(lines.iter().any(|line| line.contains("Input Folder")));
    }

    #[test]
    fn run_form_scrolls_to_keep_selected_field_visible() {
        let mut app = test_app();
        app.screen = Screen::RunForm;
        app.run_form.selected = 20;

        let lines = render_lines(&app, 140, 24);

        assert!(lines.iter().any(|line| line.contains("Run")));
        assert!(lines.iter().any(|line| line.contains("Quiet Mode")));
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

        let runtime = test_runtime();
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)))
            .expect("enter should commit edit");

        assert_eq!(app.run_form.input, "papers");
        assert!(app.overlay.is_none());
    }

    #[test]
    fn edit_overlay_escape_closes_editor_without_leaving_form() {
        let mut app = test_app();
        app.screen = Screen::RunForm;
        app.run_form.selected = 0;
        let original_input = app.run_form.input.clone();
        app.overlay = Some(Overlay::EditField {
            label: "Input".to_string(),
            buffer: "papers".to_string(),
        });

        let runtime = test_runtime();
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)))
            .expect("escape should close editor");

        assert!(matches!(app.screen, Screen::RunForm));
        assert!(app.overlay.is_none());
        assert_eq!(app.run_form.input, original_input);
    }

    #[test]
    fn edit_overlay_renders_input_box_and_places_cursor_at_buffer_end() {
        let mut app = test_app();
        app.screen = Screen::RunForm;
        app.overlay = Some(Overlay::EditField {
            label: "Input".to_string(),
            buffer: "papers".to_string(),
        });

        let mut terminal = render_app(&app, 80, 24);
        let buffer = terminal.backend().buffer();
        let area = buffer.area();
        let lines = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(lines.iter().any(|line| line.contains("Edit Field")));
        assert!(lines.iter().any(|line| line.contains("┌Input")));
        assert!(lines.iter().any(|line| line.contains("papers")));
        terminal
            .backend_mut()
            .assert_cursor_position(Position::new(20, 10));
    }

    #[test]
    fn escape_on_home_opens_quit_confirmation() {
        let mut app = test_app();
        app.screen = Screen::Home;

        let runtime = test_runtime();
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)))
            .expect("escape should open quit confirmation");

        assert!(matches!(
            app.overlay,
            Some(Overlay::Confirm {
                action: super::model::ConfirmAction::Quit,
                ..
            })
        ));
        assert!(!app.should_quit);
    }

    #[test]
    fn selecting_quit_from_home_requires_confirmation() {
        let mut app = test_app();
        app.screen = Screen::Home;
        app.home_index = 2;

        let runtime = test_runtime();
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)))
            .expect("enter should open quit confirmation");

        assert!(matches!(app.screen, Screen::Home));
        assert!(matches!(
            app.overlay,
            Some(Overlay::Confirm {
                action: super::model::ConfirmAction::Quit,
                ..
            })
        ));
        assert!(!app.should_quit);
    }

    #[test]
    fn confirming_quit_sets_should_quit() {
        let mut app = test_app();
        app.screen = Screen::Home;
        app.overlay = Some(Overlay::Confirm {
            title: "Quit".to_string(),
            message: "Quit SortYourPapers?".to_string(),
            action: super::model::ConfirmAction::Quit,
        });

        let runtime = test_runtime();
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)))
            .expect("enter should confirm quit");

        assert!(app.should_quit);
        assert!(app.overlay.is_none());
    }

    #[test]
    fn q_no_longer_quits_from_home() {
        let mut app = test_app();
        app.screen = Screen::Home;

        let runtime = test_runtime();
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)))
            .expect("q should be ignored");

        assert!(!app.should_quit);
        assert!(app.overlay.is_none());
    }

    #[test]
    fn run_form_analysis_blocks_missing_input_directory() {
        let temp = tempdir().expect("tempdir should build");
        let mut form = RunForm::default();
        form.input = temp.path().join("missing-input").display().to_string();
        form.output = temp.path().join("sorted").display().to_string();

        let analysis = form.analysis();

        assert!(analysis.has_errors());
        let issue = analysis
            .field_issue(0)
            .expect("missing input should create a field issue");
        assert_eq!(issue.severity, ValidationSeverity::Error);
    }

    #[test]
    fn run_form_analysis_allows_missing_output_directory_as_info() {
        let temp = tempdir().expect("tempdir should build");
        let mut form = RunForm::default();
        form.input = temp.path().display().to_string();
        form.output = temp.path().join("sorted").display().to_string();

        let analysis = form.analysis();

        assert!(!analysis.has_errors());
        let issue = analysis
            .field_issue(1)
            .expect("missing output should surface as a note");
        assert_eq!(issue.severity, ValidationSeverity::Info);
        assert!(analysis.config.is_some());
    }

    #[test]
    fn run_form_launch_with_errors_opens_notice_instead_of_starting() {
        let temp = tempdir().expect("tempdir should build");
        let mut app = test_app();
        app.screen = Screen::RunForm;
        app.run_form.input = temp.path().join("missing-input").display().to_string();
        app.run_form.output = temp.path().join("sorted").display().to_string();

        let runtime = test_runtime();
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)))
            .expect("run hotkey should be handled");

        assert!(matches!(app.screen, Screen::RunForm));
        assert!(matches!(app.overlay, Some(Overlay::Notice { .. })));
    }

    #[test]
    fn operation_screen_renders_tabs_and_success_actions() {
        let mut app = test_app();
        app.operation.state = OperationState::Success;
        app.operation.summary = "run completed".to_string();

        let lines = render_lines(&app, 100, 24);

        assert!(lines.iter().any(|line| line.contains("Views")));
        assert!(lines.iter().any(|line| line.contains("1 Summary")));
        assert!(lines.iter().any(|line| line.contains("2 Logs")));
        assert!(lines.iter().any(|line| line.contains("3 Taxonomy")));
        assert!(lines.iter().any(|line| line.contains("4 Report")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Next actions: 3 Taxonomy, 4 Report, s Sessions."))
        );
    }

    #[test]
    fn sessions_screen_renders_filters_and_preview_panels() {
        let mut app = test_app();
        app.screen = Screen::Sessions;
        let run = sample_session_run("run-complete", Some(RunStage::Completed));
        app.session_view.replace_runs_for_tests(vec![run.clone()]);
        app.session_view.set_status_for_tests(
            &run.run_id,
            sample_session_status(true, false, false),
        );
        app.session_view
            .set_selected_details_for_tests(sample_session_details(run));

        let lines = render_lines(&app, 120, 32);

        assert!(lines.iter().any(|line| line.contains("Filters")));
        assert!(lines.iter().any(|line| line.contains("1 All")));
        assert!(lines.iter().any(|line| line.contains("Preview Tabs")));
        assert!(lines.iter().any(|line| line.contains("provider: gemini / gemini-3-flash-preview")));
    }

    #[test]
    fn sessions_filter_hotkeys_change_visible_runs() {
        let mut app = test_app();
        app.screen = Screen::Sessions;
        let completed = sample_session_run("run-complete", Some(RunStage::Completed));
        let incomplete = sample_session_run("run-open", Some(RunStage::ExtractText));
        app.session_view
            .replace_runs_for_tests(vec![completed.clone(), incomplete.clone()]);
        app.session_view.set_status_for_tests(
            &completed.run_id,
            sample_session_status(true, false, false),
        );
        app.session_view.set_status_for_tests(
            &incomplete.run_id,
            sample_session_status(false, true, true),
        );
        app.session_view
            .set_selected_details_for_tests(sample_session_details(completed.clone()));
        let runtime = test_runtime();

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE)))
            .expect("completed filter should be handled");
        let completed_lines = render_lines(&app, 120, 32);
        assert!(completed_lines.iter().any(|line| line.contains("run-complete")));
        assert!(!completed_lines.iter().any(|line| line.contains("run-open")));

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE)))
            .expect("incomplete filter should be handled");
        let incomplete_lines = render_lines(&app, 120, 32);
        assert!(incomplete_lines.iter().any(|line| line.contains("run-open")));
        assert!(!incomplete_lines.iter().any(|line| line.contains("run-complete")));
    }

    #[test]
    fn sessions_preview_tabs_switch_and_scroll() {
        let mut app = test_app();
        app.screen = Screen::Sessions;
        let run = sample_session_run("run-complete", Some(RunStage::Completed));
        app.session_view.replace_runs_for_tests(vec![run.clone()]);
        app.session_view.set_status_for_tests(
            &run.run_id,
            sample_session_status(true, false, false),
        );
        app.session_view
            .set_selected_details_for_tests(sample_session_details(run));
        let runtime = test_runtime();

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)))
            .expect("tab should switch to report preview");
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)))
            .expect("page down should scroll report preview");

        assert_eq!(app.session_view.preview_tab_label_for_tests(), "Report");
        assert!(app.session_view.preview_scroll_for_tests() > 0);
    }

    #[test]
    fn stage_status_and_alert_events_feed_summary_panels() {
        let mut app = test_app();
        let (backend_tx, backend_rx) = mpsc::channel();
        app.backend_rx = backend_rx;

        backend_tx
            .send(BackendEvent::StageStatus {
                stage: "extract-keywords".to_string(),
                message: "running keyword extraction".to_string(),
            })
            .expect("stage status should send");
        backend_tx
            .send(BackendEvent::Alert {
                severity: AlertSeverity::Warning,
                label: "KEYWORDS".to_string(),
                message: "batch 2/4 retry 2/3".to_string(),
            })
            .expect("alert should send");

        app.drain_backend_events();
        assert_eq!(app.operation.alerts.len(), 1);
        assert_eq!(app.operation.alerts[0].label, "KEYWORDS");

        let lines = render_lines(&app, 100, 24);

        assert!(lines.iter().any(|line| line.contains("extract-keywords")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("running keyword extraction"))
        );
    }

    #[test]
    fn operation_tab_hotkeys_switch_views_and_scroll_logs() {
        let mut app = test_app();
        app.logs = (0..40)
            .map(|index| format!("log line {index:02}"))
            .collect::<VecDeque<_>>();
        let runtime = test_runtime();

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE)))
            .expect("tab hotkey should switch to logs");
        assert_eq!(app.operation.active_tab, OperationTab::Logs);

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)))
            .expect("j should scroll logs");
        assert_eq!(app.operation.log_scroll, 1);

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT)))
            .expect("G should jump to log end");
        assert_eq!(app.operation.log_scroll, 39);

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE)))
            .expect("g should jump to log start");
        assert_eq!(app.operation.log_scroll, 0);
    }

    #[test]
    fn operation_logs_tab_renders_scrolled_content() {
        let mut app = test_app();
        app.operation.active_tab = OperationTab::Logs;
        app.operation.log_scroll = 12;
        app.logs = (0..30)
            .map(|index| format!("log line {index:02}"))
            .collect::<VecDeque<_>>();

        let lines = render_lines(&app, 100, 24);

        assert!(!lines.iter().any(|line| line.contains("log line 00")));
        assert!(lines.iter().any(|line| line.contains("log line 12")));
    }

    #[test]
    fn operation_tabs_show_empty_states_for_missing_taxonomy_and_report() {
        let mut app = test_app();

        app.operation.active_tab = OperationTab::Taxonomy;
        let taxonomy_lines = render_lines(&app, 100, 24);
        assert!(
            taxonomy_lines
                .iter()
                .any(|line| line.contains("Taxonomy not available yet."))
        );

        app.operation.active_tab = OperationTab::Report;
        let report_lines = render_lines(&app, 100, 24);
        assert!(
            report_lines
                .iter()
                .any(|line| line.contains("Report not available yet."))
        );
    }

    #[test]
    fn escape_from_operation_returns_to_origin_screen() {
        let mut app = test_app();
        app.operation.state = OperationState::Success;
        app.operation.origin = Screen::RunForm;
        let runtime = test_runtime();

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)))
            .expect("escape should return to origin");

        assert!(matches!(app.screen, Screen::RunForm));
    }

    #[test]
    fn operation_sessions_shortcut_requires_idle_state() {
        let mut app = test_app();
        let runtime = test_runtime();

        app.operation.state = OperationState::Running;
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)))
            .expect("running operation should ignore sessions shortcut");
        assert!(matches!(app.screen, Screen::Operation));

        app.operation.state = OperationState::Success;
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)))
            .expect("idle operation should open sessions");
        assert!(matches!(app.screen, Screen::Sessions));
    }
}
