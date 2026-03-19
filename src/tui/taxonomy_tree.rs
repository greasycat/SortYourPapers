use ratatui::{
    layout::Rect,
    prelude::{Color, Frame, Modifier, Style},
    widgets::{Block, Paragraph, Scrollbar, ScrollbarOrientation},
};
use tui_tree_widget::{Tree, TreeItem, TreeState};

use crate::papers::taxonomy::CategoryTree;

pub(super) type TaxonomyTreeState = TreeState<usize>;

#[derive(Clone)]
pub(super) enum TaxonomySection {
    Categories {
        title: String,
        categories: Vec<CategoryTree>,
    },
}

pub(super) fn reset_state_for_categories(state: &mut TaxonomyTreeState, categories: &[CategoryTree]) {
    let items = category_items(categories);
    reset_state_for_items(state, &items);
}

pub(super) fn reset_state_for_sections(state: &mut TaxonomyTreeState, sections: &[TaxonomySection]) {
    let items = section_items(sections);
    reset_state_for_items(state, &items);
}

pub(super) fn render_category_tree(
    frame: &mut Frame,
    area: Rect,
    block: Block<'_>,
    categories: &[CategoryTree],
    state: &mut TaxonomyTreeState,
) {
    let items = category_items(categories);
    render_tree_items(frame, area, block, &items, state);
}

pub(super) fn render_section_tree(
    frame: &mut Frame,
    area: Rect,
    block: Block<'_>,
    sections: &[TaxonomySection],
    state: &mut TaxonomyTreeState,
) {
    let items = section_items(sections);
    render_tree_items(frame, area, block, &items, state);
}

fn render_tree_items(
    frame: &mut Frame,
    area: Rect,
    block: Block<'_>,
    items: &[TreeItem<'static, usize>],
    state: &mut TaxonomyTreeState,
) {
    if items.is_empty() {
        frame.render_widget(Paragraph::new("No taxonomy available.").block(block), area);
        return;
    }

    let tree = Tree::new(items)
        .expect("taxonomy tree uses unique sibling indices")
        .block(block)
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ")
        .experimental_scrollbar(Some(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_style(Style::default().fg(Color::Gray))
                .thumb_style(Style::default().fg(Color::Green)),
        ));
    frame.render_stateful_widget(tree, area, state);
}

fn reset_state_for_items(state: &mut TaxonomyTreeState, items: &[TreeItem<'static, usize>]) {
    *state = TaxonomyTreeState::default();
    open_all(state, items, &[]);
    if let Some(first) = items.first() {
        let _ = state.select(vec![*first.identifier()]);
    }
}

fn open_all(state: &mut TaxonomyTreeState, items: &[TreeItem<'static, usize>], prefix: &[usize]) {
    for item in items {
        let mut identifier = prefix.to_vec();
        identifier.push(*item.identifier());
        if !item.children().is_empty() {
            let _ = state.open(identifier.clone());
            open_all(state, item.children(), &identifier);
        }
    }
}

fn category_items(categories: &[CategoryTree]) -> Vec<TreeItem<'static, usize>> {
    categories
        .iter()
        .enumerate()
        .map(|(index, category)| category_item(index, category))
        .collect()
}

fn category_item(index: usize, category: &CategoryTree) -> TreeItem<'static, usize> {
    let children = category
        .children
        .iter()
        .enumerate()
        .map(|(child_index, child)| category_item(child_index, child))
        .collect::<Vec<_>>();

    if children.is_empty() {
        TreeItem::new_leaf(index, category.name.clone())
    } else {
        TreeItem::new(index, category.name.clone(), children)
            .expect("taxonomy tree uses unique sibling indices")
    }
}

fn section_items(sections: &[TaxonomySection]) -> Vec<TreeItem<'static, usize>> {
    sections
        .iter()
        .enumerate()
        .map(|(index, section)| match section {
            TaxonomySection::Categories { title, categories } => {
                let children = category_items(categories);
                TreeItem::new(index, title.clone(), children)
                    .expect("taxonomy section tree uses unique sibling indices")
            }
        })
        .collect()
}
