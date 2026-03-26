use std::{
    env, fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Serialize, de::DeserializeOwned};

use crate::{
    config::AppConfig,
    error::{AppError, Result},
    report::RunReport,
};

use super::{
    paths::{
        CONFIG_FILE, LATEST_RUN_FILE, MANIFEST_FILE, REPORT_FILE, latest_run_path_in,
        resume_workspace_root, runs_root_in, unique_run_dir,
    },
    types::{RunManifest, RunStage, RunWorkspace},
};

impl RunWorkspace {
    pub fn create(config: &AppConfig) -> Result<Self> {
        let cwd = env::current_dir()?;
        Self::create_in(&cwd, config)
    }

    pub fn open_latest() -> Result<Self> {
        let cwd = env::current_dir()?;
        Self::open_latest_in(&cwd)
    }

    pub fn open(run_id: &str) -> Result<Self> {
        let cwd = env::current_dir()?;
        Self::open_in(&cwd, run_id)
    }

    pub fn load_config(&self) -> Result<AppConfig> {
        read_json(&self.root_dir.join(CONFIG_FILE))
    }

    pub fn load_report(&self) -> Result<Option<RunReport>> {
        let path = self.root_dir.join(REPORT_FILE);
        if !path.exists() {
            return Ok(None);
        }
        read_json(&path).map(Some)
    }

    pub fn load_stage<T>(&self, stage: RunStage) -> Result<Option<T>>
    where
        T: DeserializeOwned,
    {
        let Some(file_name) = stage.file_name() else {
            return Ok(None);
        };
        let path = self.root_dir.join(file_name);
        if !path.exists() {
            return Ok(None);
        }
        read_json(&path).map(Some)
    }

    pub fn save_stage<T>(&mut self, stage: RunStage, value: &T) -> Result<()>
    where
        T: Serialize,
    {
        let file_name = stage.file_name().ok_or_else(|| {
            AppError::Execution(format!("stage {stage:?} does not persist a file"))
        })?;
        write_json(&self.root_dir.join(file_name), value)?;
        self.mark_stage(stage)
    }

    pub fn save_report(&self, report: &RunReport) -> Result<()> {
        write_json(&self.root_dir.join(REPORT_FILE), report)
    }

    pub fn load_artifact<T>(&self, file_name: &str) -> Result<Option<T>>
    where
        T: DeserializeOwned,
    {
        let path = self.root_dir.join(file_name);
        if !path.exists() {
            return Ok(None);
        }
        read_json(&path).map(Some)
    }

    pub fn save_artifact<T>(&self, file_name: &str, value: &T) -> Result<()>
    where
        T: Serialize,
    {
        write_json(&self.root_dir.join(file_name), value)
    }

    pub fn remove_artifact(&self, file_name: &str) -> Result<()> {
        match fs::remove_file(self.root_dir.join(file_name)) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(AppError::Io(err)),
        }
    }

    pub fn remove_stage_file(&self, stage: RunStage) -> Result<()> {
        let Some(file_name) = stage.file_name() else {
            return Ok(());
        };
        match fs::remove_file(self.root_dir.join(file_name)) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(AppError::Io(err)),
        }
    }

    pub fn mark_stage(&mut self, stage: RunStage) -> Result<()> {
        self.manifest.last_completed_stage = Some(stage);
        write_json(&self.root_dir.join(MANIFEST_FILE), &self.manifest)
    }

    pub fn set_last_completed_stage(&mut self, stage: Option<RunStage>) -> Result<()> {
        self.manifest.last_completed_stage = stage;
        write_json(&self.root_dir.join(MANIFEST_FILE), &self.manifest)
    }

    pub fn mark_completed(&mut self) -> Result<()> {
        self.mark_stage(RunStage::Completed)
    }

    pub fn last_completed_stage(&self) -> Option<RunStage> {
        self.manifest.last_completed_stage
    }

    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }

    pub fn run_id(&self) -> &str {
        &self.manifest.run_id
    }

    fn create_in(base_dir: &Path, config: &AppConfig) -> Result<Self> {
        let cache_root = resume_workspace_root(base_dir)?;
        Self::create_with_cache_root(base_dir, &cache_root, config)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn create_with_cache_root(
        base_dir: &Path,
        cache_root: &Path,
        config: &AppConfig,
    ) -> Result<Self> {
        let runs_root = cache_root.join(super::paths::RUNS_DIR);
        fs::create_dir_all(&runs_root)?;

        let created_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|err| AppError::Execution(format!("clock drift detected: {err}")))?
            .as_millis();
        let (run_id, root_dir) = unique_run_dir(&runs_root, created_unix_ms);
        fs::create_dir_all(&root_dir)?;

        let manifest = RunManifest {
            run_id: run_id.clone(),
            created_unix_ms,
            cwd: base_dir.to_path_buf(),
            last_completed_stage: None,
        };

        write_json(&root_dir.join(CONFIG_FILE), config)?;
        write_json(&root_dir.join(MANIFEST_FILE), &manifest)?;
        fs::write(cache_root.join(LATEST_RUN_FILE), &run_id)?;

        Ok(Self { root_dir, manifest })
    }

    #[cfg(test)]
    pub(crate) fn create_with_cache_root_for_tests(
        base_dir: &Path,
        cache_root: &Path,
        config: &AppConfig,
    ) -> Result<Self> {
        Self::create_with_cache_root(base_dir, cache_root, config)
    }

    fn open_latest_in(base_dir: &Path) -> Result<Self> {
        let latest_path = latest_run_path_in(base_dir)?;
        let run_id = fs::read_to_string(&latest_path).map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                AppError::Execution(format!(
                    "no resumable run found at {}",
                    latest_path.display()
                ))
            } else {
                AppError::Io(err)
            }
        })?;
        Self::open_in(base_dir, run_id.trim())
    }

    #[cfg(test)]
    pub(crate) fn open_latest_with_cache_root(base_dir: &Path, cache_root: &Path) -> Result<Self> {
        let latest_path = cache_root.join(LATEST_RUN_FILE);
        let run_id = fs::read_to_string(&latest_path).map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                AppError::Execution(format!(
                    "no resumable run found at {}",
                    latest_path.display()
                ))
            } else {
                AppError::Io(err)
            }
        })?;
        Self::open_with_cache_root(base_dir, cache_root, run_id.trim())
    }

    pub(super) fn open_in(base_dir: &Path, run_id: &str) -> Result<Self> {
        let root_dir = runs_root_in(base_dir)?.join(run_id);
        if !root_dir.exists() {
            return Err(AppError::Execution(format!(
                "run state '{}' not found under {}",
                run_id,
                root_dir.display()
            )));
        }
        let manifest: RunManifest = read_json(&root_dir.join(MANIFEST_FILE))?;
        Ok(Self { root_dir, manifest })
    }

    #[cfg(test)]
    pub(crate) fn open_with_cache_root(
        base_dir: &Path,
        cache_root: &Path,
        run_id: &str,
    ) -> Result<Self> {
        let root_dir = cache_root.join(super::paths::RUNS_DIR).join(run_id);
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
        Ok(Self { root_dir, manifest })
    }
}

pub(super) fn write_json<T>(path: &Path, value: &T) -> Result<()>
where
    T: Serialize,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(value)?;
    fs::write(&tmp_path, bytes)?;
    fs::rename(tmp_path, path)?;
    Ok(())
}

pub(super) fn read_json<T>(path: &Path) -> Result<T>
where
    T: DeserializeOwned,
{
    let raw = fs::read(path)?;
    Ok(serde_json::from_slice(&raw)?)
}
