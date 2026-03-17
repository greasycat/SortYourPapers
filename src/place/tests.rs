use std::{
    collections::VecDeque,
    path::PathBuf,
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
    OutputSnapshot, PlacementOptions, generate_placements,
    prompts::{
        build_allowed_targets, build_file_context, build_placement_prompt,
        format_placement_request_debug_message,
    },
    validation::{path_depth, validate_placements},
};
use crate::{
    error::{AppError, Result},
    llm::{LlmClient, LlmResponse},
    logging::Verbosity,
    models::{
        CategoryTree, KeywordSet, LlmCallMetrics, PaperText, PlacementDecision, PlacementMode,
        PreliminaryCategoryPair,
    },
};

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
    async fn chat(&self, _system_prompt: &str, _user_prompt: &str) -> Result<LlmResponse> {
        let mut calls = self.calls.lock().await;
        *calls += 1;
        drop(calls);

        let mut responses = self.responses.lock().await;
        responses
            .pop_front()
            .map(|content| llm_response(&content))
            .ok_or_else(|| AppError::Execution("stub client ran out of responses".to_string()))
    }
}

#[async_trait]
impl LlmClient for ConcurrentProbeClient {
    async fn chat(&self, _system_prompt: &str, user_prompt: &str) -> Result<LlmResponse> {
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

        let response = if user_prompt.contains("\"file_id\":\"f1\"") {
            Ok(llm_response(
                &serde_json::json!({
                    "placements": [
                        {
                            "file_id": "f1",
                            "target_rel_path": "."
                        }
                    ]
                })
                .to_string(),
            ))
        } else if user_prompt.contains("\"file_id\":\"f2\"") {
            Ok(llm_response(
                &serde_json::json!({
                    "placements": [
                        {
                            "file_id": "f2",
                            "target_rel_path": "."
                        }
                    ]
                })
                .to_string(),
            ))
        } else if user_prompt.contains("\"file_id\":\"f3\"") {
            Ok(llm_response(
                &serde_json::json!({
                    "placements": [
                        {
                            "file_id": "f3",
                            "target_rel_path": "."
                        }
                    ]
                })
                .to_string(),
            ))
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

#[test]
fn placement_decision_deserializes_legacy_optional_fields() {
    let placement: PlacementDecision = serde_json::from_value(serde_json::json!({
        "file_id": "f1",
        "target_rel_path": ".",
        "confidence": 0.8,
        "rationale": "legacy"
    }))
    .expect("legacy placement should deserialize");

    assert_eq!(placement.file_id, "f1");
    assert_eq!(placement.target_rel_path, ".");
}

#[test]
fn placement_prompt_uses_allowed_targets_without_extra_context() {
    let papers = vec![PaperText {
        file_id: "f1".to_string(),
        path: PathBuf::from("/tmp/p1.pdf"),
        extracted_text: "x".to_string(),
        llm_ready_text: "x".to_string(),
        pages_read: 1,
    }];
    let keyword_sets = [KeywordSet {
        file_id: "f1".to_string(),
        keywords: vec!["vision".to_string()],
    }];
    let keyword_map = keyword_sets
        .iter()
        .map(|set| (set.file_id.as_str(), set.keywords.as_slice()))
        .collect();
    let preliminary_pairs = [PreliminaryCategoryPair {
        file_id: "f1".to_string(),
        preliminary_categories_k_depth: "Vision/Detection".to_string(),
    }];
    let preliminary_map = preliminary_pairs
        .iter()
        .map(|pair| {
            (
                pair.file_id.as_str(),
                pair.preliminary_categories_k_depth.as_str(),
            )
        })
        .collect();
    let file_context = build_file_context(&papers, &keyword_map, &preliminary_map);
    let allowed_targets = build_allowed_targets(
        &[CategoryTree {
            name: "Vision".to_string(),
            children: vec![],
        }],
        &OutputSnapshot {
            is_empty: false,
            existing_folders: vec![".".to_string(), "Existing".to_string()],
            tree_map: "ignored".to_string(),
        },
        PlacementMode::ExistingOnly,
        2,
    );

    let prompt = build_placement_prompt(&file_context, &allowed_targets).expect("prompt");

    assert!(prompt.contains("allowed_targets"));
    assert!(prompt.contains("\"Existing\""));
    assert!(!prompt.contains("ignored"));
    assert!(!prompt.contains("\"path\""));
    assert!(!prompt.contains("\"confidence\""));
    assert!(!prompt.contains("\"rationale\""));
    assert!(!prompt.contains("\"Vision\""));
}

#[tokio::test]
async fn generate_placements_batches_requests() {
    let client = Arc::new(StubLlmClient {
        responses: Mutex::new(VecDeque::from(vec![
            serde_json::json!({
                "placements": [
                    {
                        "file_id": "f1",
                        "target_rel_path": "."
                    },
                    {
                        "file_id": "f2",
                        "target_rel_path": "."
                    }
                ]
            })
            .to_string(),
            serde_json::json!({
                "placements": [
                    {
                        "file_id": "f3",
                        "target_rel_path": "."
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
    let preliminary_pairs = vec![
        PreliminaryCategoryPair {
            file_id: "f1".to_string(),
            preliminary_categories_k_depth: "Root/A".to_string(),
        },
        PreliminaryCategoryPair {
            file_id: "f2".to_string(),
            preliminary_categories_k_depth: "Root/B".to_string(),
        },
        PreliminaryCategoryPair {
            file_id: "f3".to_string(),
            preliminary_categories_k_depth: "Root/C".to_string(),
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

    let (placements, usage) = generate_placements(
        client.clone(),
        &papers,
        &keyword_sets,
        &preliminary_pairs,
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

    assert_eq!(usage.call_count, 2);
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
    let preliminary_pairs = vec![
        PreliminaryCategoryPair {
            file_id: "f1".to_string(),
            preliminary_categories_k_depth: "Root/A".to_string(),
        },
        PreliminaryCategoryPair {
            file_id: "f2".to_string(),
            preliminary_categories_k_depth: "Root/B".to_string(),
        },
        PreliminaryCategoryPair {
            file_id: "f3".to_string(),
            preliminary_categories_k_depth: "Root/C".to_string(),
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

    let (placements, usage) = generate_placements(
        client.clone(),
        &papers,
        &keyword_sets,
        &preliminary_pairs,
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

    assert_eq!(usage.call_count, 3);
    assert_eq!(placements.len(), 3);
    assert_eq!(client.calls.load(Ordering::SeqCst), 3);
    assert!(client.max_active_calls.load(Ordering::SeqCst) > 1);
    let started_at = client.started_at.lock().expect("started_at");
    assert_eq!(started_at.len(), 3);
    assert!(started_at[1].duration_since(started_at[0]) >= Duration::from_millis(80));
}

fn llm_response(content: &str) -> LlmResponse {
    LlmResponse {
        content: content.to_string(),
        metrics: LlmCallMetrics {
            provider: "test".to_string(),
            model: "fake".to_string(),
            endpoint_kind: "chat".to_string(),
            request_chars: 10,
            response_chars: content.chars().count() as u64,
            http_attempt_count: 1,
            ..LlmCallMetrics::default()
        },
    }
}
