use std::{
    collections::hash_map::DefaultHasher,
    env, fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    process,
    time::{SystemTime, UNIX_EPOCH},
};

use clap::ValueEnum;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    config,
    config::AppConfig,
    error::{AppError, Result},
    papers::{PaperText, PdfCandidate},
    report::RunReport,
};

const RESUME_ROOT_DIR: &str = "resume";
const RUNS_DIR: &str = "runs";
const LATEST_RUN_FILE: &str = "latest_run";
const MANIFEST_FILE: &str = "manifest.json";
const CONFIG_FILE: &str = "config.json";
const REPORT_FILE: &str = "report.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[value(rename_all = "kebab-case")]
pub enum RunStage {
    DiscoverInput,
    DiscoverOutput,
    Dedupe,
    FilterSize,
    ExtractText,
    BuildLlmClient,
    ExtractKeywords,
    SynthesizeCategories,
    InspectOutput,
    GeneratePlacements,
    BuildPlan,
    ExecutePlan,
    #[value(skip)]
    Completed,
}

impl RunStage {
    pub fn description(self) -> &'static str {
        match self {
            Self::DiscoverInput => "Discover input PDFs",
            Self::DiscoverOutput => "Discover existing output PDFs",
            Self::Dedupe => "Deduplicate candidate PDFs",
            Self::FilterSize => "Filter oversized PDFs",
            Self::ExtractText => "Extract text and preprocess papers",
            Self::BuildLlmClient => "Build LLM client",
            Self::ExtractKeywords => "Extract keywords",
            Self::SynthesizeCategories => "Synthesize categories",
            Self::InspectOutput => "Inspect merged taxonomy",
            Self::GeneratePlacements => "Generate placements",
            Self::BuildPlan => "Build file move plan",
            Self::ExecutePlan => "Execute plan",
            Self::Completed => "Complete run",
        }
    }

    pub(crate) fn file_name(self) -> Option<&'static str> {
        match self {
            Self::DiscoverInput => Some("01-discover-input.json"),
            Self::DiscoverOutput => Some("02-discover-output.json"),
            Self::Dedupe => Some("03-dedupe.json"),
            Self::FilterSize => Some("04-filter-size.json"),
            Self::ExtractText => Some("05-extract-text.json"),
            Self::BuildLlmClient => None,
            Self::ExtractKeywords => Some("06-extract-keywords.json"),
            Self::SynthesizeCategories => Some("07-synthesize-categories.json"),
            Self::InspectOutput => Some("08-inspect-output.json"),
            Self::GeneratePlacements => Some("09-generate-placements.json"),
            Self::BuildPlan => Some("10-build-plan.json"),
            Self::ExecutePlan => None,
            Self::Completed => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageFailure {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterSizeState {
    pub accepted: Vec<PdfCandidate>,
    pub skipped: Vec<PdfCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractTextState {
    pub papers: Vec<PaperText>,
    pub failures: Vec<StageFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunManifest {
    run_id: String,
    created_unix_ms: u128,
    cwd: PathBuf,
    last_completed_stage: Option<RunStage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSummary {
    pub run_id: String,
    pub created_unix_ms: u128,
    pub cwd: PathBuf,
    pub last_completed_stage: Option<RunStage>,
    pub is_latest: bool,
}

#[derive(Debug, Clone)]
pub struct RunWorkspace {
    root_dir: PathBuf,
    manifest: RunManifest,
}

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

    pub fn list_runs() -> Result<Vec<RunSummary>> {
        let cwd = env::current_dir()?;
        Self::list_runs_in(&cwd)
    }

    pub fn remove_runs(run_ids: &[String]) -> Result<Vec<String>> {
        let cwd = env::current_dir()?;
        Self::remove_runs_in(&cwd, run_ids)
    }

    pub fn clear_incomplete_runs() -> Result<Vec<String>> {
        let cwd = env::current_dir()?;
        Self::clear_incomplete_runs_in(&cwd)
    }

    pub fn runs_root() -> Result<PathBuf> {
        let cwd = env::current_dir()?;
        runs_root_in(&cwd)
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

    fn create_with_cache_root(
        base_dir: &Path,
        cache_root: &Path,
        config: &AppConfig,
    ) -> Result<Self> {
        let runs_root = cache_root.join(RUNS_DIR);
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
    fn open_latest_with_cache_root(base_dir: &Path, cache_root: &Path) -> Result<Self> {
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

    fn open_in(base_dir: &Path, run_id: &str) -> Result<Self> {
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

    fn list_runs_in(base_dir: &Path) -> Result<Vec<RunSummary>> {
        let runs_root = runs_root_in(base_dir)?;
        if !runs_root.exists() {
            return Ok(Vec::new());
        }
        list_runs_from_cache_root(base_dir, &resume_workspace_root(base_dir)?)
    }

    #[cfg(test)]
    fn open_with_cache_root(base_dir: &Path, cache_root: &Path, run_id: &str) -> Result<Self> {
        let root_dir = cache_root.join(RUNS_DIR).join(run_id);
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

    #[cfg(test)]
    fn list_runs_with_cache_root(base_dir: &Path, cache_root: &Path) -> Result<Vec<RunSummary>> {
        list_runs_from_cache_root(base_dir, cache_root)
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
        let run_ids = Self::list_runs_in(base_dir)?
            .into_iter()
            .filter(|run| run.last_completed_stage != Some(RunStage::Completed))
            .map(|run| run.run_id)
            .collect::<Vec<_>>();
        Self::remove_runs_in(base_dir, &run_ids)
    }

    #[cfg(test)]
    fn remove_runs_with_cache_root(
        base_dir: &Path,
        cache_root: &Path,
        run_ids: &[String],
    ) -> Result<Vec<String>> {
        remove_runs_with_cache_root(base_dir, cache_root, run_ids)
    }

    #[cfg(test)]
    fn clear_incomplete_runs_with_cache_root(
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
}

fn resume_cache_root() -> Result<PathBuf> {
    config::xdg_cache_dir()
        .ok_or_else(|| AppError::Config("could not resolve XDG cache directory".to_string()))
}

fn resume_workspace_root(base_dir: &Path) -> Result<PathBuf> {
    Ok(resume_cache_root()?
        .join(RESUME_ROOT_DIR)
        .join(workspace_cache_key(base_dir)))
}

fn runs_root_in(base_dir: &Path) -> Result<PathBuf> {
    Ok(resume_workspace_root(base_dir)?.join(RUNS_DIR))
}

fn latest_run_path_in(base_dir: &Path) -> Result<PathBuf> {
    Ok(resume_workspace_root(base_dir)?.join(LATEST_RUN_FILE))
}

fn workspace_cache_key(base_dir: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    base_dir.to_string_lossy().hash(&mut hasher);
    format!("cwd-{:016x}", hasher.finish())
}

fn unique_run_dir(runs_root: &Path, created_unix_ms: u128) -> (String, PathBuf) {
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

fn list_runs_from_cache_root(base_dir: &Path, cache_root: &Path) -> Result<Vec<RunSummary>> {
    let runs_root = cache_root.join(RUNS_DIR);
    if !runs_root.exists() {
        return Ok(Vec::new());
    }

    let latest_run_id = fs::read_to_string(cache_root.join(LATEST_RUN_FILE))
        .ok()
        .map(|run_id| run_id.trim().to_string());
    let mut runs = Vec::new();
    for entry in fs::read_dir(&runs_root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        let root_dir = entry.path();
        let manifest_path = root_dir.join(MANIFEST_FILE);
        if !manifest_path.exists() {
            continue;
        }

        let manifest: RunManifest = read_json(&manifest_path)?;
        if manifest.cwd != base_dir {
            continue;
        }

        let is_latest = latest_run_id
            .as_deref()
            .is_some_and(|latest| latest == manifest.run_id);
        runs.push(RunSummary {
            run_id: manifest.run_id,
            created_unix_ms: manifest.created_unix_ms,
            cwd: manifest.cwd,
            last_completed_stage: manifest.last_completed_stage,
            is_latest,
        });
    }

    runs.sort_by(|left, right| right.created_unix_ms.cmp(&left.created_unix_ms));
    Ok(runs)
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

    let runs_root = cache_root.join(RUNS_DIR);
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

    Ok(removed)
}

fn write_json<T>(path: &Path, value: &T) -> Result<()>
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

fn read_json<T>(path: &Path) -> Result<T>
where
    T: DeserializeOwned,
{
    let raw = fs::read(path)?;
    Ok(serde_json::from_slice(&raw)?)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{FilterSizeState, RunStage, RunWorkspace, StageFailure};
    use crate::config::AppConfig;
    use crate::llm::LlmProvider;
    use crate::papers::PdfCandidate;
    use crate::placement::PlacementMode;
    use crate::report::RunReport;
    use crate::taxonomy::{KeywordBatchProgress, TaxonomyMode};

    fn sample_config() -> AppConfig {
        AppConfig {
            input: "/tmp/in".into(),
            output: "/tmp/out".into(),
            recursive: false,
            max_file_size_mb: 8,
            page_cutoff: 5,
            pdf_extract_workers: 4,
            category_depth: 2,
            taxonomy_mode: TaxonomyMode::BatchMerge,
            taxonomy_batch_size: 3,
            placement_batch_size: 25,
            placement_mode: PlacementMode::ExistingOnly,
            rebuild: false,
            dry_run: true,
            llm_provider: LlmProvider::Gemini,
            llm_model: "gemini-3-flash-preview".to_string(),
            llm_base_url: None,
            api_key: Some("secret".to_string()),
            keyword_batch_size: 50,
            batch_start_delay_ms: 100,
            subcategories_suggestion_number: 5,
            verbose: false,
            debug: false,
            quiet: false,
        }
    }

    #[test]
    fn persists_and_recovers_latest_run() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let cfg = sample_config();

        let workspace = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create workspace");
        let reopened = RunWorkspace::open_latest_with_cache_root(dir.path(), &cache_root)
            .expect("open latest");

        assert_eq!(workspace.run_id(), reopened.run_id());
        assert_eq!(
            reopened.load_config().expect("load config").llm_model,
            "gemini-3-flash-preview"
        );
    }

    #[test]
    fn stage_round_trip_works() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let cfg = sample_config();
        let mut workspace = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create workspace");
        let state = FilterSizeState {
            accepted: vec![PdfCandidate {
                path: "/tmp/in/a.pdf".into(),
                size_bytes: 123,
            }],
            skipped: vec![PdfCandidate {
                path: "/tmp/in/b.pdf".into(),
                size_bytes: 456,
            }],
        };

        workspace
            .save_stage(RunStage::FilterSize, &state)
            .expect("save stage");

        let loaded = workspace
            .load_stage::<FilterSizeState>(RunStage::FilterSize)
            .expect("load stage")
            .expect("stage should exist");
        assert_eq!(loaded.accepted.len(), 1);
        assert_eq!(loaded.skipped.len(), 1);
        assert_eq!(workspace.last_completed_stage(), Some(RunStage::FilterSize));
    }

    #[test]
    fn missing_stage_returns_none() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let cfg = sample_config();
        let workspace = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create workspace");

        let loaded = workspace
            .load_stage::<Vec<StageFailure>>(RunStage::ExtractText)
            .expect("load stage");

        assert!(loaded.is_none());
    }

    #[test]
    fn report_round_trip_works() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let cfg = sample_config();
        let workspace = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create workspace");
        let mut report = RunReport::new(true);
        report.scanned = 3;
        report.llm_usage.keywords.call_count = 2;

        workspace.save_report(&report).expect("save report");

        let loaded = workspace
            .load_report()
            .expect("load report")
            .expect("report should exist");
        assert_eq!(loaded.scanned, 3);
        assert_eq!(loaded.llm_usage.keywords.call_count, 2);
    }

    #[test]
    fn artifact_round_trip_and_removal_work() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let cfg = sample_config();
        let workspace = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create workspace");
        let mut progress = KeywordBatchProgress::default();
        progress.usage.call_count = 2;

        workspace
            .save_artifact("taxonomy-progress.json", &progress)
            .expect("save artifact");

        let loaded = workspace
            .load_artifact::<KeywordBatchProgress>("taxonomy-progress.json")
            .expect("load artifact")
            .expect("artifact should exist");
        assert_eq!(loaded.usage.call_count, 2);

        workspace
            .remove_artifact("taxonomy-progress.json")
            .expect("remove artifact");
        assert!(
            workspace
                .load_artifact::<KeywordBatchProgress>("taxonomy-progress.json")
                .expect("reload artifact")
                .is_none()
        );
    }

    #[test]
    fn lists_runs_newest_first_and_marks_latest() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let cfg = sample_config();

        let mut first = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create first");
        first
            .mark_stage(RunStage::ExtractText)
            .expect("mark first stage");
        let second = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create second");

        let runs =
            RunWorkspace::list_runs_with_cache_root(dir.path(), &cache_root).expect("list runs");

        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].run_id, second.run_id());
        assert!(runs[0].is_latest);
        assert_eq!(runs[1].run_id, first.run_id());
        assert_eq!(runs[1].last_completed_stage, Some(RunStage::ExtractText));
        assert!(!runs[1].is_latest);
    }

    #[test]
    fn removing_latest_run_repoints_latest_pointer() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let cfg = sample_config();

        let first = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create first");
        let second = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create second");

        let removed = RunWorkspace::remove_runs_with_cache_root(
            dir.path(),
            &cache_root,
            &[second.run_id().to_string()],
        )
        .expect("remove latest");

        assert_eq!(removed, vec![second.run_id().to_string()]);
        let runs =
            RunWorkspace::list_runs_with_cache_root(dir.path(), &cache_root).expect("list runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, first.run_id());
        assert!(runs[0].is_latest);
    }

    #[test]
    fn removing_last_run_clears_latest_pointer() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let cfg = sample_config();

        let run = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create run");

        RunWorkspace::remove_runs_with_cache_root(
            dir.path(),
            &cache_root,
            &[run.run_id().to_string()],
        )
        .expect("remove only run");

        let latest_path = cache_root.join("latest_run");
        assert!(!latest_path.exists());
    }

    #[test]
    fn clear_incomplete_runs_preserves_completed_runs() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let cfg = sample_config();

        let mut incomplete = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create incomplete");
        incomplete
            .mark_stage(RunStage::ExtractText)
            .expect("mark incomplete stage");

        let mut completed = RunWorkspace::create_with_cache_root(dir.path(), &cache_root, &cfg)
            .expect("create completed");
        completed.mark_completed().expect("mark completed");

        let removed = RunWorkspace::clear_incomplete_runs_with_cache_root(dir.path(), &cache_root)
            .expect("clear incomplete");

        assert_eq!(removed, vec![incomplete.run_id().to_string()]);
        let runs =
            RunWorkspace::list_runs_with_cache_root(dir.path(), &cache_root).expect("list runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, completed.run_id());
        assert_eq!(runs[0].last_completed_stage, Some(RunStage::Completed));
        assert!(runs[0].is_latest);
    }
}
