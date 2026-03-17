use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    path::{Component, Path},
    sync::Arc,
    time::Instant,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::task::JoinSet;
use tokio::time::{Instant as TokioInstant, sleep_until};
use walkdir::WalkDir;

use crate::{
    error::{AppError, Result},
    llm::{LlmClient, call_json_with_retry},
    logging::{Verbosity, format_duration},
    models::{CategoryTree, KeywordSet, PaperText, PlacementDecision, PlacementMode},
};

const MAX_JSON_ATTEMPTS: usize = 3;
const MAX_SEMANTIC_ATTEMPTS: usize = 3;
const MAX_CONCURRENT_PLACEMENT_BATCH_REQUESTS: usize = 4;
const PLACEMENT_LABEL: &str = "generate-placements";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputSnapshot {
    pub is_empty: bool,
    pub existing_folders: Vec<String>,
    pub tree_map: String,
}

#[derive(Debug, Deserialize)]
struct PlacementResponse {
    placements: Vec<PlacementDecision>,
}

#[derive(Debug, Clone)]
struct PreparedPlacementBatch {
    batch_index: usize,
    papers: Vec<PaperText>,
    file_context: Vec<Value>,
}

#[derive(Debug, Clone, Copy)]
pub struct PlacementOptions {
    pub batch_size: usize,
    pub batch_start_delay_ms: u64,
    pub placement_mode: PlacementMode,
    pub category_depth: u8,
    pub verbosity: Verbosity,
}

#[derive(Debug, Clone)]
struct PlacementBatchRuntime {
    categories: Arc<Vec<CategoryTree>>,
    snapshot: Arc<OutputSnapshot>,
    options: PlacementOptions,
    total_batches: usize,
}

pub fn inspect_output(output: &Path) -> Result<OutputSnapshot> {
    if !output.exists() {
        return Ok(OutputSnapshot {
            is_empty: true,
            existing_folders: vec![".".to_string()],
            tree_map: "<empty>".to_string(),
        });
    }

    let mut entries = fs::read_dir(output)?;
    let is_empty = match entries.next() {
        Some(item) => {
            let _ = item?;
            false
        }
        None => true,
    };

    let mut folders: BTreeSet<String> = BTreeSet::new();
    folders.insert(".".to_string());

    for entry in WalkDir::new(output).min_depth(1) {
        let entry = entry?;
        if entry.file_type().is_dir() {
            let rel = entry
                .path()
                .strip_prefix(output)
                .map_err(|e| AppError::Execution(format!("strip prefix failed: {e}")))?;
            folders.insert(rel.to_string_lossy().replace('\\', "/"));
        }
    }

    let tree_map = build_tree_map(output)?;

    Ok(OutputSnapshot {
        is_empty,
        existing_folders: folders.into_iter().collect(),
        tree_map,
    })
}

pub async fn generate_placements(
    client: Arc<dyn LlmClient>,
    papers: &[PaperText],
    keyword_sets: &[KeywordSet],
    categories: &[CategoryTree],
    snapshot: &OutputSnapshot,
    options: PlacementOptions,
) -> Result<Vec<PlacementDecision>> {
    if papers.is_empty() {
        return Ok(Vec::new());
    }
    if options.batch_size == 0 {
        return Err(AppError::Validation(
            "placement_batch_size must be greater than 0".to_string(),
        ));
    }

    let keyword_map: HashMap<&str, &[String]> = keyword_sets
        .iter()
        .map(|k| (k.file_id.as_str(), k.keywords.as_slice()))
        .collect();
    let total_batches = papers.len().div_ceil(options.batch_size);
    options.verbosity.stage_line(
        "placements",
        format!(
            "{} paper(s) ready; batching into {} request(s) of up to {} file(s)",
            papers.len(),
            total_batches,
            options.batch_size
        ),
    );

    let prepared_batches = papers
        .chunks(options.batch_size)
        .enumerate()
        .map(|(batch_index, batch)| PreparedPlacementBatch {
            batch_index: batch_index + 1,
            papers: batch.to_vec(),
            file_context: build_file_context(batch, &keyword_map),
        })
        .collect::<Vec<_>>();
    let runtime = PlacementBatchRuntime {
        categories: Arc::new(categories.to_vec()),
        snapshot: Arc::new(snapshot.clone()),
        options,
        total_batches,
    };
    let batch_results =
        run_placement_batches_concurrently(client, prepared_batches, runtime.clone()).await?;
    let mut all_placements = Vec::with_capacity(papers.len());
    for (_, placements) in batch_results {
        all_placements.extend(placements);
    }

    validate_placements(
        &all_placements,
        papers,
        snapshot,
        options.placement_mode,
        options.category_depth,
    )?;
    options.verbosity.success_line(
        "PLACEMENTS",
        format!(
            "completed {} placement batch(es) and collected {} decision(s)",
            total_batches,
            all_placements.len()
        ),
    );

    Ok(all_placements)
}

