mod app;
mod backend;
mod extract;
mod forms;
mod input;
mod model;
mod render;
mod session_view;
mod taxonomy_review;
mod taxonomy_tree;
mod ui_widgets;

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
    forms::{ExtractForm, RunForm, UiVerbosity, ValidationSeverity},
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
    use std::{collections::VecDeque, sync::mpsc, time::Duration};

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend, layout::Position, style::Color};
    use tempfile::tempdir;

    use crate::{
        config::{ApiKeySource, AppConfig},
        papers::placement::PlacementMode,
        papers::taxonomy::{CategoryTree, TaxonomyMode},
        report::{FileAction, PlanAction, RunReport},
        session::{
            RunStage, RunSummary, SessionConfigSummary, SessionDetails, SessionStatusSummary,
        },
        terminal::{AlertSeverity, InspectReviewPrompt, InspectReviewRequest},
    };

    use super::{
        App, BackendEvent, ExtractForm, OperationDetail, OperationState, OperationView, Overlay,
        ProgressEntry, RunForm, Screen, SessionView, UiVerbosity, ValidationSeverity,
        model::{OperationTab, StageTiming},
        render::stage_timing_bars,
        taxonomy_review::{
            PendingReviewReply, ReviewIteration, ReviewPane, ReviewPhase, TaxonomyReviewView,
        },
        taxonomy_tree::reset_state_for_categories,
    };

    fn test_app() -> App {
        let (_backend_tx, backend_rx) = mpsc::channel();
        let (op_tx, op_rx) = mpsc::channel();
        App {
            screen: Screen::Operation,
            home_index: 0,
            run_form: RunForm::default(),
            extract_form: ExtractForm::default(),
            session_view: SessionView::default(),
            overlay: None,
            taxonomy_review: None,
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

    fn overlay_width(lines: &[String], title: &str) -> usize {
        let line = lines
            .iter()
            .find(|line| line.contains(title) && line.contains('┌') && line.contains('┐'))
            .expect("overlay title should be rendered");
        let chars = line.chars().collect::<Vec<_>>();
        let title_byte_index = line.find(title).expect("overlay title should be rendered");
        let title_index = line[..title_byte_index].chars().count();
        let start = chars[..=title_index]
            .iter()
            .rposition(|ch| *ch == '┌')
            .expect("overlay should have a top border");
        let end = chars[title_index..]
            .iter()
            .position(|ch| *ch == '┐')
            .map(|offset| title_index + offset)
            .expect("overlay should have a top border");
        end - start + 1
    }

    fn header_colored_cell_count(app: &App, width: u16, height: u16) -> usize {
        let terminal = render_app(app, width, height);
        let buffer = terminal.backend().buffer();
        let area = buffer.area();
        let header_end = (0..area.height)
            .find(|y| (0..area.width).any(|x| buffer[(x, *y)].symbol() == "└"))
            .map(|y| y + 1)
            .unwrap_or(area.height.min(6));

        (0..header_end)
            .flat_map(|y| (0..area.width).map(move |x| buffer[(x, y)].bg))
            .filter(|bg| *bg != Color::Reset)
            .count()
    }

    fn contains_scrollbar_glyph(lines: &[String]) -> bool {
        lines
            .iter()
            .any(|line| line.contains('║') || line.contains('█'))
    }

    fn find_text_position(lines: &[String], text: &str) -> Option<(usize, usize)> {
        lines.iter().enumerate().find_map(|(y, line)| {
            line.find(text)
                .map(|byte_x| (line[..byte_x].chars().count(), y))
        })
    }

    fn contains_symbol_with_fg(
        app: &App,
        width: u16,
        height: u16,
        symbol: &str,
        fg: Color,
    ) -> bool {
        let terminal = render_app(app, width, height);
        let buffer = terminal.backend().buffer();
        let area = buffer.area();

        (0..area.height).any(|y| {
            (0..area.width).any(|x| {
                let cell = &buffer[(x, y)];
                cell.symbol() == symbol && cell.fg == fg
            })
        })
    }

    fn row_contains_symbol_with_style(
        app: &App,
        width: u16,
        height: u16,
        row_text: &str,
        symbol: &str,
        fg: Color,
        bg: Color,
    ) -> bool {
        let lines = render_lines(app, width, height);
        let Some((_, y)) = find_text_position(&lines, row_text) else {
            return false;
        };

        let terminal = render_app(app, width, height);
        let buffer = terminal.backend().buffer();
        let area = buffer.area();
        if y >= usize::from(area.height) {
            return false;
        }

        (0..area.width).any(|x| {
            let cell = &buffer[(x, y as u16)];
            cell.symbol() == symbol && cell.fg == fg && cell.bg == bg
        })
    }

    fn text_starts_with_fg(app: &App, width: u16, height: u16, text: &str, fg: Color) -> bool {
        let lines = render_lines(app, width, height);
        let Some((x, y)) = find_text_position(&lines, text) else {
            return false;
        };

        let terminal = render_app(app, width, height);
        let buffer = terminal.backend().buffer();
        let area = buffer.area();
        if x >= usize::from(area.width) || y >= usize::from(area.height) {
            return false;
        }

        let cell = &buffer[(x as u16, y as u16)];
        cell.symbol()
            == text
                .chars()
                .next()
                .map(|ch| ch.to_string())
                .as_deref()
                .unwrap_or("")
            && cell.fg == fg
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

    fn sample_taxonomy_categories() -> Vec<CategoryTree> {
        vec![CategoryTree {
            name: "AI".to_string(),
            children: vec![CategoryTree {
                name: "Vision".to_string(),
                children: vec![],
            }],
        }]
    }

    fn sample_rearrangeable_taxonomy_categories() -> Vec<CategoryTree> {
        vec![
            CategoryTree {
                name: "AI".to_string(),
                children: vec![CategoryTree {
                    name: "Vision".to_string(),
                    children: vec![],
                }],
            },
            CategoryTree {
                name: "Systems".to_string(),
                children: vec![],
            },
        ]
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
        assert!(!form.editable(16));
        assert!(!form.editable(20));
        assert!(!form.editable(21));
        assert!(!form.editable(22));
        assert!(form.editable(14));
        assert!(form.editable(18));
    }

    #[test]
    fn run_form_can_be_hydrated_from_saved_config() {
        let config = AppConfig {
            input: "/tmp/papers".into(),
            output: "/tmp/sorted".into(),
            recursive: true,
            max_file_size_mb: 32,
            page_cutoff: 4,
            pdf_extract_workers: 6,
            category_depth: 3,
            taxonomy_mode: TaxonomyMode::Global,
            taxonomy_batch_size: 9,
            use_current_folder_tree: true,
            placement_batch_size: 12,
            placement_mode: PlacementMode::AllowNew,
            rebuild: true,
            dry_run: false,
            llm_provider: crate::llm::LlmProvider::Openai,
            llm_model: "gpt-test".to_string(),
            llm_base_url: Some("http://localhost:1234/v1".to_string()),
            api_key: Some(ApiKeySource::Env("OPENAI_API_KEY".to_string())),
            keyword_batch_size: 21,
            batch_start_delay_ms: 250,
            subcategories_suggestion_number: 7,
            verbose: true,
            debug: false,
            quiet: true,
        };

        let form = RunForm::from_config(&config);

        assert_eq!(form.input, "/tmp/papers");
        assert_eq!(form.output, "/tmp/sorted");
        assert!(form.recursive);
        assert_eq!(form.value(14), "gpt-test");
        assert_eq!(form.value(15), "http://localhost:1234/v1");
        assert_eq!(form.value(16), "env");
        assert_eq!(form.value(17), "OPENAI_API_KEY");
        assert_eq!(form.value(20), "verbose");
        assert_eq!(form.value(21), "yes");
        assert_eq!(form.value(22), "yes");
        assert!(form.apply);
        assert!(form.rebuild);
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

        form.selected = 16;
        form.toggle_selected();
        assert_eq!(form.value(16), "command");

        form.selected = 20;
        form.toggle_selected();
        assert!(matches!(form.verbosity, UiVerbosity::Verbose));

        form.selected = 11;
        form.toggle_selected();
        assert!(form.rebuild);

        form.selected = 21;
        form.toggle_selected();
        assert!(form.quiet);

        form.selected = 22;
        form.toggle_selected();
        assert!(form.use_current_folder_tree);
    }

    #[test]
    fn run_form_navigation_skips_hidden_output_fields() {
        let mut form = RunForm::default();

        form.selected = 17;
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
        assert_eq!(form.selected, 17);

        form.move_column_left();
        assert_eq!(form.selected, 18);

        form.move_column_left();
        assert_eq!(form.selected, 4);
    }

    #[test]
    fn run_form_column_navigation_clamps_to_shorter_columns() {
        let mut form = RunForm::default();

        form.selected = 21;
        form.move_column_left();
        assert_eq!(form.selected, 10);

        form.move_column_right();
        assert_eq!(form.selected, 20);
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
        assert!(lines.iter().any(|line| line.contains("STATUS")));
        assert!(lines.iter().any(|line| line.contains("Pipeline")));
        assert!(lines.iter().any(|line| line.contains("Launch")));
        assert!(lines.iter().any(|line| line.contains("Selected Field")));
        assert!(lines.iter().any(|line| line.contains("Input Folder")));
        assert!(lines.iter().any(|line| line.contains("API Key Source")));
    }

    #[test]
    fn run_form_selected_run_button_shows_launch_copy() {
        let mut app = test_app();
        app.screen = Screen::RunForm;
        app.run_form.selected = RunForm::RUN_BUTTON_INDEX;

        let lines = render_lines(&app, 140, 40);

        assert!(lines.iter().any(|line| line.contains("Run Button")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Press Enter, Space, or r to launch."))
        );
    }

    #[test]
    fn run_form_scrolls_to_keep_selected_field_visible() {
        let mut app = test_app();
        app.screen = Screen::RunForm;
        app.run_form.selected = 21;

        let lines = render_lines(&app, 140, 24);

        assert!(lines.iter().any(|line| line.contains("Run")));
        assert!(lines.iter().any(|line| line.contains("Quiet Mode")));
    }

    #[test]
    fn run_form_header_includes_save_shortcut() {
        let mut app = test_app();
        app.screen = Screen::RunForm;

        let lines = render_lines(&app, 140, 24);

        assert!(lines.iter().any(|line| line.contains("save")));
    }

    #[test]
    fn home_screen_lists_remaining_actions() {
        let mut app = test_app();
        app.screen = Screen::Home;

        let lines = render_lines(&app, 120, 28);

        assert!(lines.iter().any(|line| line.contains("Run Papers")));
        assert!(lines.iter().any(|line| line.contains("Extract Text")));
        assert!(lines.iter().any(|line| line.contains("Sessions")));
        assert!(lines.iter().any(|line| line.contains("Quit")));
        assert!(!lines.iter().any(|line| line.contains("Config")));
        assert!(!lines.iter().any(|line| line.contains("Debug Tools")));
    }

    #[test]
    fn home_screen_stacks_panels_on_narrow_width() {
        let mut app = test_app();
        app.screen = Screen::Home;

        let lines = render_lines(&app, 80, 28);
        let (_, actions_y) =
            find_text_position(&lines, "Actions").expect("actions panel should render");
        let (_, overview_y) =
            find_text_position(&lines, "Overview").expect("overview panel should render");

        assert!(overview_y > actions_y + 2);
    }

    #[test]
    fn run_form_selected_field_uses_structured_description_layout() {
        let mut app = test_app();
        app.screen = Screen::RunForm;
        app.run_form.selected = 0;

        let lines = render_lines(&app, 140, 36);

        assert!(lines.iter().any(|line| line.contains("Description")));
        assert!(lines.iter().any(|line| line.contains("Current")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Source folder scanned for candidate PDFs."))
        );
        assert!(!lines.iter().any(|line| line.contains("Provider Notes")));
        assert!(!lines.iter().any(|line| line.contains("Controls")));
    }

    #[test]
    fn run_form_renders_api_key_source_and_value_fields() {
        let mut app = test_app();
        app.screen = Screen::RunForm;

        let lines = render_lines(&app, 140, 40);

        assert!(lines.iter().any(|line| line.contains("API Key Source")));
        assert!(lines.iter().any(|line| line.contains("API Key Value")));
    }

    #[test]
    fn run_form_stacks_side_panels_on_narrow_width() {
        let mut app = test_app();
        app.screen = Screen::RunForm;

        let lines = render_lines(&app, 100, 40);
        let (_, setup_y) =
            find_text_position(&lines, "Run Setup").expect("run setup panel should render");
        let (_, preview_y) = find_text_position(&lines, "Launch Preview")
            .expect("launch preview panel should render");
        let (_, selected_y) = find_text_position(&lines, "Selected Field")
            .expect("selected field panel should render");

        assert!(preview_y > setup_y + 2);
        assert!(selected_y > preview_y + 2);
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
    fn operation_detail_text_renders_custom_extract_preview_title() {
        let mut app = test_app();
        app.operation.active_tab = OperationTab::Taxonomy;
        app.operation.detail = OperationDetail::Text {
            title: "Extract Preview".to_string(),
            lines: vec![
                "=== /tmp/paper.pdf ===".to_string(),
                "--- raw ---".to_string(),
            ],
            empty_message: "No extract output".to_string(),
        };

        let lines = render_lines(&app, 120, 24);

        assert!(lines.iter().any(|line| line.contains("Extract Preview")));
        assert!(lines.iter().any(|line| line.contains("/tmp/paper.pdf")));
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
    fn run_form_enter_opens_path_picker_for_folder_fields() {
        let mut app = test_app();
        app.screen = Screen::RunForm;
        app.run_form.selected = 0;

        let runtime = test_runtime();
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)))
            .expect("enter should open path picker");

        assert!(matches!(
            app.overlay,
            Some(Overlay::SelectPath {
                ref label,
                ref buffer,
                ..
            }) if label == "Input Folder" && buffer == &app.run_form.input
        ));
    }

    #[test]
    fn path_overlay_enter_commits_value_without_reopening_editor() {
        let mut app = test_app();
        app.screen = Screen::RunForm;
        app.run_form.selected = 0;
        app.overlay = Some(Overlay::SelectPath {
            label: "Input Folder".to_string(),
            buffer: "papers".to_string(),
            directories: vec!["papers".to_string(), "reports".to_string()],
            selected: 0,
        });

        let runtime = test_runtime();
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)))
            .expect("enter should commit selected path");

        assert_eq!(app.run_form.input, "papers");
        assert!(app.overlay.is_none());
    }

    #[test]
    fn path_overlay_renders_relative_folder_list() {
        let mut app = test_app();
        app.screen = Screen::RunForm;
        app.overlay = Some(Overlay::SelectPath {
            label: "Input Folder".to_string(),
            buffer: "papers".to_string(),
            directories: vec!["papers/ml".to_string(), "papers/nlp".to_string()],
            selected: 1,
        });

        let lines = render_lines(&app, 100, 24);

        assert!(lines.iter().any(|line| line.contains("Choose Folder")));
        assert!(lines.iter().any(|line| line.contains("Folders")));
        assert!(lines.iter().any(|line| line.contains("papers/ml")));
        assert!(lines.iter().any(|line| line.contains("papers/nlp")));
    }

    #[test]
    fn home_overview_no_longer_duplicates_key_help() {
        let mut app = test_app();
        app.screen = Screen::Home;

        let lines = render_lines(&app, 80, 24);

        assert!(!lines.iter().any(|line| line.contains("Keys:")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Quit: exit after confirmation."))
        );
    }

    #[test]
    fn header_renders_colored_key_hint_chips() {
        let mut app = test_app();
        app.screen = Screen::Home;

        let lines = render_lines(&app, 80, 24);
        let colored_cells = header_colored_cell_count(&app, 80, 24);

        assert!(lines.iter().any(|line| line.contains("↑/↓: move")));
        assert!(lines.iter().any(|line| line.contains("Enter: open")));
        assert!(colored_cells > 0, "expected colored footer chips");
    }

    #[test]
    fn key_hints_render_in_header_not_bottom_panel() {
        let mut app = test_app();
        app.screen = Screen::Home;

        let lines = render_lines(&app, 80, 24);
        let header_line = lines
            .iter()
            .find(|line| line.contains("Home") && line.contains("[idle]"))
            .expect("header line should exist");

        assert!(header_line.contains("↑/↓: move"));
        assert!(!lines.iter().skip(3).any(|line| line.contains("↑/↓: move")));
    }

    #[test]
    fn run_form_header_describes_enter_as_edit_or_run() {
        let mut app = test_app();
        app.screen = Screen::RunForm;

        let lines = render_lines(&app, 100, 24);

        assert!(lines.iter().any(|line| line.contains("Enter: edit/run")));
    }

    #[test]
    fn run_form_header_capitalizes_space_key_name() {
        let mut app = test_app();
        app.screen = Screen::RunForm;

        let lines = render_lines(&app, 140, 24);

        assert!(lines.iter().any(|line| line.contains("Space: toggle")));
        assert!(!lines.iter().any(|line| line.contains("space: toggle")));
    }

    #[test]
    fn confirm_overlay_renders_compact_popup() {
        let mut app = test_app();
        app.screen = Screen::Home;
        app.overlay = Some(Overlay::Confirm {
            title: "Quit".to_string(),
            message: "Quit SortYourPapers?".to_string(),
            action: super::model::ConfirmAction::Quit,
        });

        let lines = render_lines(&app, 80, 24);
        let width = overlay_width(&lines, "Quit");

        assert!(width < 40, "quit overlay width was {width}");
    }

    #[test]
    fn confirm_overlay_adds_inner_padding() {
        let mut app = test_app();
        app.screen = Screen::Home;
        app.overlay = Some(Overlay::Confirm {
            title: "Quit".to_string(),
            message: "Quit SortYourPapers?".to_string(),
            action: super::model::ConfirmAction::Quit,
        });

        let lines = render_lines(&app, 80, 24);
        let title_y = lines
            .iter()
            .position(|line| line.contains("Quit") && line.contains('┌') && line.contains('┐'))
            .expect("overlay title should be rendered");
        let title_line = &lines[title_y];
        let chars = title_line.chars().collect::<Vec<_>>();
        let title_byte_index = title_line.find("Quit").expect("title should be rendered");
        let title_x = title_line[..title_byte_index].chars().count();
        let start = chars[..=title_x]
            .iter()
            .rposition(|ch| *ch == '┌')
            .expect("overlay should have a top border");
        let end = chars[title_x..]
            .iter()
            .position(|ch| *ch == '┐')
            .map(|offset| title_x + offset)
            .expect("overlay should have a top border");
        let padded_line = &lines[title_y + 1];
        let padded_chars = padded_line.chars().collect::<Vec<_>>();

        assert_eq!(padded_chars[start], '│');
        assert_eq!(padded_chars[end], '│');
        assert!(padded_chars[start + 1..end].iter().all(|ch| *ch == ' '));
    }

    #[test]
    fn notice_overlay_renders_compact_popup() {
        let mut app = test_app();
        app.screen = Screen::RunForm;
        app.overlay = Some(Overlay::Notice {
            title: "Validation".to_string(),
            message: "The run configuration is not ready yet.".to_string(),
        });

        let lines = render_lines(&app, 80, 24);
        let width = overlay_width(&lines, "Validation");

        assert!(width < 50, "notice overlay width was {width}");
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
    fn second_escape_on_home_dismisses_quit_confirmation() {
        let mut app = test_app();
        app.screen = Screen::Home;

        let runtime = test_runtime();
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)))
            .expect("first escape should open quit confirmation");
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)))
            .expect("second escape should dismiss quit confirmation");

        assert!(matches!(app.screen, Screen::Home));
        assert!(app.overlay.is_none());
        assert!(!app.should_quit);
    }

    #[test]
    fn selecting_quit_from_home_requires_confirmation() {
        let mut app = test_app();
        app.screen = Screen::Home;
        app.home_index = app.home_actions().len().saturating_sub(1);

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
    fn confirm_overlay_colors_enter_and_y_blue_and_esc_red() {
        let mut app = test_app();
        app.overlay = Some(Overlay::Confirm {
            title: "Confirm".to_string(),
            message: "Apply this change?".to_string(),
            action: super::model::ConfirmAction::Quit,
        });

        assert!(text_starts_with_fg(&app, 80, 24, "Enter", Color::Blue));
        assert!(text_starts_with_fg(&app, 80, 24, "y confirm", Color::Blue));
        assert!(text_starts_with_fg(&app, 80, 24, "Esc cancel", Color::Red));
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
    fn run_form_analysis_warns_when_api_key_env_is_missing() {
        let temp = tempdir().expect("tempdir should build");
        let mut form = RunForm::default();
        form.input = temp.path().display().to_string();
        form.output = temp.path().join("sorted").display().to_string();

        form.selected = 16;
        form.toggle_selected();
        form.toggle_selected();
        form.selected = 17;
        form.apply_edit("SYP_MISSING_API_KEY_FOR_TEST".to_string())
            .expect("api key env name should apply");

        let analysis = form.analysis();

        let issue = analysis
            .field_issue(17)
            .expect("missing env should create a field issue");
        assert_eq!(issue.severity, ValidationSeverity::Warning);
        assert!(issue.message.contains("Environment variable"));
    }

    #[test]
    fn run_form_analysis_accepts_present_api_key_env() {
        let temp = tempdir().expect("tempdir should build");
        let mut form = RunForm::default();
        form.input = temp.path().display().to_string();
        form.output = temp.path().join("sorted").display().to_string();

        form.selected = 16;
        form.toggle_selected();
        form.toggle_selected();
        form.selected = 17;
        form.apply_edit("PATH".to_string())
            .expect("api key env name should apply");

        let analysis = form.analysis();

        assert!(
            analysis
                .field_issue(17)
                .is_none_or(|issue| !issue.message.contains("Environment variable"))
        );
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
    fn run_button_enter_with_errors_opens_notice_instead_of_starting() {
        let temp = tempdir().expect("tempdir should build");
        let mut app = test_app();
        app.screen = Screen::RunForm;
        app.run_form.input = temp.path().join("missing-input").display().to_string();
        app.run_form.output = temp.path().join("sorted").display().to_string();
        app.run_form.selected = RunForm::RUN_BUTTON_INDEX;

        let runtime = test_runtime();
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)))
            .expect("run button should be handled");

        assert!(matches!(app.screen, Screen::RunForm));
        assert!(matches!(app.overlay, Some(Overlay::Notice { .. })));
    }

    #[test]
    fn run_form_save_hotkey_opens_config_confirmation() {
        let temp = tempdir().expect("tempdir should build");
        let mut app = test_app();
        app.screen = Screen::RunForm;
        app.run_form.input = temp.path().display().to_string();
        app.run_form.output = temp.path().join("sorted").display().to_string();

        let runtime = test_runtime();
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)))
            .expect("save hotkey should be handled");

        assert!(matches!(
            app.overlay,
            Some(Overlay::Confirm {
                action: super::model::ConfirmAction::SaveRunConfig(_),
                ..
            })
        ));
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
        assert!(lines.iter().any(|line| line.contains("4 Planned Actions")));
        assert!(
            lines
                .iter()
                .any(|line| line
                    .contains("Next actions: 3 Taxonomy, 4 Planned Actions, s Sessions."))
        );
    }

    #[test]
    fn sessions_screen_renders_filters_and_preview_panels() {
        let mut app = test_app();
        app.screen = Screen::Sessions;
        let run = sample_session_run("run-complete", Some(RunStage::Completed));
        app.session_view.replace_runs_for_tests(vec![run.clone()]);
        app.session_view
            .set_status_for_tests(&run.run_id, sample_session_status(true, false, false));
        app.session_view
            .set_selected_details_for_tests(sample_session_details(run));

        let lines = render_lines(&app, 160, 32);

        assert!(lines.iter().any(|line| line.contains("Filters")));
        assert!(lines.iter().any(|line| line.contains("1 All")));
        assert!(lines.iter().any(|line| line.contains("Preview Tabs")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("provider: gemini / gemini-3-flash-preview"))
        );
    }

    #[test]
    fn sessions_filter_hotkeys_change_visible_runs() {
        let mut app = test_app();
        app.screen = Screen::Sessions;
        let completed = sample_session_run("run-complete", Some(RunStage::Completed));
        let incomplete = sample_session_run("run-open", Some(RunStage::ExtractText));
        app.session_view
            .replace_runs_for_tests(vec![completed.clone(), incomplete.clone()]);
        app.session_view
            .set_status_for_tests(&completed.run_id, sample_session_status(true, false, false));
        app.session_view
            .set_status_for_tests(&incomplete.run_id, sample_session_status(false, true, true));
        app.session_view
            .set_selected_details_for_tests(sample_session_details(completed.clone()));
        let runtime = test_runtime();

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE)))
            .expect("completed filter should be handled");
        let completed_lines = render_lines(&app, 120, 32);
        assert!(
            completed_lines
                .iter()
                .any(|line| line.contains("run-complete"))
        );
        assert!(!completed_lines.iter().any(|line| line.contains("run-open")));

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE)))
            .expect("incomplete filter should be handled");
        let incomplete_lines = render_lines(&app, 120, 32);
        assert!(
            incomplete_lines
                .iter()
                .any(|line| line.contains("run-open"))
        );
        assert!(
            !incomplete_lines
                .iter()
                .any(|line| line.contains("run-complete"))
        );
    }

    #[test]
    fn sessions_preview_tabs_switch_and_scroll() {
        let mut app = test_app();
        app.screen = Screen::Sessions;
        let run = sample_session_run("run-complete", Some(RunStage::Completed));
        app.session_view.replace_runs_for_tests(vec![run.clone()]);
        app.session_view
            .set_status_for_tests(&run.run_id, sample_session_status(true, false, false));
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
    fn sessions_screen_capital_c_opens_clear_all_confirmation() {
        let mut app = test_app();
        app.screen = Screen::Sessions;
        let runtime = test_runtime();

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('C'), KeyModifiers::SHIFT)))
            .expect("capital C should open clear-all confirmation");

        assert!(matches!(
            app.overlay,
            Some(Overlay::Confirm {
                action: super::model::ConfirmAction::ClearAll,
                ..
            })
        ));
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
    fn operation_summary_removes_pinned_alerts_and_failure_panel() {
        let mut app = test_app();
        app.operation.state = OperationState::Failure;
        app.operation.summary = "run completed with one or more failures".to_string();
        app.operation
            .alerts
            .push_back(super::model::OperationAlert::new(
                AlertSeverity::Error,
                "EXTRACT".to_string(),
                "paper.pdf: extraction failed".to_string(),
            ));

        let lines = render_lines(&app, 100, 24);

        assert!(!lines.iter().any(|line| line.contains("Pinned Alerts")));
        assert!(!lines.iter().any(|line| line.contains("┌Failure")));
        assert!(lines.iter().any(|line| line.contains("Run Summary")));
        assert!(lines.iter().any(|line| line.contains("Elasped Time")));
    }

    #[test]
    fn operation_summary_lists_stage_timings_on_separate_lines() {
        let mut app = test_app();
        app.operation.summary = "run completed".to_string();
        app.operation.stage_timings = vec![
            StageTiming {
                stage: "discover-input".to_string(),
                elapsed: Duration::from_millis(500),
            },
            StageTiming {
                stage: "extract-text".to_string(),
                elapsed: Duration::from_secs(2),
            },
        ];

        let lines = render_lines(&app, 100, 24);

        assert!(lines.iter().any(|line| line.contains("1. discover-input")));
        assert!(lines.iter().any(|line| line.contains("500.0ms")));
        assert!(lines.iter().any(|line| line.contains("2. extract-text")));
        assert!(lines.iter().any(|line| line.contains("2.000s")));
    }

    #[test]
    fn stage_timing_progress_uses_max_elapsed_scaled_by_one_point_five() {
        let bars = stage_timing_bars(vec![
            super::render::StageTimingSnapshot {
                stage: "discover-input".to_string(),
                elapsed: Duration::from_secs(2),
                running: false,
            },
            super::render::StageTimingSnapshot {
                stage: "inspect-output".to_string(),
                elapsed: Duration::from_secs(10),
                running: false,
            },
            super::render::StageTimingSnapshot {
                stage: "extract-text".to_string(),
                elapsed: Duration::from_secs(4),
                running: false,
            },
        ]);

        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].stage, "1. discover-input");
        assert_eq!(bars[0].elapsed_label, "2.000s");
        assert_eq!(bars[1].stage, "2. extract-text");
        assert_eq!(bars[1].elapsed_label, "4.000s");
        assert!((bars[0].ratio - (2.0 / 6.0)).abs() < 0.000_001);
        assert!((bars[1].ratio - (4.0 / 6.0)).abs() < 0.000_001);
    }

    #[test]
    fn operation_summary_panel_includes_report_summary_information() {
        let mut app = test_app();
        let mut report = RunReport::new(true);
        report.scanned = 12;
        report.processed = 9;
        report.skipped = 2;
        report.failed = 1;
        report.actions = vec![PlanAction {
            source: "/tmp/in/paper.pdf".into(),
            destination: "/tmp/out/topic/paper.pdf".into(),
            action: FileAction::Move,
        }];
        app.last_report = Some(report);
        app.operation.summary = "run completed".to_string();

        let lines = render_lines(&app, 120, 28);

        assert!(
            !lines
                .iter()
                .any(|line| line.contains("SortYourPapers Summary"))
        );
        assert!(lines.iter().any(|line| line.contains("scanned 12")));
        assert!(lines.iter().any(|line| line.contains("processed 9")));
    }

    #[test]
    fn operation_elapsed_time_panel_no_longer_repeats_summary_text() {
        let mut app = test_app();
        app.operation.summary = "run completed".to_string();
        app.operation.stage_timings = vec![StageTiming {
            stage: "discover-input".to_string(),
            elapsed: Duration::from_millis(500),
        }];

        let lines = render_lines(&app, 100, 24);

        assert!(lines.iter().any(|line| line.contains("Elasped Time")));
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("summary: run completed"))
        );
        assert!(row_contains_symbol_with_style(
            &app,
            100,
            24,
            "1. discover-input",
            "█",
            Color::LightCyan,
            Color::Black,
        ));
    }

    #[test]
    fn operation_report_tab_keeps_planned_actions_only() {
        let mut app = test_app();
        let mut report = RunReport::new(true);
        report.scanned = 12;
        report.processed = 9;
        report.actions = vec![PlanAction {
            source: "/tmp/in/paper.pdf".into(),
            destination: "/tmp/out/topic/paper.pdf".into(),
            action: FileAction::Move,
        }];
        app.last_report = Some(report);
        app.operation.active_tab = super::model::OperationTab::Report;

        let lines = render_lines(&app, 120, 28);

        assert!(lines.iter().any(|line| line.contains("Planned Actions")));
        assert!(lines.iter().any(|line| line.contains("/tmp/in/paper.pdf")));
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("SortYourPapers Summary"))
        );
        assert!(!lines.iter().any(|line| line.contains("scanned 12")));
    }

    #[test]
    fn inspect_review_event_opens_dedicated_taxonomy_review_screen() {
        let mut app = test_app();
        let (backend_tx, backend_rx) = mpsc::channel();
        let (reply_tx, _reply_rx) = mpsc::channel();
        app.backend_rx = backend_rx;

        backend_tx
            .send(BackendEvent::PromptInspectReview {
                categories: sample_taxonomy_categories(),
                reply: reply_tx,
            })
            .expect("prompt event should send");

        app.drain_backend_events();

        assert!(matches!(app.screen, Screen::TaxonomyReview));
        let review = app
            .taxonomy_review
            .as_ref()
            .expect("taxonomy review should be open");
        assert_eq!(review.accepted_categories[0].name, "AI");
        assert!(review.history.is_empty());
        assert_eq!(review.phase, ReviewPhase::Drafting);
        assert_eq!(review.focused_pane, ReviewPane::Suggestion);
    }

    #[test]
    fn taxonomy_review_submits_suggestion_and_enters_waiting_phase() {
        let mut app = test_app();
        let (reply_tx, reply_rx) = mpsc::channel();
        app.screen = Screen::TaxonomyReview;
        app.taxonomy_review = Some(TaxonomyReviewView::new(
            sample_taxonomy_categories(),
            reply_tx,
        ));
        let runtime = test_runtime();

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)))
            .expect("s should start editing");
        for character in "Merge vision".chars() {
            runtime
                .block_on(
                    app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE)),
                )
                .expect("typing should update the draft");
        }
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)))
            .expect("enter should submit the suggestion");

        let review = app
            .taxonomy_review
            .as_ref()
            .expect("review should remain open while waiting");
        assert_eq!(review.phase, ReviewPhase::WaitingForModel);
        assert_eq!(
            review.last_submitted_suggestion.as_deref(),
            Some("Merge vision")
        );
        assert!(!review.editing);
        assert_eq!(
            reply_rx.recv().expect("reply should send"),
            Ok(InspectReviewPrompt::Suggest(
                InspectReviewRequest::from_user_suggestion("Merge vision".to_string()),
            ))
        );
    }

    #[test]
    fn taxonomy_review_candidate_tree_event_populates_candidate_and_history() {
        let mut app = test_app();
        let (backend_tx, backend_rx) = mpsc::channel();
        app.backend_rx = backend_rx;
        let (inspect_tx, _inspect_rx) = mpsc::channel();
        let mut review = TaxonomyReviewView::new(sample_taxonomy_categories(), inspect_tx);
        review.phase = ReviewPhase::WaitingForModel;
        review.last_submitted_suggestion = Some("Merge vision".to_string());
        app.taxonomy_review = Some(review);

        backend_tx
            .send(BackendEvent::CategoryTree(vec![CategoryTree {
                name: "AI (merged)".to_string(),
                children: vec![],
            }]))
            .expect("candidate event should send");

        app.drain_backend_events();

        let review = app
            .taxonomy_review
            .as_ref()
            .expect("review should still be open");
        assert_eq!(
            review
                .candidate_categories
                .as_ref()
                .expect("candidate taxonomy")
                .first()
                .expect("candidate root")
                .name,
            "AI (merged)"
        );
        assert_eq!(review.history.len(), 1);
        assert_eq!(review.history[0].suggestion, "Merge vision");
        assert_eq!(review.history[0].accepted_categories[0].name, "AI");
        assert_eq!(
            review.history[0].suggested_categories[0].name,
            "AI (merged)"
        );
        assert_eq!(review.history_selection, 1);
    }

    #[test]
    fn continue_prompt_switches_taxonomy_review_to_candidate_decision() {
        let mut app = test_app();
        let (backend_tx, backend_rx) = mpsc::channel();
        let (inspect_tx, _inspect_rx) = mpsc::channel();
        let (continue_tx, _continue_rx) = mpsc::channel();
        app.backend_rx = backend_rx;
        app.taxonomy_review = Some(TaxonomyReviewView::new(
            sample_taxonomy_categories(),
            inspect_tx,
        ));

        backend_tx
            .send(BackendEvent::PromptContinueImproving { reply: continue_tx })
            .expect("continue prompt should send");

        app.drain_backend_events();

        let review = app
            .taxonomy_review
            .as_ref()
            .expect("review should still exist");
        assert_eq!(review.phase, ReviewPhase::PostSuggestionDecision);
        assert_eq!(review.focused_pane, ReviewPane::IterationTaxonomy);
    }

    #[test]
    fn drafting_taxonomy_review_accept_sends_accept_and_returns_to_operation() {
        let mut app = test_app();
        let (reply_tx, reply_rx) = mpsc::channel();
        app.screen = Screen::TaxonomyReview;
        app.taxonomy_review = Some(TaxonomyReviewView::new(
            sample_taxonomy_categories(),
            reply_tx,
        ));
        let runtime = test_runtime();

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)))
            .expect("a should open confirmation for accepting the baseline taxonomy");

        assert!(matches!(
            app.overlay,
            Some(Overlay::Confirm {
                action: super::model::ConfirmAction::AcceptTaxonomyBaseline,
                ..
            })
        ));
        assert!(matches!(app.screen, Screen::TaxonomyReview));

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)))
            .expect("enter should confirm accepting the baseline taxonomy");

        assert!(matches!(app.screen, Screen::Operation));
        assert!(app.taxonomy_review.is_none());
        assert_eq!(
            reply_rx.recv().expect("reply should send"),
            Ok(InspectReviewPrompt::Accept)
        );
    }

    #[test]
    fn drafting_taxonomy_review_accept_confirmation_can_be_cancelled() {
        let mut app = test_app();
        let (reply_tx, _reply_rx) = mpsc::channel();
        app.screen = Screen::TaxonomyReview;
        app.taxonomy_review = Some(TaxonomyReviewView::new(
            sample_taxonomy_categories(),
            reply_tx,
        ));
        let runtime = test_runtime();

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)))
            .expect("a should open confirmation for accepting the baseline taxonomy");
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)))
            .expect("esc should dismiss the baseline accept confirmation");

        assert!(app.overlay.is_none());
        assert!(matches!(app.screen, Screen::TaxonomyReview));
        assert!(app.taxonomy_review.is_some());
    }

    #[test]
    fn taxonomy_review_d_marks_selected_taxonomy_node_red() {
        let mut app = test_app();
        let (reply_tx, _reply_rx) = mpsc::channel();
        let mut review = TaxonomyReviewView::new(sample_taxonomy_categories(), reply_tx);
        review.focused_pane = ReviewPane::IterationTaxonomy;
        app.screen = Screen::TaxonomyReview;
        app.taxonomy_review = Some(review);
        let runtime = test_runtime();
        let _ = render_lines(&app, 120, 32);

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)))
            .expect("down should select the first taxonomy node");
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE)))
            .expect("d should mark the selected taxonomy node");
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)))
            .expect("up should move selection away from the marked node");

        let lines = render_lines(&app, 120, 32);
        assert!(lines.iter().any(|line| line.contains("x AI")));
        assert!(contains_symbol_with_fg(&app, 120, 32, "x", Color::Red));
    }

    #[test]
    fn drafting_taxonomy_review_accept_with_removal_sends_improvement_request() {
        let mut app = test_app();
        let (reply_tx, reply_rx) = mpsc::channel();
        let mut review = TaxonomyReviewView::new(sample_taxonomy_categories(), reply_tx);
        review.focused_pane = ReviewPane::IterationTaxonomy;
        app.screen = Screen::TaxonomyReview;
        app.taxonomy_review = Some(review);
        let runtime = test_runtime();
        let _ = render_lines(&app, 120, 32);

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)))
            .expect("down should select the first taxonomy node");
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE)))
            .expect("d should mark the selected taxonomy node");
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)))
            .expect("a should submit the removal request");

        let review = app
            .taxonomy_review
            .as_ref()
            .expect("review should remain open while waiting for the model");
        assert!(matches!(app.screen, Screen::TaxonomyReview));
        assert_eq!(review.phase, ReviewPhase::WaitingForModel);
        assert_eq!(
            review.last_submitted_suggestion.as_deref(),
            Some("remove: AI")
        );
        assert_eq!(
            reply_rx.recv().expect("reply should send"),
            Ok(InspectReviewPrompt::Suggest(InspectReviewRequest::new(
                None,
                vec!["AI".to_string()],
            )))
        );
    }

    #[test]
    fn taxonomy_review_capital_d_marks_selected_subtree_red() {
        let mut app = test_app();
        let (reply_tx, reply_rx) = mpsc::channel();
        let mut review = TaxonomyReviewView::new(sample_taxonomy_categories(), reply_tx);
        review.focused_pane = ReviewPane::IterationTaxonomy;
        app.screen = Screen::TaxonomyReview;
        app.taxonomy_review = Some(review);
        let runtime = test_runtime();
        let _ = render_lines(&app, 120, 32);

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)))
            .expect("down should select the first taxonomy node");
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT)))
            .expect("D should mark the selected taxonomy subtree");
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)))
            .expect("up should move selection away from the marked subtree");

        let lines = render_lines(&app, 120, 32);
        assert!(lines.iter().any(|line| line.contains("x AI")));
        assert!(lines.iter().any(|line| line.contains("x Vision")));
        assert!(contains_symbol_with_fg(&app, 120, 32, "x", Color::Red));

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)))
            .expect("a should submit the subtree removal request");
        assert_eq!(
            reply_rx.recv().expect("reply should send"),
            Ok(InspectReviewPrompt::Suggest(InspectReviewRequest::new(
                None,
                vec!["AI".to_string()],
            )))
        );
    }

    #[test]
    fn candidate_accept_sends_finish_and_returns_to_operation() {
        let mut app = test_app();
        let (inspect_tx, _inspect_rx) = mpsc::channel();
        let (continue_tx, continue_rx) = mpsc::channel();
        let mut review = TaxonomyReviewView::new(sample_taxonomy_categories(), inspect_tx);
        review.phase = ReviewPhase::PostSuggestionDecision;
        review.candidate_categories = Some(vec![CategoryTree {
            name: "AI (candidate)".to_string(),
            children: vec![],
        }]);
        review.pending_reply = Some(PendingReviewReply::Continue(continue_tx));
        app.screen = Screen::TaxonomyReview;
        app.taxonomy_review = Some(review);
        let runtime = test_runtime();

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)))
            .expect("a should open confirmation for accepting the candidate taxonomy");

        assert!(matches!(
            app.overlay,
            Some(Overlay::Confirm {
                action: super::model::ConfirmAction::AcceptTaxonomyCandidate,
                ..
            })
        ));
        assert!(matches!(app.screen, Screen::TaxonomyReview));

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)))
            .expect("enter should confirm accepting the candidate taxonomy");

        assert!(matches!(app.screen, Screen::Operation));
        assert!(app.taxonomy_review.is_none());
        assert_eq!(continue_rx.recv().expect("reply should send"), Ok(false));
    }

    #[test]
    fn candidate_accept_confirmation_can_be_cancelled() {
        let mut app = test_app();
        let (inspect_tx, _inspect_rx) = mpsc::channel();
        let (continue_tx, _continue_rx) = mpsc::channel();
        let mut review = TaxonomyReviewView::new(sample_taxonomy_categories(), inspect_tx);
        review.phase = ReviewPhase::PostSuggestionDecision;
        review.candidate_categories = Some(vec![CategoryTree {
            name: "AI (candidate)".to_string(),
            children: vec![],
        }]);
        review.pending_reply = Some(PendingReviewReply::Continue(continue_tx));
        app.screen = Screen::TaxonomyReview;
        app.taxonomy_review = Some(review);
        let runtime = test_runtime();

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)))
            .expect("a should open confirmation for accepting the candidate taxonomy");
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)))
            .expect("esc should dismiss the confirmation");

        assert!(app.overlay.is_none());
        assert!(matches!(app.screen, Screen::TaxonomyReview));
        assert!(app.taxonomy_review.is_some());
    }

    #[test]
    fn iterate_again_promotes_candidate_and_keeps_review_open() {
        let mut app = test_app();
        let (inspect_tx, _inspect_rx) = mpsc::channel();
        let (continue_tx, continue_rx) = mpsc::channel();
        let mut review = TaxonomyReviewView::new(sample_taxonomy_categories(), inspect_tx);
        review.phase = ReviewPhase::PostSuggestionDecision;
        review.candidate_categories = Some(vec![CategoryTree {
            name: "AI (candidate)".to_string(),
            children: vec![],
        }]);
        review.pending_reply = Some(PendingReviewReply::Continue(continue_tx));
        app.screen = Screen::TaxonomyReview;
        app.taxonomy_review = Some(review);
        let runtime = test_runtime();

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)))
            .expect("i should continue the review loop");

        let review = app
            .taxonomy_review
            .as_ref()
            .expect("review should stay open");
        assert!(matches!(app.screen, Screen::TaxonomyReview));
        assert_eq!(review.accepted_categories[0].name, "AI (candidate)");
        assert!(review.candidate_categories.is_none());
        assert_eq!(review.phase, ReviewPhase::Drafting);
        assert_eq!(continue_rx.recv().expect("reply should send"), Ok(true));
    }

    #[test]
    fn taxonomy_review_renders_dedicated_workspace_panels() {
        let mut app = test_app();
        let (inspect_tx, _inspect_rx) = mpsc::channel();
        let mut review = TaxonomyReviewView::new(sample_taxonomy_categories(), inspect_tx);
        review.phase = ReviewPhase::PostSuggestionDecision;
        review.history = vec![ReviewIteration {
            number: 1,
            suggestion: "Merge vision".to_string(),
            accepted_categories: sample_taxonomy_categories(),
            suggested_categories: vec![CategoryTree {
                name: "AI (candidate)".to_string(),
                children: vec![],
            }],
        }];
        review.history_selection = 1;
        review.last_submitted_suggestion = Some("Merge vision".to_string());
        review.candidate_categories = Some(vec![CategoryTree {
            name: "AI (candidate)".to_string(),
            children: vec![],
        }]);
        app.screen = Screen::TaxonomyReview;
        app.taxonomy_review = Some(review);

        let lines = render_lines(&app, 120, 32);

        assert!(lines.iter().any(|line| line.contains("Taxonomy Review")));
        assert!(lines.iter().any(|line| line.contains("Iteration Taxonomy")));
        assert!(lines.iter().any(|line| line.contains("Suggested Taxonomy")));
        assert!(!lines.iter().any(|line| line.contains("Accepted Taxonomy")));
        assert!(lines.iter().any(|line| line.contains("Suggestion")));
        assert!(lines.iter().any(|line| line.contains("History")));
        assert!(lines.iter().any(|line| line.contains("cut")));
        assert!(lines.iter().any(|line| line.contains("paste")));
    }

    #[test]
    fn taxonomy_review_places_iteration_taxonomy_panel_to_the_right() {
        let mut app = test_app();
        let (inspect_tx, _inspect_rx) = mpsc::channel();
        let mut review = TaxonomyReviewView::new(sample_taxonomy_categories(), inspect_tx);
        review.phase = ReviewPhase::PostSuggestionDecision;
        review.candidate_categories = Some(vec![CategoryTree {
            name: "AI (candidate)".to_string(),
            children: vec![],
        }]);
        app.screen = Screen::TaxonomyReview;
        app.taxonomy_review = Some(review);

        let lines = render_lines(&app, 120, 32);
        let suggestion_x = lines
            .iter()
            .find_map(|line| line.find("Suggestion"))
            .expect("suggestion title should be present");
        let iteration_x = lines
            .iter()
            .find_map(|line| line.find("Iteration Taxonomy"))
            .expect("iteration taxonomy title should be present");

        assert!(iteration_x > suggestion_x);
    }

    #[test]
    fn taxonomy_review_space_toggles_iteration_tree_visibility() {
        let mut app = test_app();
        let (inspect_tx, _inspect_rx) = mpsc::channel();
        let mut review = TaxonomyReviewView::new(sample_taxonomy_categories(), inspect_tx);
        review.focused_pane = ReviewPane::IterationTaxonomy;
        app.screen = Screen::TaxonomyReview;
        app.taxonomy_review = Some(review);
        let runtime = test_runtime();

        let expanded_lines = render_lines(&app, 120, 32);
        assert!(expanded_lines.iter().any(|line| line.contains("Vision")));

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)))
            .expect("space should fold the selected iteration taxonomy node");
        let collapsed_lines = render_lines(&app, 120, 32);
        assert!(!collapsed_lines.iter().any(|line| line.contains("Vision")));

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)))
            .expect("space should unfold the selected iteration taxonomy node");
        let reopened_lines = render_lines(&app, 120, 32);
        assert!(reopened_lines.iter().any(|line| line.contains("Vision")));
    }

    #[test]
    fn taxonomy_review_x_and_p_move_selected_entry_in_accepted_taxonomy() {
        let mut app = test_app();
        let (inspect_tx, _inspect_rx) = mpsc::channel();
        let mut review =
            TaxonomyReviewView::new(sample_rearrangeable_taxonomy_categories(), inspect_tx);
        review.focused_pane = ReviewPane::IterationTaxonomy;
        let _ = review
            .iteration_tree_state
            .borrow_mut()
            .select(vec![0, 0, 0]);
        app.screen = Screen::TaxonomyReview;
        app.taxonomy_review = Some(review);
        let runtime = test_runtime();

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)))
            .expect("x should cut the selected taxonomy node");
        let _ = app
            .taxonomy_review
            .as_mut()
            .expect("review should remain open")
            .iteration_tree_state
            .borrow_mut()
            .select(vec![0, 1]);
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE)))
            .expect("p should paste the cut taxonomy node");

        let review = app
            .taxonomy_review
            .as_ref()
            .expect("review should remain open");
        assert!(review.cut_entry.is_none());
        assert_eq!(review.accepted_categories.len(), 2);
        assert!(review.accepted_categories[0].children.is_empty());
        assert_eq!(review.accepted_categories[1].name, "Systems");
        assert_eq!(review.accepted_categories[1].children.len(), 1);
        assert_eq!(review.accepted_categories[1].children[0].name, "Vision");
    }

    #[test]
    fn taxonomy_review_x_and_p_move_selected_entry_in_candidate_taxonomy() {
        let mut app = test_app();
        let (inspect_tx, _inspect_rx) = mpsc::channel();
        let mut review = TaxonomyReviewView::new(sample_taxonomy_categories(), inspect_tx);
        review.phase = ReviewPhase::PostSuggestionDecision;
        review.focused_pane = ReviewPane::IterationTaxonomy;
        review.candidate_categories = Some(sample_rearrangeable_taxonomy_categories());
        let _ = review
            .iteration_tree_state
            .borrow_mut()
            .select(vec![0, 0, 0]);
        app.screen = Screen::TaxonomyReview;
        app.taxonomy_review = Some(review);
        let runtime = test_runtime();

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)))
            .expect("x should cut the selected candidate taxonomy node");
        let _ = app
            .taxonomy_review
            .as_mut()
            .expect("review should remain open")
            .iteration_tree_state
            .borrow_mut()
            .select(vec![0, 1]);
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE)))
            .expect("p should paste the cut candidate taxonomy node");

        let review = app
            .taxonomy_review
            .as_ref()
            .expect("review should remain open");
        assert_eq!(review.accepted_categories[0].name, "AI");
        let candidate = review
            .candidate_categories
            .as_ref()
            .expect("candidate taxonomy should remain available");
        assert_eq!(candidate.len(), 2);
        assert!(candidate[0].children.is_empty());
        assert_eq!(candidate[1].children.len(), 1);
        assert_eq!(candidate[1].children[0].name, "Vision");
    }

    #[test]
    fn history_panel_selects_iterations_for_iteration_taxonomy_panel() {
        let mut app = test_app();
        let (inspect_tx, _inspect_rx) = mpsc::channel();
        let mut review = TaxonomyReviewView::new(sample_taxonomy_categories(), inspect_tx);
        review.focused_pane = ReviewPane::History;
        review.history = vec![
            ReviewIteration {
                number: 1,
                suggestion: "Merge vision".to_string(),
                accepted_categories: sample_taxonomy_categories(),
                suggested_categories: vec![CategoryTree {
                    name: "AI (merged)".to_string(),
                    children: vec![],
                }],
            },
            ReviewIteration {
                number: 2,
                suggestion: "Split speech".to_string(),
                accepted_categories: vec![CategoryTree {
                    name: "AI (merged)".to_string(),
                    children: vec![],
                }],
                suggested_categories: vec![CategoryTree {
                    name: "Speech".to_string(),
                    children: vec![],
                }],
            },
        ];
        app.screen = Screen::TaxonomyReview;
        app.taxonomy_review = Some(review);
        let runtime = test_runtime();

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)))
            .expect("down should select the first saved iteration");
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)))
            .expect("down should select the second saved iteration");

        let review = app
            .taxonomy_review
            .as_ref()
            .expect("review should stay open");
        assert_eq!(review.history_selection, 2);

        let lines = render_lines(&app, 120, 32);
        assert!(lines.iter().any(|line| line.contains("Iteration 2")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Suggestion: Split speech"))
        );
        assert!(lines.iter().any(|line| line.contains("Speech")));
    }

    #[test]
    fn taxonomy_review_focus_order_matches_panel_order() {
        let mut app = test_app();
        let (inspect_tx, _inspect_rx) = mpsc::channel();
        app.screen = Screen::TaxonomyReview;
        app.taxonomy_review = Some(TaxonomyReviewView::new(
            sample_taxonomy_categories(),
            inspect_tx,
        ));
        let runtime = test_runtime();

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)))
            .expect("tab should move to history");
        assert_eq!(
            app.taxonomy_review
                .as_ref()
                .expect("review should stay open")
                .focused_pane,
            ReviewPane::History
        );

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)))
            .expect("tab should move to iteration taxonomy");
        assert_eq!(
            app.taxonomy_review
                .as_ref()
                .expect("review should stay open")
                .focused_pane,
            ReviewPane::IterationTaxonomy
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
        assert_eq!(app.operation.log_scroll, u16::MAX);

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE)))
            .expect("g should jump to log start");
        assert_eq!(app.operation.log_scroll, 0);
    }

    #[test]
    fn operation_logs_can_scroll_past_raw_line_count_when_wrapped() {
        let mut app = test_app();
        app.operation.active_tab = OperationTab::Logs;
        app.logs = vec![format!("{} {}", "wrapped".repeat(24), "content".repeat(24))]
            .into_iter()
            .collect();
        let runtime = test_runtime();

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)))
            .expect("j should scroll wrapped logs");
        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)))
            .expect("j should keep scrolling wrapped logs");

        assert!(app.operation.log_scroll >= 2);
    }

    #[test]
    fn operation_header_wraps_shortcuts_on_narrow_width() {
        let mut app = test_app();
        app.screen = Screen::Operation;
        app.operation.state = OperationState::Running;

        let lines = render_lines(&app, 70, 24);
        let (_, title_y) =
            find_text_position(&lines, "Operation").expect("operation title should render");
        let (_, hint_y) = find_text_position(&lines, "Tab/h/l: switch")
            .expect("wrapped shortcut hint should render");

        assert!(hint_y > title_y);
        assert!(hint_y < 5);
    }

    #[test]
    fn operation_taxonomy_space_toggles_tree_visibility() {
        let mut app = test_app();
        let categories = sample_taxonomy_categories();
        app.screen = Screen::Operation;
        app.operation.active_tab = OperationTab::Taxonomy;
        app.operation.detail = OperationDetail::Tree(categories.clone());
        reset_state_for_categories(
            &mut app.operation.taxonomy_tree_state.borrow_mut(),
            &categories,
        );
        let runtime = test_runtime();

        let expanded_lines = render_lines(&app, 100, 24);
        assert!(expanded_lines.iter().any(|line| line.contains("Vision")));

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)))
            .expect("space should fold the selected taxonomy node");
        let collapsed_lines = render_lines(&app, 100, 24);
        assert!(!collapsed_lines.iter().any(|line| line.contains("Vision")));

        runtime
            .block_on(app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)))
            .expect("space should unfold the selected taxonomy node");
        let reopened_lines = render_lines(&app, 100, 24);
        assert!(reopened_lines.iter().any(|line| line.contains("Vision")));
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
    fn operation_logs_tab_renders_scrollbar_for_overflowing_content() {
        let mut app = test_app();
        app.operation.active_tab = OperationTab::Logs;
        app.logs = (0..80)
            .map(|index| format!("log line {index:02}"))
            .collect::<VecDeque<_>>();

        let lines = render_lines(&app, 100, 20);

        assert!(contains_scrollbar_glyph(&lines));
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
                .any(|line| line.contains("Planned actions are not available yet."))
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
