use crate::{
    error::{AppError, Result},
    papers::SynthesizeCategoriesState,
    session::{RunStage, RunWorkspace},
    terminal::{self, Verbosity},
};

use super::selection::select_completed_run_for_review_interactively;

/// Displays the saved synthesized category tree for a completed session.
///
/// # Errors
/// Returns an error when the run cannot be selected or opened, the run is not
/// completed, or the synthesized category stage is unavailable.
pub fn review_session(run_id: Option<String>) -> Result<()> {
    let workspace = if let Some(run_id) = run_id {
        RunWorkspace::open(&run_id)?
    } else {
        let selected_run = select_completed_run_for_review_interactively()?;
        RunWorkspace::open(&selected_run)?
    };

    if workspace.last_completed_stage() != Some(RunStage::Completed) {
        return Err(AppError::Execution(format!(
            "run '{}' is not completed",
            workspace.run_id()
        )));
    }

    let categories = workspace
        .load_stage::<SynthesizeCategoriesState>(RunStage::SynthesizeCategories)?
        .ok_or_else(|| {
            AppError::Execution(format!(
                "run '{}' has no saved synthesized categories",
                workspace.run_id()
            ))
        })?;
    let verbosity = Verbosity::new(false, false, false);
    println!(
        "Reviewing {} from {}",
        workspace.run_id(),
        workspace.root_dir().display()
    );
    terminal::report::print_category_tree(&categories.categories, verbosity);
    Ok(())
}