fn format_placement_request_debug_message(system: &str, user: &str) -> String {
    format!("{PLACEMENT_LABEL} request\nsystem:\n{system}\nuser:\n{user}")
}

async fn generate_placement_batch(
    client: &dyn LlmClient,
    batch: &PreparedPlacementBatch,
    runtime: &PlacementBatchRuntime,
) -> Result<Vec<PlacementDecision>> {
    let system = "You assign PDFs to category folders. Return strict JSON only.";
    let base_user = build_placement_prompt(
        &batch.file_context,
        runtime.categories.as_slice(),
        runtime.snapshot.as_ref(),
        runtime.options.placement_mode,
        runtime.options.category_depth,
    )?;
    let mut user = base_user.clone();
    let mut last_issue = String::new();

    for attempt in 1..=MAX_SEMANTIC_ATTEMPTS {
        if runtime.options.verbosity.debug_enabled() {
            runtime.options.verbosity.debug_line(
                "LLM",
                format!(
                    "placement batch {}/{}\n{}",
                    batch.batch_index,
                    runtime.total_batches,
                    format_placement_request_debug_message(system, &user)
                ),
            );
        }

        let response: PlacementResponse =
            call_json_with_retry(client, system, &user, MAX_JSON_ATTEMPTS).await?;
        match validate_placements(
            &response.placements,
            &batch.papers,
            runtime.snapshot.as_ref(),
            runtime.options.placement_mode,
            runtime.options.category_depth,
        ) {
            Ok(()) => return Ok(response.placements),
            Err(err) => last_issue = err.to_string(),
        }

        if attempt < MAX_SEMANTIC_ATTEMPTS {
            runtime.options.verbosity.warn_line(
                "PLACEMENTS",
                format!(
                    "batch {}/{} retry {}/{}: {}",
                    batch.batch_index,
                    runtime.total_batches,
                    attempt + 1,
                    MAX_SEMANTIC_ATTEMPTS,
                    last_issue
                ),
            );
            user = format!(
                "{base_user}\n\nYour previous response had this issue: {last_issue}.\nReturn JSON again.\nImportant: return exactly one placement for every file_id in this batch only."
            );
        }
    }

    Err(AppError::Validation(format!(
        "failed placement validation for batch {}/{}: {}",
        batch.batch_index, runtime.total_batches, last_issue
    )))
}

async fn run_placement_batches_concurrently(
    client: Arc<dyn LlmClient>,
    prepared_batches: Vec<PreparedPlacementBatch>,
    runtime: PlacementBatchRuntime,
) -> Result<Vec<(usize, Vec<PlacementDecision>)>> {
    let max_in_flight = MAX_CONCURRENT_PLACEMENT_BATCH_REQUESTS.max(1);
    let dispatch_spacing = batch_dispatch_spacing(runtime.options.batch_start_delay_ms);
    let mut pending_batches = prepared_batches.into_iter();
    let mut in_flight = JoinSet::new();
    let mut batch_results = Vec::with_capacity(runtime.total_batches);
    let mut next_dispatch_at = None;

    for _ in 0..max_in_flight {
        let Some(batch) = pending_batches.next() else {
            break;
        };
        wait_for_dispatch_slot(&mut next_dispatch_at, dispatch_spacing).await;
        spawn_placement_batch(&mut in_flight, Arc::clone(&client), batch, runtime.clone());
    }

    while let Some(join_result) = in_flight.join_next().await {
        let (batch_index, placements) = match join_result {
            Ok(Ok(result)) => result,
            Ok(Err(err)) => return Err(err),
            Err(err) => {
                return Err(AppError::Execution(format!(
                    "placement batch task failed: {err}"
                )));
            }
        };
        batch_results.push((batch_index, placements));

        if let Some(batch) = pending_batches.next() {
            wait_for_dispatch_slot(&mut next_dispatch_at, dispatch_spacing).await;
            spawn_placement_batch(&mut in_flight, Arc::clone(&client), batch, runtime.clone());
        }
    }

    batch_results.sort_by_key(|(batch_index, _)| *batch_index);
    Ok(batch_results)
}

