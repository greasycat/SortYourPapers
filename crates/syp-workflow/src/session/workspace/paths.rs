use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    process,
};

use crate::{
    config,
    error::{AppError, Result},
};

pub(super) const RESUME_ROOT_DIR: &str = "resume";
pub(super) const RUNS_DIR: &str = "runs";
pub(super) const LATEST_RUN_FILE: &str = "latest_run";
pub(super) const MANIFEST_FILE: &str = "manifest.json";
pub(super) const CONFIG_FILE: &str = "config.json";
pub(super) const REPORT_FILE: &str = "report.json";

pub(super) fn resume_cache_root() -> Result<PathBuf> {
    config::xdg_cache_dir()
        .ok_or_else(|| AppError::Config("could not resolve XDG cache directory".to_string()))
}

pub(super) fn resume_workspace_root(base_dir: &Path) -> Result<PathBuf> {
    Ok(resume_cache_root()?
        .join(RESUME_ROOT_DIR)
        .join(workspace_cache_key(base_dir)))
}

pub(super) fn runs_root_in(base_dir: &Path) -> Result<PathBuf> {
    Ok(resume_workspace_root(base_dir)?.join(RUNS_DIR))
}

pub(super) fn latest_run_path_in(base_dir: &Path) -> Result<PathBuf> {
    Ok(resume_workspace_root(base_dir)?.join(LATEST_RUN_FILE))
}

fn workspace_cache_key(base_dir: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    base_dir.to_string_lossy().hash(&mut hasher);
    format!("cwd-{:016x}", hasher.finish())
}

pub(super) fn unique_run_dir(runs_root: &Path, created_unix_ms: u128) -> (String, PathBuf) {
    let base_id = format!("run-{}-{created_unix_ms}", process::id());
    let mut suffix = 0usize;
    loop {
        let run_id = if suffix == 0 {
            base_id.clone()
        } else {
            format!("{base_id}-{suffix}")
        };
        let root_dir = runs_root.join(&run_id);
        if !root_dir.exists() {
            return (run_id, root_dir);
        }
        suffix += 1;
    }
}
