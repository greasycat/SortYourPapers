use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Color, Frame, Line, Modifier, Span, Style},
    widgets::{ListItem, Paragraph, Wrap},
};

use crate::cli::{DEFAULT_INPUT, DEFAULT_OUTPUT};
use crate::tui::{
    forms::{
        bool_label, placement_mode_label, provider_label, run_field_help, run_field_label,
        taxonomy_mode_label,
    },
    theme::ThemePalette,
    ui_widgets::{render_selectable_list, stylized_body_line},
};

use super::{
    RunForm,
    state::RunFormAnalysis,
    validation::{api_key_summary, display_path_line, summarize_issue},
};

impl RunForm {
    pub(crate) fn draw(&self, frame: &mut Frame, area: Rect, theme: ThemePalette) {
        let analysis = self.analysis();
        let (chunks, side_chunks) = if area.width < 140 {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(56), Constraint::Percentage(44)])
                .split(area);
            let side_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
                .split(chunks[1]);
            (chunks, side_chunks)
        } else {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(64), Constraint::Percentage(36)])
                .split(area);
            let side_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(56), Constraint::Percentage(44)])
                .split(chunks[1]);
            (chunks, side_chunks)
        };

        self.draw_form_workspace(frame, chunks[0], &analysis, theme);
        self.draw_summary(frame, side_chunks[0], &analysis, theme);
        self.draw_selected_field(frame, side_chunks[1], &analysis, theme);
    }

    fn draw_form_workspace(
        &self,
        frame: &mut Frame,
        area: Rect,
        analysis: &RunFormAnalysis,
        theme: ThemePalette,
    ) {
        let outer = theme.block("Run Setup");
        let inner = outer.inner(area);
        frame.render_widget(outer, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let chunks = if inner.width < 120 {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(34),
                    Constraint::Percentage(33),
                    Constraint::Percentage(33),
                ])
                .split(inner)
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(34),
                    Constraint::Percentage(33),
                    Constraint::Percentage(33),
                ])
                .split(inner)
        };

        const COLUMN_SECTIONS: [[(&str, &[usize]); 2]; 3] = [
            [("Paths & Scope", &[0, 1, 2]), ("Extraction", &[3, 4, 5])],
            [
                ("Taxonomy", &[6, 7, 8, 22, 18, 19]),
                ("Placement", &[9, 10]),
            ],
            [
                ("LLM & API", &[13, 14, 15, 16, 17]),
                ("Run", &[11, 12, 20, 21, RunForm::RUN_BUTTON_INDEX]),
            ],
        ];

        for (column, sections) in chunks.iter().zip(COLUMN_SECTIONS.iter()) {
            self.draw_column(frame, *column, sections, analysis, theme);
        }
    }

    fn draw_summary(
        &self,
        frame: &mut Frame,
        area: Rect,
        analysis: &RunFormAnalysis,
        theme: ThemePalette,
    ) {
        let (infos, warnings, errors) = analysis.issue_counts();
        let readiness_color = if errors > 0 {
            theme.error
        } else if warnings > 0 {
            theme.warning
        } else {
            theme.success
        };
        let mode_color = if self.apply { theme.error } else { theme.info };
        let output_color = if self.quiet {
            theme.warning
        } else {
            theme.success
        };
        let body_color = theme.panel_fg;
        let mut lines = vec![
            Line::from(vec![
                badge_span("STATUS", readiness_color),
                Span::raw(" "),
                Span::styled(
                    analysis.readiness_text(),
                    Style::default()
                        .fg(readiness_color)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                badge_span(if self.apply { "APPLY" } else { "PREVIEW" }, mode_color),
                Span::raw(" "),
                badge_span(&format!("{errors} ERR"), theme.error),
                Span::raw(" "),
                badge_span(&format!("{warnings} WARN"), theme.warning),
                Span::raw(" "),
                badge_span(&format!("{infos} NOTE"), theme.info),
            ]),
            Line::from(""),
            section_header_line("Paths", theme.info),
            labeled_value_line(
                "In ",
                &display_path_line(&self.input, DEFAULT_INPUT),
                theme.info,
                body_color,
            ),
            labeled_value_line(
                "Out",
                &display_path_line(&self.output, DEFAULT_OUTPUT),
                theme.info,
                body_color,
            ),
            Line::from(""),
            section_header_line("Pipeline", theme.accent),
            labeled_value_line(
                "Extract",
                &format!(
                    "{} MB | {} page(s) | {} worker(s)",
                    self.max_file_size_mb.trim(),
                    self.page_cutoff.trim(),
                    self.pdf_extract_workers.trim()
                ),
                theme.accent,
                body_color,
            ),
            labeled_value_line(
                "Taxonomy",
                &format!(
                    "depth {} | {} | batch {} | tree {}",
                    self.category_depth.trim(),
                    taxonomy_mode_label(self.taxonomy_mode),
                    self.taxonomy_batch_size.trim(),
                    bool_label(self.use_current_folder_tree)
                ),
                theme.accent,
                body_color,
            ),
            labeled_value_line(
                "Ideas",
                &format!(
                    "keywords {} | suggestions {}",
                    self.keyword_batch_size.trim(),
                    self.subcategories_suggestion_number.trim()
                ),
                theme.accent,
                body_color,
            ),
            labeled_value_line(
                "Place",
                &format!(
                    "{} | batch {}",
                    placement_mode_label(self.placement_mode),
                    self.placement_batch_size.trim()
                ),
                theme.accent,
                body_color,
            ),
            Line::from(""),
            section_header_line("Launch", theme.success),
            labeled_value_line(
                "LLM",
                &format!(
                    "{} / {}",
                    provider_label(self.llm_provider),
                    if self.llm_model.trim().is_empty() {
                        "<missing>"
                    } else {
                        self.llm_model.trim()
                    }
                ),
                theme.success,
                body_color,
            ),
            labeled_value_line(
                "Auth",
                &format!(
                    "{} / {}",
                    self.api_key_source.label(),
                    api_key_summary(self.api_key_source, &self.api_key_value)
                ),
                theme.success,
                body_color,
            ),
            labeled_value_line(
                "Output",
                &format!(
                    "rebuild {} | quiet {} | {}",
                    bool_label(self.rebuild),
                    bool_label(self.quiet),
                    self.verbosity.label()
                ),
                theme.success,
                output_color,
            ),
        ];

        if let Some(config) = &analysis.config {
            lines.push(Line::from(""));
            lines.push(section_header_line("Resolved", theme.warning));
            lines.push(labeled_value_line(
                "Mode",
                if config.dry_run { "preview" } else { "apply" },
                theme.warning,
                body_color,
            ));
            lines.push(labeled_value_line(
                "Scope",
                if self.recursive {
                    "recursive"
                } else {
                    "top-level only"
                },
                theme.warning,
                body_color,
            ));
        }

        let notable_issues = analysis.issues.iter().take(4).collect::<Vec<_>>();
        if !notable_issues.is_empty() {
            lines.push(Line::from(""));
            lines.push(section_header_line("Issues", theme.error));
            for issue in notable_issues {
                lines.push(Line::from(Span::styled(
                    format!(
                        "{} {}",
                        issue.severity.marker(),
                        summarize_issue(issue, analysis)
                    ),
                    Style::default().fg(issue.severity.color()),
                )));
            }
        }

        frame.render_widget(
            Paragraph::new(lines)
                .style(theme.panel_style())
                .wrap(Wrap { trim: false })
                .block(
                    theme
                        .block("Launch Preview")
                        .border_style(Style::default().fg(readiness_color).bg(theme.panel_bg)),
                ),
            area,
        );
    }

    fn draw_selected_field(
        &self,
        frame: &mut Frame,
        area: Rect,
        analysis: &RunFormAnalysis,
        theme: ThemePalette,
    ) {
        let selected_label = run_field_label(self.selected);
        let selected_value = self.value(self.selected);
        let mut lines = vec![
            Line::from(Span::styled(
                selected_label,
                Style::default()
                    .fg(theme.info)
                    .bg(theme.panel_bg)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            section_header_line("Description", theme.info),
            stylized_body_line(run_field_help(self.selected), theme),
            Line::from(""),
            section_header_line("Current", theme.success),
            stylized_body_line(&selected_value, theme),
        ];

        if let Some(issue) = analysis.field_issue(self.selected) {
            lines.push(Line::from(""));
            lines.push(section_header_line("Issue", issue.severity.color()));
            lines.push(Line::from(Span::styled(
                format!("{}: {}", issue.severity.title(), issue.message),
                Style::default()
                    .fg(issue.severity.color())
                    .add_modifier(Modifier::BOLD),
            )));
        }

        frame.render_widget(
            Paragraph::new(lines)
                .style(theme.panel_style())
                .wrap(Wrap { trim: false })
                .block(theme.block("Selected Field")),
            area,
        );
    }

    fn draw_column(
        &self,
        frame: &mut Frame,
        area: Rect,
        sections: &[(&str, &[usize])],
        analysis: &RunFormAnalysis,
        theme: ThemePalette,
    ) {
        let mut items = Vec::new();
        let mut selected_item = None;
        for (section_index, (title, fields)) in sections.iter().enumerate() {
            if section_index > 0 {
                items.push(ListItem::new(""));
            }
            items.push(ListItem::new(Line::from(Span::styled(
                (*title).to_string(),
                Style::default()
                    .fg(theme.info)
                    .bg(theme.panel_bg)
                    .add_modifier(Modifier::BOLD),
            ))));

            for field_index in *fields {
                if *field_index == Self::RUN_BUTTON_INDEX {
                    if *field_index == self.selected {
                        selected_item = Some(items.len());
                    }
                    items.push(ListItem::new(Line::styled(
                        "  [ Run ]  ",
                        Style::default()
                            .fg(theme.selection_fg)
                            .bg(theme.selection_bg)
                            .add_modifier(Modifier::BOLD),
                    )));
                    continue;
                }

                let marker = analysis
                    .field_issue(*field_index)
                    .map_or(' ', |issue| issue.severity.marker());
                let content = format!(
                    "{} {}: {}",
                    marker,
                    run_field_label(*field_index),
                    self.value(*field_index)
                );

                if *field_index == self.selected {
                    selected_item = Some(items.len());
                }

                if let Some(issue) = analysis.field_issue(*field_index) {
                    items.push(ListItem::new(Line::styled(
                        content,
                        Style::default().fg(issue.severity.color()),
                    )));
                } else {
                    items.push(ListItem::new(content));
                }
            }
        }

        render_selectable_list(frame, area, theme.block(""), items, selected_item, theme);
    }
}

fn badge_span(label: &str, color: Color) -> Span<'static> {
    Span::styled(
        format!("[{label}]"),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

fn section_header_line(title: &str, color: Color) -> Line<'static> {
    Line::from(Span::styled(
        title.to_string(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ))
}

fn labeled_value_line(
    label: &str,
    value: &str,
    label_color: Color,
    value_color: Color,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{label:<8}"),
            Style::default()
                .fg(label_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(value.to_string(), Style::default().fg(value_color)),
    ])
}
