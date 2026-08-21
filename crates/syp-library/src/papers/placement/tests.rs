use std::{collections::VecDeque, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use syp_ai::error::{AppError as AiError, Result as AiResult};
use tokio::sync::Mutex;

use super::{
    OutputSnapshot, PlacementBatchProgress, PlacementBatchResult, PlacementOptions,
    generate_placements, generate_placements_with_progress,
    prompts::{
        build_allowed_targets, build_file_context, build_placement_prompt,
        format_placement_request_debug_message,
    },
    validation::{path_depth, validate_placements},
};
use super::{PlacementAssistance, PlacementDecision, PlacementMode};
use crate::{
    llm::{LlmCallMetrics, LlmClient, LlmResponse},
    papers::taxonomy::CategoryTree,
    papers::{KeywordSet, PaperText, PreliminaryCategoryPair},
    terminal::Verbosity,
};

struct StubLlmClient {
    responses: Mutex<VecDeque<String>>,
    calls: Mutex<usize>,
}

#[async_trait]
impl LlmClient for StubLlmClient {
    async fn chat(&self, _system_prompt: &str, user_prompt: &str) -> AiResult<LlmResponse> {
        let mut calls = self.calls.lock().await;
        *calls += 1;
        drop(calls);

        // Batches run concurrently, so they do not necessarily arrive in queue
        // order. Answer each one with the queued response naming the files it
        // actually asked about, and these tests keep checking placement
        // behaviour rather than dispatch timing.
        let mut responses = self.responses.lock().await;
        let matched = responses
            .iter()
            .position(|content| response_matches_request(content, user_prompt))
            .or(if responses.is_empty() { None } else { Some(0) });

        matched
            .and_then(|index| responses.remove(index))
            .map(|content| llm_response(&content))
            .ok_or_else(|| AiError::Execution("stub client ran out of responses".to_string()))
    }
}

/// Whether every `file_id` a queued response places was asked for by this request.
fn response_matches_request(content: &str, user_prompt: &str) -> bool {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(content) else {
        return false;
    };
    let Some(placements) = parsed.get("placements").and_then(|value| value.as_array()) else {
        return false;
    };
    if placements.is_empty() {
        return false;
    }
    placements.iter().all(|placement| {
        placement
            .get("file_id")
            .and_then(|value| value.as_str())
            .is_some_and(|file_id| user_prompt.contains(&format!("\"file_id\":\"{file_id}\"")))
    })
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
        from_page_images: false,
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
fn placement_prompt_uses_allowed_targets_without_extra_context() {
    let papers = vec![PaperText {
        file_id: "f1".to_string(),
        path: PathBuf::from("/tmp/p1.pdf"),
        extracted_text: "x".to_string(),
        llm_ready_text: "x".to_string(),
        pages_read: 1,
        from_page_images: false,
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

#[test]
fn placement_prompt_includes_embedding_shortlist_rule_when_present() {
    let prompt = build_placement_prompt(
        &[serde_json::json!({
            "file_id": "f1",
            "file_name": "paper.pdf",
            "keywords": ["vision"],
            "preliminary_categories_k_depth": "Vision/Detection",
            "embedding_ranked_targets": [
                {"target_rel_path": "Vision/Detection", "similarity": 0.91}
            ]
        })],
        &["Vision/Detection".to_string()],
    )
    .expect("prompt");

    assert!(prompt.contains("embedding_ranked_targets"));
    assert!(prompt.contains("primary candidate set"));
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
            from_page_images: false,
        },
        PaperText {
            file_id: "f2".to_string(),
            path: PathBuf::from("/tmp/p2.pdf"),
            extracted_text: "x".to_string(),
            llm_ready_text: "x".to_string(),
            pages_read: 1,
            from_page_images: false,
        },
        PaperText {
            file_id: "f3".to_string(),
            path: PathBuf::from("/tmp/p3.pdf"),
            extracted_text: "x".to_string(),
            llm_ready_text: "x".to_string(),
            pages_read: 1,
            from_page_images: false,
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
            assistance: PlacementAssistance::LlmOnly,
            placement_mode: PlacementMode::AllowNew,
            category_depth: 2,
            embedding: None,
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

#[tokio::test]
async fn generate_placements_uses_stable_batch_order() {
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
            file_id: "f3".to_string(),
            path: PathBuf::from("/tmp/p3.pdf"),
            extracted_text: "x".to_string(),
            llm_ready_text: "x".to_string(),
            pages_read: 1,
            from_page_images: false,
        },
        PaperText {
            file_id: "f1".to_string(),
            path: PathBuf::from("/tmp/p1.pdf"),
            extracted_text: "x".to_string(),
            llm_ready_text: "x".to_string(),
            pages_read: 1,
            from_page_images: false,
        },
        PaperText {
            file_id: "f2".to_string(),
            path: PathBuf::from("/tmp/p2.pdf"),
            extracted_text: "x".to_string(),
            llm_ready_text: "x".to_string(),
            pages_read: 1,
            from_page_images: false,
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
            assistance: PlacementAssistance::LlmOnly,
            placement_mode: PlacementMode::AllowNew,
            category_depth: 2,
            embedding: None,
            verbosity: Verbosity::new(false, false, true),
        },
    )
    .await
    .expect("stable placement batching should succeed");

    assert_eq!(usage.call_count, 2);
    assert_eq!(placements.len(), 3);
    assert_eq!(placements[0].file_id, "f1");
    assert_eq!(placements[1].file_id, "f2");
    assert_eq!(placements[2].file_id, "f3");
}

#[tokio::test]
async fn placement_resume_skips_saved_batches() {
    let client = Arc::new(StubLlmClient {
        responses: Mutex::new(VecDeque::from(vec![
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
            file_id: "f2".to_string(),
            path: PathBuf::from("/tmp/p2.pdf"),
            extracted_text: "x".to_string(),
            llm_ready_text: "x".to_string(),
            pages_read: 1,
            from_page_images: false,
        },
        PaperText {
            file_id: "f3".to_string(),
            path: PathBuf::from("/tmp/p3.pdf"),
            extracted_text: "x".to_string(),
            llm_ready_text: "x".to_string(),
            pages_read: 1,
            from_page_images: false,
        },
        PaperText {
            file_id: "f1".to_string(),
            path: PathBuf::from("/tmp/p1.pdf"),
            extracted_text: "x".to_string(),
            llm_ready_text: "x".to_string(),
            pages_read: 1,
            from_page_images: false,
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
    let saved_progress = PlacementBatchProgress {
        completed_batches: vec![PlacementBatchResult {
            batch_index: 1,
            file_ids: vec!["f1".to_string(), "f2".to_string()],
            placements: vec![
                PlacementDecision {
                    file_id: "f1".to_string(),
                    target_rel_path: ".".to_string(),
                },
                PlacementDecision {
                    file_id: "f2".to_string(),
                    target_rel_path: ".".to_string(),
                },
            ],
            evidence: Vec::new(),
            elapsed_ms: 10,
        }],
        usage: crate::llm::LlmUsageSummary {
            call_count: 1,
            ..crate::llm::LlmUsageSummary::default()
        },
    };

    let result = generate_placements_with_progress(
        client.clone(),
        &papers,
        &keyword_sets,
        &preliminary_pairs,
        &categories,
        &snapshot,
        PlacementOptions {
            batch_size: 2,
            batch_start_delay_ms: 100,
            assistance: PlacementAssistance::LlmOnly,
            placement_mode: PlacementMode::AllowNew,
            category_depth: 2,
            embedding: None,
            verbosity: Verbosity::new(false, false, true),
        },
        saved_progress,
        |_| Ok(()),
    )
    .await
    .expect("placement resume should succeed");

    assert_eq!(result.usage.call_count, 2);
    assert_eq!(result.placements.len(), 3);
    assert_eq!(result.placements[0].file_id, "f1");
    assert_eq!(result.placements[1].file_id, "f2");
    assert_eq!(result.placements[2].file_id, "f3");
    assert_eq!(*client.calls.lock().await, 1);
}

struct ConcurrentPlacementProbeClient {
    active_calls: std::sync::atomic::AtomicUsize,
    max_active_calls: std::sync::atomic::AtomicUsize,
    delay: std::time::Duration,
}

#[async_trait]
impl LlmClient for ConcurrentPlacementProbeClient {
    async fn chat(&self, _system_prompt: &str, user_prompt: &str) -> AiResult<LlmResponse> {
        use std::sync::atomic::Ordering;

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

        tokio::time::sleep(self.delay).await;
        self.active_calls.fetch_sub(1, Ordering::SeqCst);

        // Place back exactly the files this request asked about.
        let placements = ["f1", "f2", "f3", "f4"]
            .into_iter()
            .filter(|file_id| user_prompt.contains(&format!("\"file_id\":\"{file_id}\"")))
            .map(|file_id| serde_json::json!({"file_id": file_id, "target_rel_path": "."}))
            .collect::<Vec<_>>();
        Ok(llm_response(
            &serde_json::json!({ "placements": placements }).to_string(),
        ))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn placement_batches_run_concurrently() {
    use std::sync::atomic::Ordering;

    let client = Arc::new(ConcurrentPlacementProbeClient {
        active_calls: std::sync::atomic::AtomicUsize::new(0),
        max_active_calls: std::sync::atomic::AtomicUsize::new(0),
        delay: std::time::Duration::from_millis(150),
    });
    let file_ids = ["f1", "f2", "f3", "f4"];
    let papers = file_ids
        .iter()
        .map(|file_id| PaperText {
            file_id: (*file_id).to_string(),
            path: PathBuf::from(format!("/tmp/{file_id}.pdf")),
            extracted_text: "x".to_string(),
            llm_ready_text: "x".to_string(),
            pages_read: 1,
            from_page_images: false,
        })
        .collect::<Vec<_>>();
    let keyword_sets = file_ids
        .iter()
        .map(|file_id| KeywordSet {
            file_id: (*file_id).to_string(),
            keywords: vec!["a".to_string()],
        })
        .collect::<Vec<_>>();
    let preliminary_pairs = file_ids
        .iter()
        .map(|file_id| PreliminaryCategoryPair {
            file_id: (*file_id).to_string(),
            preliminary_categories_k_depth: "Root/A".to_string(),
        })
        .collect::<Vec<_>>();
    let categories = vec![CategoryTree {
        name: "Root".to_string(),
        children: vec![],
    }];
    let snapshot = OutputSnapshot {
        is_empty: true,
        existing_folders: vec![".".to_string()],
        tree_map: "<empty>".to_string(),
    };

    let started_at = std::time::Instant::now();
    let (placements, usage) = generate_placements(
        client.clone(),
        &papers,
        &keyword_sets,
        &preliminary_pairs,
        &categories,
        &snapshot,
        PlacementOptions {
            batch_size: 1,
            // No dispatch spacing, so nothing serializes these but the driver.
            batch_start_delay_ms: 0,
            assistance: PlacementAssistance::LlmOnly,
            placement_mode: PlacementMode::AllowNew,
            category_depth: 2,
            embedding: None,
            verbosity: Verbosity::new(false, false, true),
        },
    )
    .await
    .expect("concurrent placement generation should succeed");
    let elapsed = started_at.elapsed();

    assert_eq!(usage.call_count, 4);
    assert_eq!(placements.len(), 4);
    assert!(
        client.max_active_calls.load(Ordering::SeqCst) > 1,
        "placement batches should overlap rather than run one at a time"
    );
    assert!(
        elapsed < std::time::Duration::from_millis(450),
        "four 150ms batches capped at 4 in flight should finish well under the \
         600ms a sequential run would take, took {elapsed:?}"
    );
    // Concurrent completion must still yield placements in stable batch order.
    let ordered = placements
        .iter()
        .map(|placement| placement.file_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ordered, vec!["f1", "f2", "f3", "f4"]);
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

/// The watcher's situation: a library that already has folders, and a new
/// paper on a subject none of them cover.
mod growing_library {
    use super::*;

    fn new_subject() -> Vec<CategoryTree> {
        vec![CategoryTree {
            name: "Marine Biology".to_string(),
            children: vec![CategoryTree {
                name: "Coral Reefs".to_string(),
                children: vec![],
            }],
        }]
    }

    fn established_library() -> OutputSnapshot {
        OutputSnapshot {
            is_empty: false,
            existing_folders: vec![
                ".".to_string(),
                "Computer Science".to_string(),
                "Computer Science/Vision".to_string(),
            ],
            tree_map: String::new(),
        }
    }

    #[test]
    fn the_first_run_into_an_empty_library_creates_the_taxonomy() {
        let allowed = build_allowed_targets(
            &new_subject(),
            &OutputSnapshot {
                is_empty: true,
                existing_folders: vec![".".to_string()],
                tree_map: String::new(),
            },
            PlacementMode::ExistingOnly,
            2,
        );

        assert!(allowed.iter().any(|target| target == "Marine Biology"));
    }

    #[test]
    fn later_runs_discard_the_new_taxonomy_under_the_default_mode() {
        let allowed = build_allowed_targets(
            &new_subject(),
            &established_library(),
            PlacementMode::ExistingOnly,
            2,
        );

        assert!(
            !allowed.iter().any(|target| target.contains("Marine")),
            "existing-only keeps a new subject out of the library: {allowed:?}"
        );
        assert_eq!(
            allowed,
            vec![
                ".".to_string(),
                "Computer Science".to_string(),
                "Computer Science/Vision".to_string()
            ],
            "the paper can only go to a folder that already exists"
        );
    }

    #[test]
    fn allow_new_lets_a_genuinely_new_subject_get_its_own_folder() {
        let allowed = build_allowed_targets(
            &new_subject(),
            &established_library(),
            PlacementMode::AllowNew,
            2,
        );

        assert!(allowed.iter().any(|target| target == "Marine Biology"));
        assert!(
            allowed
                .iter()
                .any(|target| target == "Marine Biology/Coral Reefs")
        );
        assert!(
            allowed
                .iter()
                .any(|target| target == "Computer Science/Vision"),
            "existing folders stay available"
        );
    }
}
