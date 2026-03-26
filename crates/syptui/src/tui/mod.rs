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
pub(crate) mod theme;
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

use crate::{error::Result, prefs, terminal::install_backend};

use self::{app::App, backend::TuiBackend};

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

    let prefs = prefs::load_tui_preferences()?;
    let mut app = App::new(backend_rx, op_rx, op_tx, debug_tui, prefs.theme)?;
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
mod tests;
