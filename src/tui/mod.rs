mod app;
mod backend;
mod extract;
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
    forms::{ExtractForm, RunForm, UiVerbosity},
    model::{OperationDetail, OperationView, Overlay, ProgressEntry, Screen},
    session_view::SessionView,
};

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

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::mpsc};

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    use crate::{papers::placement::PlacementMode, papers::taxonomy::TaxonomyMode};

    use super::{
        App, BackendEvent, ExtractForm, OperationDetail, OperationView, Overlay, ProgressEntry,
        RunForm, Screen, SessionView, UiVerbosity,
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

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)))
            .expect("escape should close editor");

        assert!(matches!(app.screen, Screen::RunForm));
        assert!(app.overlay.is_none());
        assert_eq!(app.run_form.input, original_input);
    }
}