fn spawn_placement_batch(
    join_set: &mut JoinSet<Result<(usize, Vec<PlacementDecision>)>>,
    client: Arc<dyn LlmClient>,
    batch: PreparedPlacementBatch,
    runtime: PlacementBatchRuntime,
) {
    join_set.spawn(async move {
        let started_at = Instant::now();
        let batch_span = format_paper_batch_span(&batch.papers);
        runtime.options.verbosity.stage_line(
            "placements",
            format!(
                "batch {}/{} {}",
                batch.batch_index, runtime.total_batches, batch_span
            ),
        );
        let placements = match generate_placement_batch(client.as_ref(), &batch, &runtime).await {
            Ok(placements) => placements,
            Err(err) => {
                runtime.options.verbosity.error_line(
                    "PLACEMENTS",
                    format!(
                        "batch {}/{} failed after {} {}: {}",
                        batch.batch_index,
                        runtime.total_batches,
                        format_duration(started_at.elapsed()),
                        batch_span,
                        err
                    ),
                );
                return Err(err);
            }
        };
        runtime.options.verbosity.success_line(
            "PLACEMENTS",
            format!(
                "batch {}/{} completed in {} {}",
                batch.batch_index,
                runtime.total_batches,
                format_duration(started_at.elapsed()),
                batch_span
            ),
        );
        Ok((batch.batch_index, placements))
    });
}

fn batch_dispatch_spacing(batch_start_delay_ms: u64) -> std::time::Duration {
    std::time::Duration::from_millis(batch_start_delay_ms)
}

async fn wait_for_dispatch_slot(
    next_dispatch_at: &mut Option<TokioInstant>,
    dispatch_spacing: std::time::Duration,
) {
    if let Some(deadline) = *next_dispatch_at {
        sleep_until(deadline).await;
    }
    *next_dispatch_at = Some(TokioInstant::now() + dispatch_spacing);
}

fn build_file_context(papers: &[PaperText], keyword_map: &HashMap<&str, &[String]>) -> Vec<Value> {
    papers
        .iter()
        .map(|paper| {
            let file_name = paper
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown.pdf");
            let keywords = keyword_map
                .get(paper.file_id.as_str())
                .copied()
                .unwrap_or(&[]);
            serde_json::json!({
                "file_id": paper.file_id,
                "file_name": file_name,
                "path": paper.path.to_string_lossy(),
                "keywords": keywords,
            })
        })
        .collect()
}

fn build_placement_prompt(
    file_context: &[Value],
    categories: &[CategoryTree],
    snapshot: &OutputSnapshot,
    placement_mode: PlacementMode,
    category_depth: u8,
) -> Result<String> {
    if snapshot.is_empty {
        Ok(format!(
            "Return JSON with schema:\n{{\"placements\":[{{\"file_id\":\"...\",\"target_rel_path\":\"...\",\"confidence\":0.0,\"rationale\":\"...\"}}]}}\nRules:\n- exactly one placement per file\n- target_rel_path must be a relative directory path (no file name)\n- max depth for target_rel_path is {category_depth}\n- use taxonomy context below\n- no markdown\n\ncategories:\n{}\n\nfiles:\n{}",
            serde_json::to_string_pretty(categories).map_err(AppError::from)?,
            serde_json::to_string_pretty(file_context).map_err(AppError::from)?,
        ))
    } else {
        Ok(format!(
            "Return JSON with schema:\n{{\"placements\":[{{\"file_id\":\"...\",\"target_rel_path\":\"...\",\"confidence\":0.0,\"rationale\":\"...\"}}]}}\nRules:\n- exactly one placement per file\n- target_rel_path must be a relative directory path (no file name)\n- max depth for target_rel_path is {category_depth}\n- placement_mode is {placement_mode:?}\n- if placement_mode is ExistingOnly, choose only from existing_folders\n- if placement_mode is AllowNew, you may use existing_folders or create new paths up to max depth\n- no markdown\n\nexisting_folders:\n{}\n\ncurrent_tree_map:\n{}\n\ncategory_hints:\n{}\n\nfiles:\n{}",
            serde_json::to_string_pretty(&snapshot.existing_folders).map_err(AppError::from)?,
            snapshot.tree_map,
            serde_json::to_string_pretty(categories).map_err(AppError::from)?,
            serde_json::to_string_pretty(file_context).map_err(AppError::from)?,
        ))
    }
}

