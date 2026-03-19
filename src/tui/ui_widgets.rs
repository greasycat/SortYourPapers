use ratatui::{
    layout::Rect,
    prelude::{Color, Frame, Line, Modifier, Style},
    widgets::{Block, HighlightSpacing, List, ListItem, ListState, Tabs},
};

pub(super) fn selected_style() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(Color::Green)
        .add_modifier(Modifier::BOLD)
}

pub(super) fn muted_style() -> Style {
    Style::default().fg(Color::Gray)
}

pub(super) fn render_tabs<'a>(
    frame: &mut Frame,
    area: Rect,
    block: Block<'a>,
    titles: Vec<Line<'a>>,
    selected: usize,
) {
    let tabs = Tabs::new(titles)
        .block(block)
        .select(selected)
        .style(muted_style())
        .highlight_style(selected_style())
        .divider(" ")
        .padding("", "");
    frame.render_widget(tabs, area);
}

pub(super) fn render_selectable_list<'a>(
    frame: &mut Frame,
    area: Rect,
    block: Block<'a>,
    items: Vec<ListItem<'a>>,
    selected: Option<usize>,
) {
    let mut state = ListState::default();
    state.select(selected);
    let list = List::new(items)
        .block(block)
        .highlight_style(selected_style())
        .highlight_symbol(">> ")
        .highlight_spacing(HighlightSpacing::Always)
        .scroll_padding(1);
    frame.render_stateful_widget(list, area, &mut state);
}
