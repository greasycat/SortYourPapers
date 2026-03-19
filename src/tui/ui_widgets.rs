use ratatui::{
    layout::Rect,
    prelude::{Color, Frame, Line, Modifier, Style},
    widgets::{
        Block, HighlightSpacing, List, ListItem, ListState, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Tabs, Wrap,
    },
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

pub(super) fn render_scrolled_paragraph<'a>(
    frame: &mut Frame,
    area: Rect,
    block: Block<'a>,
    content: Vec<Line<'a>>,
    scroll: u16,
    wrap: bool,
) {
    let content_len = content.len();
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let needs_scrollbar = content_len > usize::from(inner.height) && inner.width > 1;
    let text_area = if needs_scrollbar {
        Rect::new(inner.x, inner.y, inner.width.saturating_sub(1), inner.height)
    } else {
        inner
    };

    let mut paragraph = Paragraph::new(content).scroll((scroll, 0));
    if wrap {
        paragraph = paragraph.wrap(Wrap { trim: false });
    }
    frame.render_widget(paragraph, text_area);

    if !needs_scrollbar {
        return;
    }

    let mut scrollbar_state = ScrollbarState::new(content_len)
        .position(usize::from(scroll).min(content_len.saturating_sub(1)))
        .viewport_content_length(usize::from(inner.height));
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None)
        .track_style(muted_style())
        .thumb_style(Style::default().fg(Color::Green));
    frame.render_stateful_widget(scrollbar, inner, &mut scrollbar_state);
}