fn format_paper_batch_span(papers: &[PaperText]) -> String {
    let Some(first) = papers.first() else {
        return "empty batch".to_string();
    };
    let Some(last) = papers.last() else {
        return "empty batch".to_string();
    };
    format!(
        "file_ids {}..{} ({} file(s))",
        first.file_id,
        last.file_id,
        papers.len()
    )
}

fn validate_placements(
    placements: &[PlacementDecision],
    papers: &[PaperText],
    snapshot: &OutputSnapshot,
    placement_mode: PlacementMode,
    category_depth: u8,
) -> Result<()> {
    if placements.len() != papers.len() {
        return Err(AppError::Validation(format!(
            "placements count mismatch: expected {}, got {}",
            papers.len(),
            placements.len()
        )));
    }

    let expected_ids = papers
        .iter()
        .map(|p| p.file_id.clone())
        .collect::<HashSet<_>>();
    let mut seen_ids = HashSet::new();
    let existing_folder_set = snapshot
        .existing_folders
        .iter()
        .cloned()
        .collect::<HashSet<_>>();

    for placement in placements {
        if !expected_ids.contains(&placement.file_id) {
            return Err(AppError::Validation(format!(
                "placement references unknown file_id {}",
                placement.file_id
            )));
        }
        if !seen_ids.insert(placement.file_id.clone()) {
            return Err(AppError::Validation(format!(
                "duplicate placement for file_id {}",
                placement.file_id
            )));
        }

        let rel = normalize_rel_dir(&placement.target_rel_path)?;
        let depth = path_depth(&rel);
        if depth > usize::from(category_depth) {
            return Err(AppError::Validation(format!(
                "placement path '{}' depth {} exceeds max {}",
                rel, depth, category_depth
            )));
        }

        if !snapshot.is_empty
            && placement_mode == PlacementMode::ExistingOnly
            && !existing_folder_set.contains(&rel)
        {
            return Err(AppError::Validation(format!(
                "path '{}' does not exist in output tree",
                rel
            )));
        }
    }

    Ok(())
}

fn normalize_rel_dir(raw: &str) -> Result<String> {
    let normalized = raw.trim().replace('\\', "/");
    if normalized.is_empty() {
        return Err(AppError::Validation(
            "target_rel_path cannot be empty".to_string(),
        ));
    }

    let path = Path::new(&normalized);
    if path.is_absolute() {
        return Err(AppError::Validation(
            "target_rel_path must be relative".to_string(),
        ));
    }

    for component in path.components() {
        match component {
            Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                return Err(AppError::Validation(format!(
                    "target_rel_path contains illegal segment: {}",
                    raw
                )));
            }
            _ => {}
        }
    }

    Ok(if normalized == "." {
        ".".to_string()
    } else {
        normalized.trim_matches('/').to_string()
    })
}

fn path_depth(path: &str) -> usize {
    if path == "." {
        return 0;
    }
    Path::new(path)
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .count()
}

