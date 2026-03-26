use std::{env, fs, path::Path};

use crate::error::{AppError, Result};

use super::{
    paths::{LATEST_RUN_FILE, MANIFEST_FILE, resume_workspace_root, runs_root_in},
    queries::{list_runs_from_cache_root, list_runs_in},
    store::read_json,
    types::{RunManifest, RunStage, RunWorkspace},
};

impl RunWorkspace {
    pub fn remove_runs(run_ids: &[String]) -> Result<Vec<String>> {
        let cwd = env::current_dir()?;
        Self::remove_runs_in(&cwd, run_ids)
    }

    pub fn clear_incomplete_runs() -> Result<Vec<String>> {
        let cwd = env::current_dir()?;
        Self::clear_incomplete_runs_in(&cwd)
    }

    pub fn clear_all_runs() -> Result<Vec<String>> {
        let cwd = env::current_dir()?;
        Self::clear_all_runs_in(&cwd)
    }

    fn remove_runs_in(base_dir: &Path, run_ids: &[String]) -> Result<Vec<String>> {
        if run_ids.is_empty() {
            return Ok(Vec::new());
        }

        let runs_root = runs_root_in(base_dir)?;
        let mut removed = Vec::with_capacity(run_ids.len());
        for run_id in run_ids {
            let root_dir = runs_root.join(run_id);
            if !root_dir.exists() {
                return Err(AppError::Execution(format!(
                    "run state '{}' not found under {}",
                    run_id,
                    root_dir.display()
                )));
            }

            let manifest: RunManifest = read_json(&root_dir.join(MANIFEST_FILE))?;
            if manifest.cwd != base_dir {
                return Err(AppError::Execution(format!(
                    "run '{}' does not belong to working directory {}",
                    run_id,
                    base_dir.display()
                )));
            }

            fs::remove_dir_all(&root_dir)?;
            removed.push(run_id.clone());
        }

        refresh_latest_run(base_dir)?;
        Ok(removed)
    }

    fn clear_incomplete_runs_in(base_dir: &Path) -> Result<Vec<String>> {
        let run_ids = list_runs_in(base_dir)?
            .into_iter()
            .filter(|run| run.last_completed_stage != Some(RunStage::Completed))
            .map(|run| run.run_id)
            .collect::<Vec<_>>();
        Self::remove_runs_in(base_dir, &run_ids)
    }

    fn clear_all_runs_in(base_dir: &Path) -> Result<Vec<String>> {
        let run_ids = list_runs_in(base_dir)?
            .into_iter()
            .map(|run| run.run_id)
            .collect::<Vec<_>>();
        Self::remove_runs_in(base_dir, &run_ids)
    }

    #[cfg(test)]
    pub(crate) fn remove_runs_with_cache_root(
        base_dir: &Path,
        cache_root: &Path,
        run_ids: &[String],
    ) -> Result<Vec<String>> {
        remove_runs_with_cache_root(base_dir, cache_root, run_ids)
    }

    #[cfg(test)]
    pub(crate) fn clear_incomplete_runs_with_cache_root(
        base_dir: &Path,
        cache_root: &Path,
    ) -> Result<Vec<String>> {
        let run_ids = list_runs_from_cache_root(base_dir, cache_root)?
            .into_iter()
            .filter(|run| run.last_completed_stage != Some(RunStage::Completed))
            .map(|run| run.run_id)
            .collect::<Vec<_>>();
        remove_runs_with_cache_root(base_dir, cache_root, &run_ids)
    }

    #[cfg(test)]
    pub(crate) fn clear_all_runs_with_cache_root(
        base_dir: &Path,
        cache_root: &Path,
    ) -> Result<Vec<String>> {
        let run_ids = list_runs_from_cache_root(base_dir, cache_root)?
            .into_iter()
            .map(|run| run.run_id)
            .collect::<Vec<_>>();
        remove_runs_with_cache_root(base_dir, cache_root, &run_ids)
    }
}

fn refresh_latest_run(base_dir: &Path) -> Result<()> {
    let cache_root = resume_workspace_root(base_dir)?;
    let latest_path = cache_root.join(LATEST_RUN_FILE);
    let next_latest = list_runs_from_cache_root(base_dir, &cache_root)?
        .into_iter()
        .next()
        .map(|run| run.run_id);

    match next_latest {
        Some(run_id) => fs::write(latest_path, run_id)?,
        None => match fs::remove_file(&latest_path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(AppError::Io(err)),
        },
    }

    Ok(())
}

#[cfg(test)]
fn remove_runs_with_cache_root(
    base_dir: &Path,
    cache_root: &Path,
    run_ids: &[String],
) -> Result<Vec<String>> {
    if run_ids.is_empty() {
        return Ok(Vec::new());
    }

    let runs_root = cache_root.join(super::paths::RUNS_DIR);
    let mut removed = Vec::with_capacity(run_ids.len());
    for run_id in run_ids {
        let root_dir = runs_root.join(run_id);
        if !root_dir.exists() {
            return Err(AppError::Execution(format!(
                "run state '{}' not found under {}",
                run_id,
                root_dir.display()
            )));
        }

        let manifest: RunManifest = read_json(&root_dir.join(MANIFEST_FILE))?;
        if manifest.cwd != base_dir {
            return Err(AppError::Execution(format!(
                "run '{}' does not belong to working directory {}",
                run_id,
                base_dir.display()
            )));
        }

        fs::remove_dir_all(&root_dir)?;
        removed.push(run_id.clone());
    }

    refresh_latest_run_with_cache_root(base_dir, cache_root)?;
    Ok(removed)
}

#[cfg(test)]
fn refresh_latest_run_with_cache_root(base_dir: &Path, cache_root: &Path) -> Result<()> {
    let latest_path = cache_root.join(LATEST_RUN_FILE);
    let next_latest = list_runs_from_cache_root(base_dir, cache_root)?
        .into_iter()
        .next()
        .map(|run| run.run_id);

    match next_latest {
        Some(run_id) => fs::write(latest_path, run_id)?,
        None => match fs::remove_file(&latest_path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(AppError::Io(err)),
        },
    }

    Ok(())
}
