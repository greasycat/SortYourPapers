use std::collections::BTreeSet;

use ratatui::{
    layout::Rect,
    prelude::{Color, Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Scrollbar, ScrollbarOrientation},
};
use tui_tree_widget::{Tree, TreeItem, TreeState};

use crate::papers::taxonomy::CategoryTree;

use super::theme::ThemePalette;

pub(super) type TaxonomyTreeState = TreeState<usize>;

#[derive(Clone)]
pub(super) enum TaxonomySection {
    Categories {
        title: String,
        categories: Vec<CategoryTree>,
        marked_paths: BTreeSet<Vec<String>>,
    },
}

pub(super) fn reset_state_for_categories(
    state: &mut TaxonomyTreeState,
    categories: &[CategoryTree],
) {
    let items = category_items(categories);
    reset_state_for_items(state, &items);
}

pub(super) fn reset_state_for_sections(
    state: &mut TaxonomyTreeState,
    sections: &[TaxonomySection],
) {
    let items = section_items(sections);
    reset_state_for_items(state, &items);
}

pub(super) fn render_category_tree(
    frame: &mut Frame,
    area: Rect,
    block: Block<'_>,
    categories: &[CategoryTree],
    state: &mut TaxonomyTreeState,
    theme: ThemePalette,
) {
    let items = category_items(categories);
    render_tree_items(frame, area, block, &items, state, theme);
}

pub(super) fn render_section_tree(
    frame: &mut Frame,
    area: Rect,
    block: Block<'_>,
    sections: &[TaxonomySection],
    state: &mut TaxonomyTreeState,
    theme: ThemePalette,
) {
    let items = section_items(sections);
    render_tree_items(frame, area, block, &items, state, theme);
}

fn render_tree_items(
    frame: &mut Frame,
    area: Rect,
    block: Block<'_>,
    items: &[TreeItem<'static, usize>],
    state: &mut TaxonomyTreeState,
    theme: ThemePalette,
) {
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new("No taxonomy available.")
                .style(theme.panel_style())
                .block(block),
            area,
        );
        return;
    }

    let tree = Tree::new(items)
        .expect("taxonomy tree uses unique sibling indices")
        .block(block)
        .style(theme.panel_style())
        .highlight_style(
            Style::default()
                .fg(theme.selection_fg)
                .bg(theme.selection_bg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ")
        .experimental_scrollbar(Some(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_style(theme.muted_style())
                .thumb_style(
                    Style::default()
                        .fg(theme.scrollbar_thumb)
                        .bg(theme.panel_bg),
                ),
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
    category_items_with_marks(categories, &BTreeSet::new())
}

fn category_items_with_marks(
    categories: &[CategoryTree],
    marked_paths: &BTreeSet<Vec<String>>,
) -> Vec<TreeItem<'static, usize>> {
    categories
        .iter()
        .enumerate()
        .map(|(index, category)| category_item(index, category, &mut Vec::new(), marked_paths))
        .collect()
}

fn category_item(
    index: usize,
    category: &CategoryTree,
    path: &mut Vec<String>,
    marked_paths: &BTreeSet<Vec<String>>,
) -> TreeItem<'static, usize> {
    path.push(category.name.clone());
    let children = category
        .children
        .iter()
        .enumerate()
        .map(|(child_index, child)| category_item(child_index, child, path, marked_paths))
        .collect::<Vec<_>>();
    let is_marked = marked_paths.contains(path);
    let text = if is_marked {
        Line::from(vec![Span::styled(
            format!("x {}", category.name),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )])
    } else {
        Line::from(category.name.clone())
    };
    path.pop();

    if children.is_empty() {
        TreeItem::new_leaf(index, text)
    } else {
        TreeItem::new(index, text, children).expect("taxonomy tree uses unique sibling indices")
    }
}

fn section_items(sections: &[TaxonomySection]) -> Vec<TreeItem<'static, usize>> {
    sections
        .iter()
        .enumerate()
        .map(|(index, section)| match section {
            TaxonomySection::Categories {
                title,
                categories,
                marked_paths,
            } => {
                let children = category_items_with_marks(categories, marked_paths);
                TreeItem::new(index, title.clone(), children)
                    .expect("taxonomy section tree uses unique sibling indices")
            }
        })
        .collect()
}
