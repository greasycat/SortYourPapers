use std::env;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::{
    error::{AppError, Result},
    papers::SynthesizeCategoriesState,
    session::workspace::RunStage as WorkspaceRunStage,
    session::{RunWorkspace, stage_sequence},
    terminal::InspectReviewPrompt,
};

use super::{
    app::App,
    extract::{collect_extract_preview, render_extract_result_lines},
    forms::{extract_field_label, list_relative_directories, run_field_label},
    model::{
        ConfirmAction, HomeAction, OperationDetail, OperationOutcome, OperationState, OperationTab,
        Overlay, Screen,
    },
    session_view::rerun_stage_name,
    taxonomy_review::ReviewPhase,
};

impl App {
    pub(super) async fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        if self.handle_overlay_key(key).await? {
            return Ok(());
        }

        match self.screen {
            Screen::Home => self.handle_home_key(key).await,
            Screen::RunForm => self.handle_run_form_key(key).await,
            Screen::ExtractForm => self.handle_extract_form_key(key).await,
            Screen::Sessions => self.handle_sessions_key(key).await,
            Screen::Operation => self.handle_operation_key(key),
            Screen::TaxonomyReview => self.handle_taxonomy_review_key(key),
        }
    }

    async fn handle_home_key(&mut self, key: KeyEvent) -> Result<()> {
        self.clamp_home_index();
        let action_count = self.home_actions().len();
        match key.code {
            KeyCode::Esc => self.open_quit_confirmation(),
            KeyCode::Down | KeyCode::Char('j') => {
                self.home_index = (self.home_index + 1).min(action_count.saturating_sub(1));
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.home_index = self.home_index.saturating_sub(1);
            }
            KeyCode::Enter => {
                self.screen = match self.selected_home_action() {
                    HomeAction::RunPapers => Screen::RunForm,
                    HomeAction::ExtractText => Screen::ExtractForm,
                    HomeAction::Sessions => {
                        self.session_view.refresh()?;
                        Screen::Sessions
                    }
                    HomeAction::Quit => {
                        self.open_quit_confirmation();
                        Screen::Home
                    }
                };
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_extract_form_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => self.screen = Screen::Home,
            KeyCode::Down | KeyCode::Char('j') => {
                self.extract_form.selected = (self.extract_form.selected + 1).min(4);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.extract_form.selected = self.extract_form.selected.saturating_sub(1);
            }
            KeyCode::Left | KeyCode::Char('h') => self.extract_form.cycle_selected(-1),
            KeyCode::Right | KeyCode::Char('l') => self.extract_form.cycle_selected(1),
            KeyCode::Char('r') => {
                let args = match self.extract_form.build_args() {
                    Ok(args) => args,
                    Err(err) => {
                        self.overlay = Some(Overlay::Notice {
                            title: "Extract Configuration".to_string(),
                            message: err.to_string(),
                        });
                        return Ok(());
                    }
                };

                self.start_async_operation("Extract Text", move |tx| async move {
                    let outcome = match collect_extract_preview(args).await {
                        Ok(result) => {
                            let summary = format!(
                                "extracted {} PDF(s); {} extraction failure(s)",
                                result.papers.len(),
                                result.failures.len()
                            );
                            let detail = OperationDetail::Text {
                                title: "Extract Preview".to_string(),
                                lines: render_extract_result_lines(&result),
                                empty_message: "No extracted text was produced.".to_string(),
                            };
                            if result.failures.is_empty() {
                                OperationOutcome::success("Extract Text", summary, detail)
                            } else {
                                OperationOutcome::failure("Extract Text", summary, detail)
                            }
                        }
                        Err(err) => OperationOutcome::failure(
                            "Extract Text",
                            err.to_string(),
                            OperationDetail::None,
                        ),
                    };
                    let _ = tx.send(outcome);
                });
            }
            KeyCode::Enter => {
                if matches!(self.extract_form.selected, 0 | 1 | 3) {
                    self.overlay = Some(Overlay::EditField {
                        label: extract_field_label(self.extract_form.selected).to_string(),
                        buffer: self.extract_form.value(self.extract_form.selected),
                    });
                } else {
                    self.extract_form.cycle_selected(1);
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_sessions_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => self.screen = Screen::Home,
            KeyCode::Char('g') => self.session_view.refresh()?,
            KeyCode::Down | KeyCode::Char('j') => self.session_view.move_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.session_view.move_selection(-1),
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                self.session_view.switch_preview_tab(1);
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                self.session_view.switch_preview_tab(-1);
            }
            KeyCode::PageDown => self.session_view.scroll_preview(10),
            KeyCode::PageUp => self.session_view.scroll_preview(-10),
            KeyCode::Char('1')
            | KeyCode::Char('2')
            | KeyCode::Char('3')
            | KeyCode::Char('4')
            | KeyCode::Char('5') => {
                if let KeyCode::Char(key) = key.code {
                    self.session_view.set_filter_for_key(key);
                }
            }
            KeyCode::Char('p') => {
                if let Some(run_id) = self.session_view.selected_run_id() {
                    self.start_async_operation("Resume Session", move |tx| async move {
                        match crate::resume_run(Some(run_id.clone()), false, 0, false).await {
                            Ok(_) => tx.send(OperationOutcome::success(
                                "Resume Session",
                                format!("resumed {run_id} in preview mode"),
                                OperationDetail::None,
                            )),
                            Err(err) => tx.send(OperationOutcome::failure(
                                "Resume Session",
                                err.to_string(),
                                OperationDetail::None,
                            )),
                        }
                        .ok();
                    });
                }
            }
            KeyCode::Char('a') => {
                if let Some(run_id) = self.session_view.selected_run_id() {
                    self.start_async_operation("Resume Session", move |tx| async move {
                        match crate::resume_run(Some(run_id.clone()), true, 0, false).await {
                            Ok(_) => tx.send(OperationOutcome::success(
                                "Resume Session",
                                format!("resumed {run_id} in apply mode"),
                                OperationDetail::None,
                            )),
                            Err(err) => tx.send(OperationOutcome::failure(
                                "Resume Session",
                                err.to_string(),
                                OperationDetail::None,
                            )),
                        }
                        .ok();
                    });
                }
            }
            KeyCode::Char('r') => self.open_rerun_overlay(false)?,
            KeyCode::Char('x') => self.open_rerun_overlay(true)?,
            KeyCode::Char('v') => {
                if let Some(run_id) = self.session_view.selected_run_id() {
                    self.start_blocking_operation("Review Session", move || {
                        let workspace = RunWorkspace::open(&run_id)?;
                        if workspace.last_completed_stage() != Some(WorkspaceRunStage::Completed) {
                            return Err(AppError::Execution(format!(
                                "run '{run_id}' is not completed"
                            )));
                        }
                        let categories = workspace
                            .load_stage::<SynthesizeCategoriesState>(
                                WorkspaceRunStage::SynthesizeCategories,
                            )?
                            .ok_or_else(|| {
                                AppError::Execution(format!(
                                    "run '{run_id}' has no saved synthesized categories"
                                ))
                            })?;
                        Ok(OperationOutcome::success(
                            "Review Session",
                            format!("loaded taxonomy for {run_id}"),
                            OperationDetail::Tree(categories.categories),
                        ))
                    });
                }
            }
            KeyCode::Char('d') => {
                if let Some(run_id) = self.session_view.selected_run_id() {
                    self.overlay = Some(Overlay::Confirm {
                        title: "Remove Session".to_string(),
                        message: format!("Remove saved session {run_id}?"),
                        action: ConfirmAction::RemoveRun(run_id),
                    });
                }
            }
            KeyCode::Char('c') => {
                self.overlay = Some(Overlay::Confirm {
                    title: "Clear Incomplete Sessions".to_string(),
                    message: "Clear all incomplete saved sessions for this workspace?".to_string(),
                    action: ConfirmAction::ClearIncomplete,
                });
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_run_form_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => self.screen = Screen::Home,
            KeyCode::Down | KeyCode::Char('j') => {
                self.run_form.select_next();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.run_form.select_previous();
            }
            KeyCode::Left | KeyCode::Char('h') => self.run_form.move_column_left(),
            KeyCode::Right | KeyCode::Char('l') => self.run_form.move_column_right(),
            KeyCode::Char(' ') => {
                if self.run_form.run_button_selected() {
                    self.launch_run_from_form();
                } else {
                    self.run_form.toggle_selected();
                }
            }
            KeyCode::Char('r') => self.launch_run_from_form(),
            KeyCode::Char('s') => {
                let analysis = self.run_form.analysis();
                if analysis.has_errors() {
                    self.overlay = Some(Overlay::Notice {
                        title: "Fix Validation Errors".to_string(),
                        message: analysis.blocking_message(),
                    });
                    return Ok(());
                }

                let Some(config) = analysis.config else {
                    self.overlay = Some(Overlay::Notice {
                        title: "Save Config".to_string(),
                        message: "The run configuration is not ready to save yet.".to_string(),
                    });
                    return Ok(());
                };

                let Some(path) = crate::config::xdg_config_path() else {
                    self.overlay = Some(Overlay::Notice {
                        title: "Save Config".to_string(),
                        message: "Could not resolve the XDG config path.".to_string(),
                    });
                    return Ok(());
                };

                self.overlay = Some(Overlay::Confirm {
                    title: "Save Config".to_string(),
                    message: format!(
                        "Save the current run parameters to {}?\nThis overwrites the existing config file.",
                        path.display()
                    ),
                    action: ConfirmAction::SaveRunConfig(config),
                });
            }
            KeyCode::Enter => {
                if self.run_form.run_button_selected() {
                    self.launch_run_from_form();
                } else if self.run_form.editable(self.run_form.selected) {
                    if self.run_form.selected <= 1 {
                        self.open_run_path_overlay()?;
                    } else {
                        self.overlay = Some(Overlay::EditField {
                            label: run_field_label(self.run_form.selected).to_string(),
                            buffer: self.run_form.value(self.run_form.selected),
                        });
                    }
                } else {
                    self.run_form.toggle_selected();
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn launch_run_from_form(&mut self) {
        let analysis = self.run_form.analysis();
        if analysis.has_errors() {
            self.overlay = Some(Overlay::Notice {
                title: "Fix Validation Errors".to_string(),
                message: analysis.blocking_message(),
            });
            return;
        }

        let Some(config) = analysis.config else {
            self.overlay = Some(Overlay::Notice {
                title: "Run Configuration".to_string(),
                message: "The run configuration is not ready yet.".to_string(),
            });
            return;
        };
        let use_debug_tui = self.debug_tui;
        let op_tx = self.op_tx.clone();
        self.start_async_operation("Run Papers", move |_tx| async move {
            let outcome = match if use_debug_tui {
                crate::app::run_debug_tui(config).await
            } else {
                crate::run(config).await
            } {
                Ok(_) => OperationOutcome::success(
                    "Run Papers",
                    "run completed".to_string(),
                    OperationDetail::None,
                ),
                Err(err) => {
                    OperationOutcome::failure("Run Papers", err.to_string(), OperationDetail::None)
                }
            };
            let _ = op_tx.send(outcome);
        });
    }

    fn handle_operation_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => self.switch_operation_tab(1),
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                self.switch_operation_tab(-1);
            }
            KeyCode::Char('1') => self.set_operation_tab(OperationTab::Summary),
            KeyCode::Char('2') => self.set_operation_tab(OperationTab::Logs),
            KeyCode::Char('3') => self.set_operation_tab(OperationTab::Taxonomy),
            KeyCode::Char('4') => self.set_operation_tab(OperationTab::Report),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_active_operation_tab(1),
            KeyCode::Up | KeyCode::Char('k') => self.scroll_active_operation_tab(-1),
            KeyCode::Char(' ') => self.toggle_active_operation_taxonomy(),
            KeyCode::PageDown => self.scroll_active_operation_tab(10),
            KeyCode::PageUp => self.scroll_active_operation_tab(-10),
            KeyCode::Char('g') => self.jump_active_operation_tab(false),
            KeyCode::Char('G') => self.jump_active_operation_tab(true),
            KeyCode::Char('s') => {
                if !matches!(self.operation.state, OperationState::Running) {
                    self.session_view.refresh()?;
                    self.screen = Screen::Sessions;
                }
            }
            KeyCode::Esc => {
                if !matches!(self.operation.state, OperationState::Running) {
                    if matches!(self.operation.origin, Screen::Sessions) {
                        self.session_view.refresh()?;
                    }
                    self.screen = self.operation.origin;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_taxonomy_review_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(review) = self.taxonomy_review.as_mut() else {
            self.screen = Screen::Operation;
            return Ok(());
        };

        if review.editing {
            match key.code {
                KeyCode::Esc => review.stop_editing(),
                KeyCode::Enter => {
                    if let Some(request) = review.submit_review_request()
                        && let Some(reply) = review.take_inspect_reply()
                    {
                        let _ = reply.send(Ok(InspectReviewPrompt::Suggest(request)));
                    }
                }
                KeyCode::Backspace => review.pop_input(),
                KeyCode::Char(c) => {
                    if !key.modifiers.contains(KeyModifiers::CONTROL) {
                        review.append_input(c);
                    }
                }
                _ => {}
            }
            return Ok(());
        }

        match key.code {
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => review.focus_next(),
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => review.focus_previous(),
            KeyCode::Down | KeyCode::Char('j') => review.scroll_focused(1),
            KeyCode::Up | KeyCode::Char('k') => review.scroll_focused(-1),
            KeyCode::Char(' ') => review.toggle_focused_tree(),
            KeyCode::Char('d') => review.toggle_selected_removal(),
            KeyCode::Char('D') => review.toggle_selected_subtree_removal(),
            KeyCode::Char('x') => {
                review.cut_selected_entry();
            }
            KeyCode::Char('p') => {
                review.paste_cut_entry();
            }
            KeyCode::PageDown => review.scroll_focused(10),
            KeyCode::PageUp => review.scroll_focused(-10),
            KeyCode::Char('g') => review.jump_focused(false),
            KeyCode::Char('G') => review.jump_focused(true),
            KeyCode::Char('s') if matches!(review.phase, ReviewPhase::Drafting) => {
                review.start_editing();
            }
            KeyCode::Char('a') => match review.phase {
                ReviewPhase::Drafting => {
                    if review.has_marked_removals() {
                        if let Some(request) = review.submit_review_request()
                            && let Some(reply) = review.take_inspect_reply()
                        {
                            let _ = reply.send(Ok(InspectReviewPrompt::Suggest(request)));
                        }
                    } else if let Some(reply) = review.take_inspect_reply() {
                        let _ = reply.send(Ok(InspectReviewPrompt::Accept));
                        self.finish_taxonomy_review();
                    }
                }
                ReviewPhase::PostSuggestionDecision => {
                    self.overlay = Some(Overlay::Confirm {
                        title: "Accept Candidate".to_string(),
                        message:
                            "Accept this candidate taxonomy and finish the review?".to_string(),
                        action: ConfirmAction::AcceptTaxonomyCandidate,
                    });
                }
                ReviewPhase::WaitingForModel => {}
            },
            KeyCode::Char('i') if matches!(review.phase, ReviewPhase::PostSuggestionDecision) => {
                if let Some(reply) = review.take_continue_reply() {
                    let _ = reply.send(Ok(true));
                    review.promote_candidate_to_accepted();
                }
            }
            KeyCode::Char('c') | KeyCode::Esc => {
                let cancelled = if let Some(reply) = review.take_inspect_reply() {
                    let _ = reply.send(Err("inspect-output cancelled".to_string()));
                    true
                } else if let Some(reply) = review.take_continue_reply() {
                    let _ = reply.send(Err("inspect-output cancelled".to_string()));
                    true
                } else {
                    false
                };

                if cancelled {
                    self.finish_taxonomy_review();
                }
            }
            _ => {}
        }

        Ok(())
    }

    async fn handle_overlay_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(mut overlay) = self.overlay.take() else {
            return Ok(false);
        };

        let handled = match &mut overlay {
            Overlay::EditField { buffer, .. } => {
                match key.code {
                    KeyCode::Esc => {}
                    KeyCode::Enter => {
                        self.apply_edit(buffer.clone())?;
                    }
                    KeyCode::Backspace => {
                        buffer.pop();
                        self.overlay = Some(overlay);
                        return Ok(true);
                    }
                    KeyCode::Char(c) => {
                        if !key.modifiers.contains(KeyModifiers::CONTROL) {
                            buffer.push(c);
                            self.overlay = Some(overlay);
                        }
                        return Ok(true);
                    }
                    _ => {
                        self.overlay = Some(overlay);
                        return Ok(true);
                    }
                }
                return Ok(true);
            }
            Overlay::SelectPath {
                buffer,
                directories,
                selected,
                ..
            } => {
                match key.code {
                    KeyCode::Esc => {}
                    KeyCode::Enter => {
                        self.apply_edit(buffer.clone())?;
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        *selected = (*selected + 1).min(directories.len().saturating_sub(1));
                        self.overlay = Some(overlay);
                        return Ok(true);
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        *selected = selected.saturating_sub(1);
                        self.overlay = Some(overlay);
                        return Ok(true);
                    }
                    KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                        if let Some(choice) = directories.get(*selected).cloned() {
                            *buffer = choice;
                            Self::refresh_path_overlay(buffer, directories, selected)?;
                        }
                        self.overlay = Some(overlay);
                        return Ok(true);
                    }
                    KeyCode::Backspace => {
                        buffer.pop();
                        Self::refresh_path_overlay(buffer, directories, selected)?;
                        self.overlay = Some(overlay);
                        return Ok(true);
                    }
                    KeyCode::Char(c) => {
                        if !key.modifiers.contains(KeyModifiers::CONTROL) {
                            buffer.push(c);
                            Self::refresh_path_overlay(buffer, directories, selected)?;
                            self.overlay = Some(overlay);
                        }
                        return Ok(true);
                    }
                    _ => {
                        self.overlay = Some(overlay);
                        return Ok(true);
                    }
                }
                return Ok(true);
            }
            Overlay::Confirm { action, .. } => match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    self.confirm_action(action.clone())?;
                    return Ok(true);
                }
                KeyCode::Esc => {
                    return Ok(true);
                }
                _ => {
                    self.overlay = Some(overlay);
                    return Ok(true);
                }
            },
            Overlay::Notice { .. } => match key.code {
                KeyCode::Enter | KeyCode::Esc => true,
                _ => {
                    self.overlay = Some(overlay);
                    return Ok(true);
                }
            },
            Overlay::SelectRerunStage {
                stages,
                selected,
                run_id,
                apply,
                ..
            } => {
                match key.code {
                    KeyCode::Down | KeyCode::Char('j') => {
                        *selected = (*selected + 1).min(stages.len().saturating_sub(1));
                        self.overlay = Some(overlay);
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        *selected = selected.saturating_sub(1);
                        self.overlay = Some(overlay);
                    }
                    KeyCode::Enter => {
                        if let Some(stage) = stages.get(*selected).copied() {
                            let run_id = run_id.clone();
                            let apply = *apply;
                            self.start_async_operation("Rerun Session", move |tx| async move {
                                match crate::rerun_run(
                                    Some(run_id.clone()),
                                    Some(stage),
                                    apply,
                                    0,
                                    false,
                                )
                                .await
                                {
                                    Ok(_) => tx.send(OperationOutcome::success(
                                        "Rerun Session",
                                        format!(
                                            "reran {run_id} from {} in {} mode",
                                            rerun_stage_name(stage),
                                            if apply { "apply" } else { "preview" }
                                        ),
                                        OperationDetail::None,
                                    )),
                                    Err(err) => tx.send(OperationOutcome::failure(
                                        "Rerun Session",
                                        err.to_string(),
                                        OperationDetail::None,
                                    )),
                                }
                                .ok();
                            });
                        }
                    }
                    KeyCode::Esc => {}
                    _ => {
                        self.overlay = Some(overlay);
                        return Ok(true);
                    }
                }
                return Ok(true);
            }
        };

        Ok(handled)
    }

    fn confirm_action(&mut self, action: ConfirmAction) -> Result<()> {
        match action {
            ConfirmAction::Quit => {
                self.should_quit = true;
            }
            ConfirmAction::SaveRunConfig(config) => {
                let path = crate::config::save_xdg_config(&config)?;
                self.overlay = Some(Overlay::Notice {
                    title: "Config Saved".to_string(),
                    message: format!("Saved current run parameters to {}", path.display()),
                });
            }
            ConfirmAction::RemoveRun(run_id) => {
                self.start_blocking_operation("Remove Session", move || {
                    let removed = RunWorkspace::remove_runs(&[run_id.clone()])?;
                    let summary = if removed.is_empty() {
                        "no sessions removed".to_string()
                    } else {
                        format!("removed {}", removed.join(", "))
                    };
                    Ok(OperationOutcome::success(
                        "Remove Session",
                        summary,
                        OperationDetail::None,
                    ))
                });
            }
            ConfirmAction::ClearIncomplete => {
                self.start_blocking_operation("Clear Incomplete Sessions", move || {
                    let removed = RunWorkspace::clear_incomplete_runs()?;
                    Ok(OperationOutcome::success(
                        "Clear Incomplete Sessions",
                        format!("cleared {} incomplete session(s)", removed.len()),
                        OperationDetail::None,
                    ))
                });
            }
            ConfirmAction::AcceptTaxonomyCandidate => {
                if let Some(review) = self.taxonomy_review.as_mut()
                    && let Some(reply) = review.take_continue_reply()
                {
                    let _ = reply.send(Ok(false));
                    self.finish_taxonomy_review();
                }
            }
        }
        Ok(())
    }

    fn open_quit_confirmation(&mut self) {
        self.overlay = Some(Overlay::Confirm {
            title: "Quit".to_string(),
            message: "Quit SortYourPapers?".to_string(),
            action: ConfirmAction::Quit,
        });
    }

    fn open_rerun_overlay(&mut self, apply: bool) -> Result<()> {
        let Some(run_id) = self.session_view.selected_run_id() else {
            return Ok(());
        };
        let workspace = RunWorkspace::open(&run_id)?;
        let config = workspace.load_config()?;
        let stages = stage_sequence(config.rebuild && config.output.exists(), true);
        self.overlay = Some(Overlay::SelectRerunStage {
            run_id,
            apply,
            config,
            stages,
            selected: 0,
        });
        Ok(())
    }

    fn open_run_path_overlay(&mut self) -> Result<()> {
        let buffer = self.run_form.value(self.run_form.selected);
        let directories = Self::path_overlay_directories(&buffer)?;
        self.overlay = Some(Overlay::SelectPath {
            label: run_field_label(self.run_form.selected).to_string(),
            buffer,
            selected: 0,
            directories,
        });
        Ok(())
    }

    fn refresh_path_overlay(
        buffer: &str,
        directories: &mut Vec<String>,
        selected: &mut usize,
    ) -> Result<()> {
        *directories = Self::path_overlay_directories(buffer)?;
        *selected = (*selected).min(directories.len().saturating_sub(1));
        Ok(())
    }

    fn path_overlay_directories(buffer: &str) -> Result<Vec<String>> {
        let cwd = env::current_dir()?;
        Ok(list_relative_directories(&cwd, buffer))
    }
}