fn build_tree_map(root: &Path) -> Result<String> {
    if !root.exists() {
        return Ok("<missing>".to_string());
    }

    let mut lines = vec![".".to_string()];

    for entry in WalkDir::new(root).min_depth(1) {
        let entry = entry?;
        let rel = entry
            .path()
            .strip_prefix(root)
            .map_err(|e| AppError::Execution(format!("strip prefix failed: {e}")))?;
        let depth = rel.components().count();
        let indent = "  ".repeat(depth.saturating_sub(1));
        let name = rel.to_string_lossy().replace('\\', "/");
        let suffix = if entry.file_type().is_dir() { "/" } else { "" };
        lines.push(format!("{indent}{name}{suffix}"));
    }

    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };

    use async_trait::async_trait;
    use tokio::sync::Mutex;
    use tokio::time::sleep;

    use super::{
        OutputSnapshot, PlacementOptions, format_placement_request_debug_message,
        generate_placements, path_depth, validate_placements,
    };
    use crate::{
        error::{AppError, Result},
        llm::LlmClient,
        logging::Verbosity,
        models::{CategoryTree, KeywordSet, PaperText, PlacementDecision, PlacementMode},
    };
    use std::path::PathBuf;

    struct StubLlmClient {
        responses: Mutex<VecDeque<String>>,
        calls: Mutex<usize>,
    }

    struct ConcurrentProbeClient {
        calls: AtomicUsize,
        active_calls: AtomicUsize,
        max_active_calls: AtomicUsize,
        started_at: std::sync::Mutex<Vec<Instant>>,
        delay: Duration,
    }

    #[async_trait]
    impl LlmClient for StubLlmClient {
        async fn chat(&self, _system_prompt: &str, _user_prompt: &str) -> Result<String> {
            let mut calls = self.calls.lock().await;
            *calls += 1;
            drop(calls);

            let mut responses = self.responses.lock().await;
            responses
                .pop_front()
                .ok_or_else(|| AppError::Execution("stub client ran out of responses".to_string()))
        }
    }

    #[async_trait]
    impl LlmClient for ConcurrentProbeClient {
        async fn chat(&self, _system_prompt: &str, _user_prompt: &str) -> Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.started_at
                .lock()
                .expect("started_at lock")
                .push(Instant::now());
            let active = self.active_calls.fetch_add(1, Ordering::SeqCst) + 1;
            let mut observed = self.max_active_calls.load(Ordering::SeqCst);
            while active > observed {
                match self.max_active_calls.compare_exchange(
                    observed,
                    active,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(actual) => observed = actual,
                }
            }

            sleep(self.delay).await;

            let response = if _user_prompt.contains("\"file_id\": \"f1\"") {
                Ok(serde_json::json!({
                    "placements": [
                        {
                            "file_id": "f1",
                            "target_rel_path": ".",
                            "confidence": 0.8,
                            "rationale": "fits root"
                        }
                    ]
                })
                .to_string())
            } else if _user_prompt.contains("\"file_id\": \"f2\"") {
                Ok(serde_json::json!({
                    "placements": [
                        {
                            "file_id": "f2",
                            "target_rel_path": ".",
                            "confidence": 0.8,
                            "rationale": "fits root"
                        }
                    ]
                })
                .to_string())
            } else if _user_prompt.contains("\"file_id\": \"f3\"") {
                Ok(serde_json::json!({
                    "placements": [
                        {
                            "file_id": "f3",
                            "target_rel_path": ".",
                            "confidence": 0.8,
                            "rationale": "fits root"
                        }
                    ]
                })
                .to_string())
            } else {
                Err(AppError::Execution(
                    "probe client could not determine requested file_id".to_string(),
                ))
            };
            self.active_calls.fetch_sub(1, Ordering::SeqCst);
            response
        }
    }

    #[test]
    fn depth_for_root_is_zero() {
        assert_eq!(path_depth("."), 0);
        assert_eq!(path_depth("a/b"), 2);
    }

    #[test]
    fn existing_only_rejects_unknown_folder() {
        let papers = vec![PaperText {
            file_id: "f1".to_string(),
            path: PathBuf::from("/tmp/p1.pdf"),
            extracted_text: "x".to_string(),
            llm_ready_text: "x".to_string(),
            pages_read: 1,
        }];
        let placements = vec![PlacementDecision {
            file_id: "f1".to_string(),
            target_rel_path: "new-folder".to_string(),
            rationale: None,
            confidence: Some(0.8),
        }];
        let snapshot = OutputSnapshot {
            is_empty: false,
            existing_folders: vec![".".to_string(), "existing".to_string()],
            tree_map: ".".to_string(),
        };

        let result = validate_placements(
            &placements,
            &papers,
            &snapshot,
            PlacementMode::ExistingOnly,
            2,
        );
        assert!(result.is_err());
    }

    #[test]
    fn debug_message_formats_placement_request() {
        let message = format_placement_request_debug_message("system prompt", "user prompt");

        assert!(message.contains("generate-placements request"));
        assert!(message.contains("system:\nsystem prompt"));
        assert!(message.contains("user:\nuser prompt"));
    }

    #[tokio::test]
    async fn generate_placements_batches_requests() {
        let client = Arc::new(StubLlmClient {
            responses: Mutex::new(VecDeque::from(vec![
                serde_json::json!({
                    "placements": [
                        {
                            "file_id": "f1",
                            "target_rel_path": ".",
                            "confidence": 0.8,
                            "rationale": "fits root"
                        },
                        {
                            "file_id": "f2",
                            "target_rel_path": ".",
                            "confidence": 0.7,
                            "rationale": "fits root"
                        }
                    ]
                })
                .to_string(),
                serde_json::json!({
                    "placements": [
                        {
                            "file_id": "f3",
                            "target_rel_path": ".",
                            "confidence": 0.9,
                            "rationale": "fits root"
                        }
                    ]
                })
                .to_string(),
            ])),
            calls: Mutex::new(0),
        });
        let papers = vec![
            PaperText {
                file_id: "f1".to_string(),
                path: PathBuf::from("/tmp/p1.pdf"),
                extracted_text: "x".to_string(),
                llm_ready_text: "x".to_string(),
                pages_read: 1,
            },
            PaperText {
                file_id: "f2".to_string(),
                path: PathBuf::from("/tmp/p2.pdf"),
                extracted_text: "x".to_string(),
                llm_ready_text: "x".to_string(),
                pages_read: 1,
            },
            PaperText {
                file_id: "f3".to_string(),
                path: PathBuf::from("/tmp/p3.pdf"),
                extracted_text: "x".to_string(),
                llm_ready_text: "x".to_string(),
                pages_read: 1,
            },
        ];
        let keyword_sets = vec![
            KeywordSet {
                file_id: "f1".to_string(),
                keywords: vec!["a".to_string()],
            },
            KeywordSet {
                file_id: "f2".to_string(),
                keywords: vec!["b".to_string()],
            },
            KeywordSet {
                file_id: "f3".to_string(),
                keywords: vec!["c".to_string()],
            },
        ];
        let categories = vec![CategoryTree {
            name: "Root".to_string(),
            children: vec![],
        }];
        let snapshot = OutputSnapshot {
            is_empty: true,
            existing_folders: vec![".".to_string()],
            tree_map: "<empty>".to_string(),
        };

        let placements = generate_placements(
            client.clone(),
            &papers,
            &keyword_sets,
            &categories,
            &snapshot,
            PlacementOptions {
                batch_size: 2,
                batch_start_delay_ms: 100,
                placement_mode: PlacementMode::AllowNew,
                category_depth: 2,
                verbosity: Verbosity::new(false, false, true),
            },
        )
        .await
        .expect("batched placement generation should succeed");

        assert_eq!(placements.len(), 3);
        assert_eq!(placements[0].file_id, "f1");
        assert_eq!(placements[2].file_id, "f3");
        assert_eq!(*client.calls.lock().await, 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn generate_placements_runs_batches_concurrently() {
        let client = Arc::new(ConcurrentProbeClient {
            calls: AtomicUsize::new(0),
            active_calls: AtomicUsize::new(0),
            max_active_calls: AtomicUsize::new(0),
            started_at: std::sync::Mutex::new(Vec::new()),
            delay: Duration::from_millis(150),
        });
        let papers = vec![
            PaperText {
                file_id: "f1".to_string(),
                path: PathBuf::from("/tmp/p1.pdf"),
                extracted_text: "x".to_string(),
                llm_ready_text: "x".to_string(),
                pages_read: 1,
            },
            PaperText {
                file_id: "f2".to_string(),
                path: PathBuf::from("/tmp/p2.pdf"),
                extracted_text: "x".to_string(),
                llm_ready_text: "x".to_string(),
                pages_read: 1,
            },
            PaperText {
                file_id: "f3".to_string(),
                path: PathBuf::from("/tmp/p3.pdf"),
                extracted_text: "x".to_string(),
                llm_ready_text: "x".to_string(),
                pages_read: 1,
            },
        ];
        let keyword_sets = vec![
            KeywordSet {
                file_id: "f1".to_string(),
                keywords: vec!["a".to_string()],
            },
            KeywordSet {
                file_id: "f2".to_string(),
                keywords: vec!["b".to_string()],
            },
            KeywordSet {
                file_id: "f3".to_string(),
                keywords: vec!["c".to_string()],
            },
        ];
        let categories = vec![CategoryTree {
            name: "Root".to_string(),
            children: vec![],
        }];
        let snapshot = OutputSnapshot {
            is_empty: true,
            existing_folders: vec![".".to_string()],
            tree_map: "<empty>".to_string(),
        };

        let placements = generate_placements(
            client.clone(),
            &papers,
            &keyword_sets,
            &categories,
            &snapshot,
            PlacementOptions {
                batch_size: 1,
                batch_start_delay_ms: 100,
                placement_mode: PlacementMode::AllowNew,
                category_depth: 2,
                verbosity: Verbosity::new(false, false, true),
            },
        )
        .await
        .expect("concurrent placement generation should succeed");

        assert_eq!(placements.len(), 3);
        assert_eq!(client.calls.load(Ordering::SeqCst), 3);
        assert!(client.max_active_calls.load(Ordering::SeqCst) > 1);
        let started_at = client.started_at.lock().expect("started_at");
        assert_eq!(started_at.len(), 3);
        assert!(started_at[1].duration_since(started_at[0]) >= Duration::from_millis(80));
    }
}
