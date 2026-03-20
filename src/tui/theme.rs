use ratatui::{
    prelude::{Color, Modifier, Style},
    widgets::{Block, Borders, Padding, block::Title},
};
use serde::{Deserialize, Serialize};

use super::model::OperationState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum UiThemeName {
    #[default]
    Dark,
    Light,
}

impl UiThemeName {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }

    pub(super) fn palette(self) -> ThemePalette {
        match self {
            Self::Dark => ThemePalette::dark(),
            Self::Light => ThemePalette::light(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ThemePalette {
    pub(super) app_bg: Color,
    pub(super) panel_fg: Color,
    pub(super) panel_bg: Color,
    pub(super) border: Color,
    pub(super) muted: Color,
    pub(super) selection_fg: Color,
    pub(super) selection_bg: Color,
    pub(super) scrollbar_thumb: Color,
    pub(super) focus_border: Color,
    pub(super) input_bg: Color,
    pub(super) input_fg: Color,
    pub(super) info: Color,
    pub(super) success: Color,
    pub(super) warning: Color,
    pub(super) error: Color,
    pub(super) accent: Color,
    pub(super) chip_fg: Color,
    pub(super) chip_palette: [Color; 6],
}

impl ThemePalette {
    const fn dark() -> Self {
        Self {
            app_bg: Color::Rgb(8, 12, 18),
            panel_fg: Color::Rgb(222, 229, 238),
            panel_bg: Color::Rgb(18, 24, 33),
            border: Color::Rgb(70, 84, 102),
            muted: Color::Rgb(133, 148, 166),
            selection_fg: Color::Rgb(9, 12, 17),
            selection_bg: Color::Rgb(111, 214, 184),
            scrollbar_thumb: Color::Rgb(111, 214, 184),
            focus_border: Color::Rgb(255, 214, 102),
            input_bg: Color::Rgb(28, 36, 48),
            input_fg: Color::Rgb(245, 247, 250),
            info: Color::Rgb(121, 197, 255),
            success: Color::Rgb(111, 214, 184),
            warning: Color::Rgb(255, 214, 102),
            error: Color::Rgb(255, 133, 127),
            accent: Color::Rgb(170, 148, 255),
            chip_fg: Color::Black,
            chip_palette: [
                Color::LightCyan,
                Color::LightGreen,
                Color::LightYellow,
                Color::LightMagenta,
                Color::LightBlue,
                Color::LightRed,
            ],
        }
    }

    const fn light() -> Self {
        Self {
            app_bg: Color::Rgb(237, 241, 247),
            panel_fg: Color::Rgb(31, 42, 54),
            panel_bg: Color::Rgb(252, 252, 250),
            border: Color::Rgb(139, 151, 168),
            muted: Color::Rgb(102, 116, 134),
            selection_fg: Color::Rgb(255, 255, 255),
            selection_bg: Color::Rgb(36, 114, 145),
            scrollbar_thumb: Color::Rgb(36, 114, 145),
            focus_border: Color::Rgb(193, 122, 33),
            input_bg: Color::Rgb(242, 246, 251),
            input_fg: Color::Rgb(21, 31, 43),
            info: Color::Blue,
            success: Color::Green,
            warning: Color::Rgb(184, 116, 0),
            error: Color::Red,
            accent: Color::Magenta,
            chip_fg: Color::Black,
            chip_palette: [
                Color::Rgb(142, 219, 255),
                Color::Rgb(174, 232, 192),
                Color::Rgb(255, 226, 153),
                Color::Rgb(244, 190, 255),
                Color::Rgb(174, 198, 255),
                Color::Rgb(255, 194, 194),
            ],
        }
    }

    pub(super) fn app_style(self) -> Style {
        Style::default().fg(self.panel_fg).bg(self.app_bg)
    }

    pub(super) fn panel_style(self) -> Style {
        Style::default().fg(self.panel_fg).bg(self.panel_bg)
    }

    pub(super) fn border_style(self) -> Style {
        Style::default().fg(self.border).bg(self.panel_bg)
    }

    pub(super) fn muted_style(self) -> Style {
        Style::default().fg(self.muted).bg(self.panel_bg)
    }

    pub(super) fn selected_style(self) -> Style {
        Style::default()
            .fg(self.selection_fg)
            .bg(self.selection_bg)
            .add_modifier(Modifier::BOLD)
    }

    pub(super) fn input_style(self) -> Style {
        Style::default()
            .fg(self.input_fg)
            .bg(self.input_bg)
            .add_modifier(Modifier::BOLD)
    }

    pub(super) fn status_color(self, state: OperationState) -> Color {
        match state {
            OperationState::Idle => self.info,
            OperationState::Running => self.warning,
            OperationState::Success => self.success,
            OperationState::Failure => self.error,
        }
    }

    pub(super) fn chip_style(self, index: usize) -> Style {
        Style::default()
            .fg(self.chip_fg)
            .bg(self.chip_palette[index % self.chip_palette.len()])
            .add_modifier(Modifier::BOLD)
    }

    pub(super) fn block<'a, T>(self, title: T) -> Block<'a>
    where
        T: Into<Title<'a>>,
    {
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .style(self.panel_style())
            .border_style(self.border_style())
    }

    pub(super) fn overlay_block<'a, T>(self, title: T, padding: u16) -> Block<'a>
    where
        T: Into<Title<'a>>,
    {
        self.block(title).padding(Padding::uniform(padding))
    }

    pub(super) fn focused_block<'a, T>(self, title: T, focused: bool) -> Block<'a>
    where
        T: Into<Title<'a>>,
    {
        let border_style = if focused {
            Style::default()
                .fg(self.focus_border)
                .bg(self.panel_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            self.border_style()
        };

        self.block(title).border_style(border_style)
    }
}
