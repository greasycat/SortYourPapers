use std::fs;

use crate::{
    error::{AppError, Result},
    models::{FileAction, PlanAction},
};

pub fn execute_plan(actions: &[PlanAction], dry_run: bool) -> Result<usize> {
    if dry_run {
        return Ok(0);
    }

    let mut moved = 0usize;

    for action in actions {
        match action.action {
            FileAction::Move => {
                if let Some(parent) = action.destination.parent() {
                    fs::create_dir_all(parent)?;
                }

                match fs::rename(&action.source, &action.destination) {
                    Ok(_) => {
                        moved += 1;
                    }
                    Err(rename_err) => {
                        if action.source.exists() {
                            fs::copy(&action.source, &action.destination)?;
                            fs::remove_file(&action.source)?;
                            moved += 1;
                        } else {
                            return Err(AppError::Execution(format!(
                                "failed moving '{}' -> '{}': {rename_err}",
                                action.source.display(),
                                action.destination.display()
                            )));
                        }
                    }
                }
            }
        }
    }

    Ok(moved)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::execute_plan;
    use crate::models::{FileAction, PlanAction};

    #[test]
    fn dry_run_keeps_source_file() {
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("a.pdf");
        let dest = dir.path().join("out").join("a.pdf");
        fs::write(&source, b"content").expect("write source");

        let actions = vec![PlanAction {
            source: source.clone(),
            destination: dest.clone(),
            action: FileAction::Move,
        }];

        execute_plan(&actions, true).expect("dry-run execution should succeed");

        assert!(source.exists());
        assert!(!dest.exists());
    }

    #[test]
    fn apply_moves_file() {
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("a.pdf");
        let dest = dir.path().join("out").join("a.pdf");
        fs::write(&source, b"content").expect("write source");

        let actions = vec![PlanAction {
            source: source.clone(),
            destination: dest.clone(),
            action: FileAction::Move,
        }];

        let moved = execute_plan(&actions, false).expect("apply execution should succeed");

        assert_eq!(moved, 1);
        assert!(!source.exists());
        assert!(dest.exists());
    }
}
