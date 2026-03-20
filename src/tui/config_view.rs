use std::{env, fs};

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Color, Frame, Line, Modifier, Span, Style},
    widgets::{Block, Borders, ListItem, Paragraph, Wrap},
};

use crate::{config, error::Result};

use super::ui_widgets::{render_scrolled_paragraph, render_selectable_list};

const ENV_KEYS: [(&str, bool); 14] = [
    ("SYP_INPUT", false),
    ("SYP_OUTPUT", false),
    ("SYP_RECURSIVE", false),
    ("SYP_MAX_FILE_SIZE_MB", false),
    ("SYP_PAGE_CUTOFF", false),
    ("SYP_PDF_EXTRACT_WORKERS", false),
    ("SYP_CATEGORY_DEPTH", false),
    ("SYP_TAXONOMY_MODE", false),
    ("SYP_TAXONOMY_BATCH_SIZE", false),
    ("SYP_PLACEMENT_BATCH_SIZE", false),
    ("SYP_PLACEMENT_MODE", false),
    ("SYP_LLM_PROVIDER", false),
    ("SYP_LLM_BASE_URL", false),
    ("SYP_API_KEY", true),
];

#[derive(Clone, Copy)]
pub(super) enum ConfigAction {
    Refresh,
    Init,
    ForceInit,
}

impl ConfigAction {
    fn label(self) -> &'static str {
        match self {
            Self::Refresh => "Refresh Diagnostics",
            Self::Init => "Write Default Config",
            Self::ForceInit => "Overwrite Config",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Self::Refresh => "Reload file, cache, and environment diagnostics.",
            Self::Init => "Create the default XDG config if it does not already exist.",
            Self::ForceInit => "Rewrite the XDG config file even if one already exists.",
        }
    }
}

pub(crate) struct ConfigView {
    selected_action: usize,
    detail_scroll: u16,
    lines: Vec<String>,
    status_message: String,
}

impl Default for ConfigView {
    fn default() -> Self {
        let mut view = Self {
            selected_action: 0,
            detail_scroll: 0,
            lines: Vec::new(),
            status_message: "Press Enter to run the selected config action.".to_string(),
        };
        let _ = view.refresh();
        view
    }
}

impl ConfigView {
    const ACTIONS: [ConfigAction; 3] = [
        ConfigAction::Refresh,
        ConfigAction::Init,
        ConfigAction::ForceInit,
    ];

    pub(super) fn refresh(&mut self) -> Result<()> {
        self.lines = self.build_lines()?;
        self.detail_scroll = 0;
        Ok(())
    }

    pub(super) fn move_selection(&mut self, delta: i8) {
        if delta < 0 {
            self.selected_action = self.selected_action.saturating_sub(1);
        } else {
            self.selected_action = (self.selected_action + 1).min(Self::ACTIONS.len() - 1);
        }
    }

    pub(super) fn selected_action(&self) -> ConfigAction {
        Self::ACTIONS[self.selected_action]
    }

    pub(super) fn scroll(&mut self, delta: isize) {
        self.detail_scroll = (self.detail_scroll as isize + delta).max(0) as u16;
    }

    pub(super) fn set_status(&mut self, status_message: String) {
        self.status_message = status_message;
    }

    pub(crate) fn draw(&self, frame: &mut Frame, area: Rect) {
        let chunks = if area.width < 110 {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(14), Constraint::Min(0)])
                .split(area)
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(34), Constraint::Percentage(66)])
                .split(area)
        };
        let left = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(8), Constraint::Min(8)])
            .split(chunks[0]);

        let action_items = Self::ACTIONS
            .iter()
            .map(|action| ListItem::new(action.label()))
            .collect::<Vec<_>>();
        render_selectable_list(
            frame,
            left[0],
            Block::default().title("Actions").borders(Borders::ALL),
            action_items,
            Some(self.selected_action),
        );

        let help_lines = vec![
            Line::from(Span::styled(
                "Selected Action",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(self.selected_action().help()),
            Line::from(""),
            Line::from(self.status_message.clone()),
            Line::from(""),
            Line::from("Enter run action"),
            Line::from("g refresh diagnostics"),
            Line::from("PgUp/PgDn scroll"),
            Line::from("Esc back"),
        ];
        frame.render_widget(
            Paragraph::new(help_lines)
                .wrap(Wrap { trim: false })
                .block(Block::default().title("Status").borders(Borders::ALL)),
            left[1],
        );

        render_scrolled_paragraph(
            frame,
            chunks[1],
            Block::default().title("Diagnostics").borders(Borders::ALL),
            self.lines
                .iter()
                .cloned()
                .map(Line::from)
                .collect::<Vec<_>>(),
            self.detail_scroll,
            true,
        );
    }

    fn build_lines(&self) -> Result<Vec<String>> {
        let mut lines = Vec::new();

        lines.push("XDG".to_string());
        lines.push(String::new());

        match config::xdg_config_path() {
            Some(path) => {
                lines.push(format!("config_path: {}", path.display()));
                lines.push(format!(
                    "config_status: {}",
                    if path.exists() { "present" } else { "missing" }
                ));

                if path.exists() {
                    let raw = fs::read_to_string(&path)?;
                    let metadata = fs::metadata(&path)?;
                    lines.push(format!("config_size_bytes: {}", metadata.len()));
                    lines.push(String::new());
                    lines.push("Current Config".to_string());
                    lines.push(String::new());
                    lines.extend(raw.lines().map(ToOwned::to_owned));
                } else {
                    lines.push(
                        "config_hint: press Enter on \"Write Default Config\" to create it."
                            .to_string(),
                    );
                    lines.push(String::new());
                    lines.push("Default Config Template".to_string());
                    lines.push(String::new());
                    lines.extend(config::default_config_toml().lines().map(ToOwned::to_owned));
                }
            }
            None => {
                lines.push("config_path: unavailable".to_string());
                lines
                    .push("config_status: XDG config directory could not be resolved.".to_string());
            }
        }

        lines.push(String::new());
        lines.push("Cache".to_string());
        lines.push(String::new());
        lines.push(match config::xdg_cache_dir() {
            Some(path) => format!("cache_dir: {}", path.display()),
            None => "cache_dir: unavailable".to_string(),
        });

        lines.push(String::new());
        lines.push("Environment Overrides".to_string());
        lines.push(String::new());
        for (key, secret) in ENV_KEYS {
            let value = env::var(key).ok();
            let rendered = match (secret, value) {
                (_, None) => "<unset>".to_string(),
                (true, Some(raw)) if raw.trim().is_empty() => "<set but empty>".to_string(),
                (true, Some(_)) => "<set>".to_string(),
                (false, Some(raw)) => raw,
            };
            lines.push(format!("{key}={rendered}"));
        }

        Ok(lines)
    }
}
