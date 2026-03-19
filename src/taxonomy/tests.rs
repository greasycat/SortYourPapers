use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use serde_json::Value;
use tokio::time::sleep;

use super::{
    KeywordBatchProgress, KeywordBatchResult, KeywordPair, TaxonomyBatchProgress,
    TaxonomyBatchResult, extract_keywords, extract_keywords_with_progress,
    prompts::{
        build_batch_keyword_prompt, build_category_prompt, build_merge_category_plain_text_prompt,
        build_merge_category_prompt, format_llm_request_debug_message,
    },
    synthesize_categories, synthesize_categories_with_progress,
    taxonomy::{
        merge_category_batches, merge_category_batches_with_timeout,
        parse_plain_text_category_paths,
    },
    validation::{
        aggregate_preliminary_categories, validate_category_depth, validate_keyword_batch_response,
    },
};
use crate::error::Result;
use crate::llm::{JsonResponseSchema, LlmCallMetrics, LlmClient, LlmResponse, LlmUsageSummary};
use crate::papers::{KeywordSet, PaperText, PreliminaryCategoryPair};
use crate::taxonomy::CategoryTree;
use crate::terminal::Verbosity;

struct ConcurrentKeywordProbeClient {
    calls: AtomicUsize,
    active_calls: AtomicUsize,
    max_active_calls: AtomicUsize,
    started_at: Mutex<Vec<Instant>>,
    delay: Duration,
}

#[derive(Debug, Clone)]
struct CapturedSchemaCall {
    name: String,
    schema: Value,
    user_prompt: String,
}

#[derive(Default)]
struct JsonOnlySchemaProbeClient {
    captured_calls: Mutex<Vec<CapturedSchemaCall>>,
}

struct MergeTimeoutFallbackClient {
    chat_json_delay: Duration,
    chat_json_calls: AtomicUsize,
    chat_calls: AtomicUsize,
    chat_json_prompts: Mutex<Vec<String>>,
    chat_prompts: Mutex<Vec<String>>,
    chat_responses: Mutex<Vec<String>>,
}

struct PlainTextOnlyTaxonomyMergeClient {
    chat_json_calls: AtomicUsize,
    chat_calls: AtomicUsize,
    chat_prompts: Mutex<Vec<String>>,
}

impl MergeTimeoutFallbackClient {
    fn new(chat_json_delay: Duration, chat_responses: Vec<String>) -> Self {
        Self {
            chat_json_delay,
            chat_json_calls: AtomicUsize::new(0),
            chat_calls: AtomicUsize::new(0),
            chat_json_prompts: Mutex::new(Vec::new()),
            chat_prompts: Mutex::new(Vec::new()),
            chat_responses: Mutex::new(chat_responses.into_iter().rev().collect()),
        }
    }
}

impl PlainTextOnlyTaxonomyMergeClient {
    fn new() -> Self {
        Self {
            chat_json_calls: AtomicUsize::new(0),
            chat_calls: AtomicUsize::new(0),
            chat_prompts: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl LlmClient for ConcurrentKeywordProbeClient {
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

        let response = if user_prompt.contains("\"file_id\":\"a\"") {
            Ok(llm_response(
                &serde_json::json!({
                    "pairs": [
                        {
                            "file_id": "a",
                            "keywords": ["alpha", "beta"],
                            "preliminary_categories_k_depth": "AI/Sequence Models"
                        }
                    ]
                })
                .to_string(),
            ))
        } else if user_prompt.contains("\"file_id\":\"b\"") {
            Ok(llm_response(
                &serde_json::json!({
                    "pairs": [
                        {
                            "file_id": "b",
                            "keywords": ["gamma", "delta"],
                            "preliminary_categories_k_depth": "AI/Generative Models"
                        }
                    ]
                })
                .to_string(),
            ))
        } else if user_prompt.contains("\"file_id\":\"c\"") {
            Ok(llm_response(
                &serde_json::json!({
                    "pairs": [
                        {
                            "file_id": "c",
                            "keywords": ["epsilon", "zeta"],
                            "preliminary_categories_k_depth": "AI/Vision"
                        }
                    ]
                })
                .to_string(),
            ))
        } else {
            Err(crate::error::AppError::Execution(
                "probe client could not determine requested file_id".to_string(),
            ))
        };
        self.active_calls.fetch_sub(1, Ordering::SeqCst);
        response
    }
}

#[async_trait]
impl LlmClient for JsonOnlySchemaProbeClient {
    async fn chat(&self, _system_prompt: &str, _user_prompt: &str) -> Result<LlmResponse> {
        Err(crate::error::AppError::Execution(
            "schema probe client requires chat_json()".to_string(),
        ))
    }

    async fn chat_json(
        &self,
        _system_prompt: &str,
        user_prompt: &str,
        schema: &JsonResponseSchema,
    ) -> Result<LlmResponse> {
        self.captured_calls
            .lock()
            .expect("captured_calls lock")
            .push(CapturedSchemaCall {
                name: schema.name().to_string(),
                schema: schema.schema().clone(),
                user_prompt: user_prompt.to_string(),
            });

        match schema.name() {
            "keyword_batch_response" => {
                let file_id = if user_prompt.contains("\"file_id\":\"b\"") {
                    "b"
                } else {
                    "a"
                };
                Ok(llm_response(
                    &serde_json::json!({
                        "pairs": [
                            {
                                "file_id": file_id,
                                "keywords": ["alpha", "beta"],
                                "preliminary_categories_k_depth": "AI/Transformers"
                            }
                        ]
                    })
                    .to_string(),
                ))
            }
            "category_response" => Ok(llm_response(
                &serde_json::json!({
                    "categories": [
                        ["AI"],
                        ["AI", "Transformers"]
                    ]
                })
                .to_string(),
            )),
            other => Err(crate::error::AppError::Execution(format!(
                "unexpected schema {other}"
            ))),
        }
    }
}

#[async_trait]
impl LlmClient for MergeTimeoutFallbackClient {
    async fn chat(&self, _system_prompt: &str, user_prompt: &str) -> Result<LlmResponse> {
        self.chat_calls.fetch_add(1, Ordering::SeqCst);
        self.chat_prompts
            .lock()
            .expect("chat_prompts lock")
            .push(user_prompt.to_string());
        let content = self
            .chat_responses
            .lock()
            .expect("chat_responses lock")
            .pop()
            .expect("chat response");
        Ok(llm_response(&content))
    }

    async fn chat_json(
        &self,
        _system_prompt: &str,
        user_prompt: &str,
        _schema: &JsonResponseSchema,
    ) -> Result<LlmResponse> {
        self.chat_json_calls.fetch_add(1, Ordering::SeqCst);
        self.chat_json_prompts
            .lock()
            .expect("chat_json_prompts lock")
            .push(user_prompt.to_string());
        sleep(self.chat_json_delay).await;
        Ok(llm_response(
            &serde_json::json!({
                "categories": [
                    ["Late"],
                    ["Late", "Response"]
                ]
            })
            .to_string(),
        ))
    }
}

#[async_trait]
impl LlmClient for PlainTextOnlyTaxonomyMergeClient {
    async fn chat(&self, _system_prompt: &str, user_prompt: &str) -> Result<LlmResponse> {
        self.chat_calls.fetch_add(1, Ordering::SeqCst);
        self.chat_prompts
            .lock()
            .expect("chat_prompts lock")
            .push(user_prompt.to_string());
        Ok(llm_response(
            "AI\nAI > Transformers\nSystems\nSystems > Databases",
        ))
    }

    fn prefers_plain_text_taxonomy_merge(&self) -> bool {
        true
    }

    async fn chat_json(
        &self,
        _system_prompt: &str,
        _user_prompt: &str,
        _schema: &JsonResponseSchema,
    ) -> Result<LlmResponse> {
        self.chat_json_calls.fetch_add(1, Ordering::SeqCst);
        Err(crate::error::AppError::Execution(
            "plain-text taxonomy merge client should not receive chat_json()".to_string(),
        ))
    }
}

#[test]
fn rejects_depth_above_limit() {
    let tree = vec![CategoryTree {
        name: "A".to_string(),
        children: vec![CategoryTree {
            name: "B".to_string(),
            children: vec![CategoryTree {
                name: "C".to_string(),
                children: vec![],
            }],
        }],
    }];

    let result = validate_category_depth(&tree, 2);
    assert!(result.is_err());
}

#[test]
fn validates_keyword_pairs_for_batch_and_keeps_preliminary_text() {
    let batch = vec![make_paper("a"), make_paper("b")];

    let response = vec![
        KeywordPair {
            file_id: "a".to_string(),
            keywords: vec!["A".to_string(), "A".to_string(), "B".to_string()],
            preliminary_categories_k_depth: "AI/Transformers".to_string(),
        },
        KeywordPair {
            file_id: "b".to_string(),
            keywords: vec!["C".to_string()],
            preliminary_categories_k_depth: "AI/Vision".to_string(),
        },
    ];

    let (keyword_sets, preliminary_pairs) =
        validate_keyword_batch_response(&response, &batch).expect("valid pairs");
    assert_eq!(keyword_sets.len(), 2);
    assert_eq!(keyword_sets[0].file_id, "a");
    assert_eq!(
        keyword_sets[0].keywords,
        vec!["A".to_string(), "B".to_string()]
    );
    assert_eq!(
        preliminary_pairs[0].preliminary_categories_k_depth,
        "AI/Transformers"
    );
}

#[test]
fn prompt_uses_llm_ready_terms_instead_of_raw_text() {
    let batch = vec![PaperText {
        file_id: "a".to_string(),
        path: PathBuf::from("a.pdf"),
        extracted_text: "raw prose sentence".to_string(),
        llm_ready_text: "graph neural network, node classification".to_string(),
        pages_read: 1,
    }];

    let prompt = build_batch_keyword_prompt(&batch).expect("prompt");
    assert!(prompt.contains("\"terms\":\"graph neural network, node classification\""));
    assert!(prompt.contains("\"preliminary_categories_k_depth\""));
    assert!(!prompt.contains("raw prose sentence"));
    assert!(!prompt.contains("\"path\""));
    assert!(!prompt.contains("\"pages_read\""));
}

#[test]
fn category_prompt_uses_aggregated_preliminary_categories() {
    let aggregated = vec![
        ("AI/Transformers".to_string(), 2),
        ("AI/Vision".to_string(), 1),
    ];

    let user = build_category_prompt(&aggregated, 2).expect("prompt");

    assert!(user.contains("aggregated_preliminary_categories"));
    assert!(user.contains("AI/Transformers"));
    assert!(user.contains("AI/Vision"));
    assert!(user.contains("category depth must be <= 2"));
    assert!(!user.contains("keywords"));
}

#[test]
fn debug_message_formats_taxonomy_request() {
    let message =
        format_llm_request_debug_message("taxonomy/global", 2, "system prompt", "user prompt");

    assert!(message.contains("taxonomy/global request attempt 2/3"));
    assert!(message.contains("system:\nsystem prompt"));
    assert!(message.contains("user:\nuser prompt"));
}

#[test]
fn aggregates_duplicate_preliminary_categories() {
    let aggregated = aggregate_preliminary_categories(&[
        PreliminaryCategoryPair {
            file_id: "a".to_string(),
            preliminary_categories_k_depth: "AI/Transformers".to_string(),
        },
        PreliminaryCategoryPair {
            file_id: "b".to_string(),
            preliminary_categories_k_depth: "AI/Transformers".to_string(),
        },
        PreliminaryCategoryPair {
            file_id: "c".to_string(),
            preliminary_categories_k_depth: "AI/Vision".to_string(),
        },
    ]);

    assert_eq!(
        aggregated,
        vec![
            ("AI/Transformers".to_string(), 2),
            ("AI/Vision".to_string(), 1)
        ]
    );
}

#[tokio::test]
async fn keyword_extraction_uses_chat_json_with_strict_keyword_schema() {
    let raw_client = Arc::new(JsonOnlySchemaProbeClient::default());
    let client: Arc<dyn LlmClient> = raw_client.clone();
    let papers = vec![make_paper("a")];

    let (keyword_state, usage) =
        extract_keywords(client, &papers, 1, 0, Verbosity::new(false, false, false))
            .await
            .expect("keyword extraction should use chat_json");

    assert_eq!(usage.call_count, 1);
    assert_eq!(keyword_state.keyword_sets.len(), 1);
    assert_eq!(keyword_state.preliminary_pairs.len(), 1);
    assert_eq!(keyword_state.keyword_sets[0].file_id, "a");

    let captured = raw_client
        .captured_calls
        .lock()
        .expect("captured_calls lock");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].name, "keyword_batch_response");
    assert_eq!(captured[0].schema["type"], "object");
    assert_eq!(captured[0].schema["required"], serde_json::json!(["pairs"]));
    assert_eq!(
        captured[0].schema["properties"]["pairs"]["items"]["required"],
        serde_json::json!(["file_id", "keywords", "preliminary_categories_k_depth"])
    );
}

#[tokio::test]
async fn taxonomy_synthesis_uses_aggregated_preliminary_categories() {
    let raw_client = Arc::new(JsonOnlySchemaProbeClient::default());
    let client: Arc<dyn LlmClient> = raw_client.clone();
    let preliminary_pairs = vec![
        PreliminaryCategoryPair {
            file_id: "a".to_string(),
            preliminary_categories_k_depth: "AI/Transformers".to_string(),
        },
        PreliminaryCategoryPair {
            file_id: "b".to_string(),
            preliminary_categories_k_depth: "AI/Transformers".to_string(),
        },
    ];

    let (categories, usage) = synthesize_categories(
        client.as_ref(),
        &preliminary_pairs,
        2,
        10,
        0,
        5,
        Verbosity::new(false, false, false),
    )
    .await
    .expect("taxonomy synthesis should use chat_json");

    assert_eq!(usage.call_count, 1);
    assert_eq!(categories.len(), 1);
    assert_eq!(categories[0].name, "AI");

    let captured = raw_client
        .captured_calls
        .lock()
        .expect("captured_calls lock");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].name, "category_response");
    assert!(
        captured[0]
            .user_prompt
            .contains("aggregated_preliminary_categories")
    );
    assert!(captured[0].user_prompt.contains("AI/Transformers"));
    assert!(!captured[0].user_prompt.contains("\"keywords\""));
}

#[test]
fn merge_prompt_flattens_and_sorts_category_paths() {
    let prompt = build_merge_category_prompt(
        &[
            vec![CategoryTree {
                name: "Systems".to_string(),
                children: vec![CategoryTree {
                    name: "Databases".to_string(),
                    children: vec![],
                }],
            }],
            vec![CategoryTree {
                name: "AI".to_string(),
                children: vec![
                    CategoryTree {
                        name: "Vision".to_string(),
                        children: vec![],
                    },
                    CategoryTree {
                        name: "Transformers".to_string(),
                        children: vec![],
                    },
                ],
            }],
        ],
        2,
        5,
        Some("Merge speech categories under one parent"),
    )
    .expect("prompt");

    assert!(prompt.contains("category_paths"));
    assert!(!prompt.contains("batch_categories"));
    assert!(!prompt.contains("\"batch_index\""));
    assert!(prompt.contains(
        "[[\"AI\"],[\"AI\",\"Transformers\"],[\"AI\",\"Vision\"],[\"Systems\"],[\"Systems\",\"Databases\"]]"
    ));
    assert!(!prompt.contains("\"children\""));
    assert!(prompt.contains("user_merge_suggestion"));
    assert!(prompt.contains("Merge speech categories under one parent"));
    assert!(prompt.contains("try your best to keep the number of subcategories less than 5"));
}

#[test]
fn merge_plain_text_prompt_uses_line_format() {
    let prompt = build_merge_category_plain_text_prompt(
        &[vec![CategoryTree {
            name: "AI".to_string(),
            children: vec![CategoryTree {
                name: "Transformers".to_string(),
                children: vec![],
            }],
        }]],
        2,
        5,
        None,
    )
    .expect("plain-text prompt");

    assert!(prompt.contains("Return plain text only."));
    assert!(prompt.contains("return one full category path per line"));
    assert!(prompt.contains("use ` > ` between path segments"));
    assert!(prompt.contains("try your best to keep the number of subcategories less than 5"));
    assert!(prompt.contains("- no JSON"));
    assert!(!prompt.contains("Return JSON with schema"));
}

#[test]
fn parse_plain_text_category_paths_trims_and_skips_blank_lines() {
    let paths = parse_plain_text_category_paths(
        "```text\n AI \n\nAI > Transformers\n Systems > Databases \n```",
    )
    .expect("paths");

    assert_eq!(
        paths,
        vec![
            vec!["AI".to_string()],
            vec!["AI".to_string(), "Transformers".to_string()],
            vec!["Systems".to_string(), "Databases".to_string()],
        ]
    );
}

#[tokio::test]
async fn taxonomy_merge_keeps_structured_schema_before_timeout() {
    let raw_client = Arc::new(JsonOnlySchemaProbeClient::default());
    let client: Arc<dyn LlmClient> = raw_client.clone();
    let partial_categories = vec![
        vec![CategoryTree {
            name: "Systems".to_string(),
            children: vec![],
        }],
        vec![CategoryTree {
            name: "AI".to_string(),
            children: vec![CategoryTree {
                name: "Transformers".to_string(),
                children: vec![],
            }],
        }],
    ];

    let (categories, usage) = merge_category_batches_with_timeout(
        client.as_ref(),
        &partial_categories,
        2,
        5,
        None,
        Verbosity::new(false, false, false),
        Duration::from_secs(1),
    )
    .await
    .expect("structured merge should succeed");

    assert_eq!(usage.call_count, 1);
    assert_eq!(categories[0].name, "AI");

    let captured = raw_client
        .captured_calls
        .lock()
        .expect("captured_calls lock");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].name, "category_response");
    assert!(captured[0].user_prompt.contains("category_paths"));
}

#[tokio::test]
async fn taxonomy_merge_times_out_to_plain_text_paths() {
    let client = MergeTimeoutFallbackClient::new(
        Duration::from_millis(50),
        vec!["AI\nAI > Transformers\nSystems\nSystems > Databases".to_string()],
    );
    let partial_categories = vec![
        vec![CategoryTree {
            name: "Systems".to_string(),
            children: vec![CategoryTree {
                name: "Databases".to_string(),
                children: vec![],
            }],
        }],
        vec![CategoryTree {
            name: "AI".to_string(),
            children: vec![CategoryTree {
                name: "Transformers".to_string(),
                children: vec![],
            }],
        }],
    ];

    let (categories, usage) = merge_category_batches_with_timeout(
        &client,
        &partial_categories,
        2,
        5,
        None,
        Verbosity::new(false, false, false),
        Duration::from_millis(5),
    )
    .await
    .expect("plain-text fallback should succeed");

    assert_eq!(usage.call_count, 1);
    assert_eq!(client.chat_json_calls.load(Ordering::SeqCst), 1);
    assert_eq!(client.chat_calls.load(Ordering::SeqCst), 1);
    assert_eq!(categories[0].name, "AI");
    assert_eq!(categories[1].name, "Systems");

    let prompts = client.chat_prompts.lock().expect("chat_prompts lock");
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].contains("return one full category path per line"));
    assert!(prompts[0].contains("use ` > ` between path segments"));
}

#[tokio::test]
async fn taxonomy_merge_plain_text_retry_repairs_invalid_response() {
    let client = MergeTimeoutFallbackClient::new(
        Duration::from_millis(50),
        vec!["AI >".to_string(), "AI\nAI > Transformers".to_string()],
    );
    let partial_categories = vec![
        vec![CategoryTree {
            name: "Systems".to_string(),
            children: vec![],
        }],
        vec![CategoryTree {
            name: "AI".to_string(),
            children: vec![CategoryTree {
                name: "Transformers".to_string(),
                children: vec![],
            }],
        }],
    ];

    let (categories, usage) = merge_category_batches_with_timeout(
        &client,
        &partial_categories,
        2,
        5,
        None,
        Verbosity::new(false, false, false),
        Duration::from_millis(5),
    )
    .await
    .expect("fallback retry should succeed");

    assert_eq!(usage.call_count, 2);
    assert_eq!(client.chat_calls.load(Ordering::SeqCst), 2);
    assert_eq!(categories[0].name, "AI");

    let prompts = client.chat_prompts.lock().expect("chat_prompts lock");
    assert_eq!(prompts.len(), 2);
    assert!(prompts[1].contains("Return corrected plain text"));
    assert!(prompts[1].contains("do not return JSON or markdown"));
}

#[tokio::test]
async fn taxonomy_merge_uses_plain_text_directly_for_plain_text_clients() {
    let client = PlainTextOnlyTaxonomyMergeClient::new();
    let partial_categories = vec![
        vec![CategoryTree {
            name: "Systems".to_string(),
            children: vec![CategoryTree {
                name: "Databases".to_string(),
                children: vec![],
            }],
        }],
        vec![CategoryTree {
            name: "AI".to_string(),
            children: vec![CategoryTree {
                name: "Transformers".to_string(),
                children: vec![],
            }],
        }],
    ];

    let (categories, usage) = merge_category_batches(
        &client,
        &partial_categories,
        2,
        5,
        None,
        Verbosity::new(false, false, false),
    )
    .await
    .expect("plain-text-only merge should succeed");

    assert_eq!(usage.call_count, 1);
    assert_eq!(client.chat_json_calls.load(Ordering::SeqCst), 0);
    assert_eq!(client.chat_calls.load(Ordering::SeqCst), 1);
    assert_eq!(categories[0].name, "AI");

    let prompts = client.chat_prompts.lock().expect("chat_prompts lock");
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].contains("return one full category path per line"));
    assert!(prompts[0].contains("use ` > ` between path segments"));
}

#[tokio::test]
async fn taxonomy_synthesis_batches_preliminary_categories_before_merge() {
    let raw_client = Arc::new(JsonOnlySchemaProbeClient::default());
    let client: Arc<dyn LlmClient> = raw_client.clone();
    let preliminary_pairs = vec![
        PreliminaryCategoryPair {
            file_id: "a".to_string(),
            preliminary_categories_k_depth: "AI/Transformers".to_string(),
        },
        PreliminaryCategoryPair {
            file_id: "b".to_string(),
            preliminary_categories_k_depth: "AI/Vision".to_string(),
        },
        PreliminaryCategoryPair {
            file_id: "c".to_string(),
            preliminary_categories_k_depth: "Systems/Databases".to_string(),
        },
    ];

    let (categories, usage) = synthesize_categories(
        client.as_ref(),
        &preliminary_pairs,
        2,
        2,
        0,
        5,
        Verbosity::new(false, false, false),
    )
    .await
    .expect("batched taxonomy synthesis should succeed");

    assert_eq!(usage.call_count, 3);
    assert_eq!(categories[0].name, "AI");

    let captured = raw_client
        .captured_calls
        .lock()
        .expect("captured_calls lock");
    assert_eq!(captured.len(), 3);
    assert!(
        captured[0]
            .user_prompt
            .contains("aggregated_preliminary_categories")
    );
    assert!(
        captured[1]
            .user_prompt
            .contains("aggregated_preliminary_categories")
    );
    assert!(captured[2].user_prompt.contains("category_paths"));
}

#[tokio::test]
async fn taxonomy_resume_skips_saved_batches() {
    let raw_client = Arc::new(JsonOnlySchemaProbeClient::default());
    let client: Arc<dyn LlmClient> = raw_client.clone();
    let preliminary_pairs = vec![
        PreliminaryCategoryPair {
            file_id: "a".to_string(),
            preliminary_categories_k_depth: "AI/Transformers".to_string(),
        },
        PreliminaryCategoryPair {
            file_id: "b".to_string(),
            preliminary_categories_k_depth: "AI/Vision".to_string(),
        },
        PreliminaryCategoryPair {
            file_id: "c".to_string(),
            preliminary_categories_k_depth: "Systems/Databases".to_string(),
        },
    ];
    let saved_progress = TaxonomyBatchProgress {
        completed_batches: vec![TaxonomyBatchResult {
            batch_index: 1,
            input_count: 2,
            input_fingerprint: Some(
                serde_json::to_string(&vec![
                    ("AI/Transformers".to_string(), 1usize),
                    ("AI/Vision".to_string(), 1usize),
                ])
                .expect("taxonomy fingerprint"),
            ),
            categories: vec![CategoryTree {
                name: "Saved".to_string(),
                children: vec![],
            }],
            elapsed_ms: 10,
        }],
        usage: LlmUsageSummary {
            call_count: 1,
            ..LlmUsageSummary::default()
        },
    };

    let (categories, usage) = synthesize_categories_with_progress(
        client.as_ref(),
        &preliminary_pairs,
        2,
        2,
        0,
        5,
        saved_progress,
        |_| Ok(()),
        Verbosity::new(false, false, false),
    )
    .await
    .expect("taxonomy resume should succeed");

    assert_eq!(usage.call_count, 3);
    assert_eq!(categories[0].name, "AI");

    let captured = raw_client
        .captured_calls
        .lock()
        .expect("captured_calls lock");
    assert_eq!(captured.len(), 2);
    assert!(captured[0].user_prompt.contains("Systems/Databases"));
    assert!(captured[1].user_prompt.contains("category_paths"));
    assert!(!captured[0].user_prompt.contains("AI/Vision"));
}

#[tokio::test]
async fn taxonomy_resume_rejects_saved_batch_with_stale_inputs() {
    let raw_client = Arc::new(JsonOnlySchemaProbeClient::default());
    let client: Arc<dyn LlmClient> = raw_client;
    let preliminary_pairs = vec![
        PreliminaryCategoryPair {
            file_id: "a".to_string(),
            preliminary_categories_k_depth: "AI/Transformers".to_string(),
        },
        PreliminaryCategoryPair {
            file_id: "b".to_string(),
            preliminary_categories_k_depth: "AI/Agents".to_string(),
        },
    ];
    let saved_progress = TaxonomyBatchProgress {
        completed_batches: vec![TaxonomyBatchResult {
            batch_index: 1,
            input_count: 2,
            input_fingerprint: Some(
                serde_json::to_string(&vec![
                    ("AI/Transformers".to_string(), 1usize),
                    ("AI/Vision".to_string(), 1usize),
                ])
                .expect("taxonomy fingerprint"),
            ),
            categories: vec![CategoryTree {
                name: "Saved".to_string(),
                children: vec![],
            }],
            elapsed_ms: 10,
        }],
        usage: LlmUsageSummary::default(),
    };

    let err = synthesize_categories_with_progress(
        client.as_ref(),
        &preliminary_pairs,
        2,
        2,
        0,
        5,
        saved_progress,
        |_| Ok(()),
        Verbosity::new(false, false, false),
    )
    .await
    .expect_err("stale taxonomy resume should be rejected");

    assert!(
        err.to_string()
            .contains("no longer matches the current input")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn keyword_batches_run_concurrently() {
    let client = Arc::new(ConcurrentKeywordProbeClient {
        calls: AtomicUsize::new(0),
        active_calls: AtomicUsize::new(0),
        max_active_calls: AtomicUsize::new(0),
        started_at: Mutex::new(Vec::new()),
        delay: Duration::from_millis(150),
    });
    let papers = vec![make_paper("a"), make_paper("b"), make_paper("c")];

    let (keyword_state, usage) = extract_keywords(
        client.clone(),
        &papers,
        1,
        100,
        Verbosity::new(false, false, true),
    )
    .await
    .expect("concurrent keyword extraction should succeed");

    assert_eq!(usage.call_count, 3);
    assert_eq!(keyword_state.keyword_sets.len(), 3);
    assert_eq!(keyword_state.preliminary_pairs.len(), 3);
    assert_eq!(keyword_state.keyword_sets[0].file_id, "a");
    assert_eq!(keyword_state.keyword_sets[2].file_id, "c");
    assert_eq!(
        keyword_state.preliminary_pairs[1].preliminary_categories_k_depth,
        "AI/Generative Models"
    );
    assert_eq!(client.calls.load(Ordering::SeqCst), 3);
    assert!(client.max_active_calls.load(Ordering::SeqCst) > 1);
    let started_at = client.started_at.lock().expect("started_at");
    assert_eq!(started_at.len(), 3);
    assert!(started_at[1].duration_since(started_at[0]) >= Duration::from_millis(80));
}

#[tokio::test]
async fn keyword_resume_skips_saved_batches() {
    let raw_client = Arc::new(JsonOnlySchemaProbeClient::default());
    let client: Arc<dyn LlmClient> = raw_client.clone();
    let papers = vec![make_paper("a"), make_paper("b")];
    let saved_progress = KeywordBatchProgress {
        completed_batches: vec![KeywordBatchResult {
            batch_index: 1,
            keyword_sets: vec![KeywordSet {
                file_id: "a".to_string(),
                keywords: vec!["saved".to_string()],
            }],
            preliminary_pairs: vec![PreliminaryCategoryPair {
                file_id: "a".to_string(),
                preliminary_categories_k_depth: "Saved/Category".to_string(),
            }],
        }],
        usage: LlmUsageSummary {
            call_count: 1,
            ..LlmUsageSummary::default()
        },
    };

    let (keyword_state, usage) = extract_keywords_with_progress(
        client,
        &papers,
        1,
        0,
        saved_progress,
        |_| Ok(()),
        Verbosity::new(false, false, false),
    )
    .await
    .expect("keyword resume should succeed");

    assert_eq!(usage.call_count, 2);
    assert_eq!(keyword_state.keyword_sets.len(), 2);
    assert_eq!(
        keyword_state.keyword_sets[0].keywords,
        vec!["saved".to_string()]
    );
    assert_eq!(
        keyword_state.preliminary_pairs[0].preliminary_categories_k_depth,
        "Saved/Category"
    );

    let captured = raw_client
        .captured_calls
        .lock()
        .expect("captured_calls lock");
    assert_eq!(captured.len(), 1);
    assert!(captured[0].user_prompt.contains("\"file_id\":\"b\""));
}

#[tokio::test]
async fn keyword_resume_rejects_saved_batch_with_stale_file_ids() {
    let raw_client = Arc::new(JsonOnlySchemaProbeClient::default());
    let client: Arc<dyn LlmClient> = raw_client;
    let papers = vec![make_paper("a"), make_paper("b")];
    let saved_progress = KeywordBatchProgress {
        completed_batches: vec![KeywordBatchResult {
            batch_index: 1,
            keyword_sets: vec![KeywordSet {
                file_id: "b".to_string(),
                keywords: vec!["saved".to_string()],
            }],
            preliminary_pairs: vec![PreliminaryCategoryPair {
                file_id: "b".to_string(),
                preliminary_categories_k_depth: "Saved/Category".to_string(),
            }],
        }],
        usage: LlmUsageSummary::default(),
    };

    let err = extract_keywords_with_progress(
        client,
        &papers,
        1,
        0,
        saved_progress,
        |_| Ok(()),
        Verbosity::new(false, false, false),
    )
    .await
    .expect_err("stale keyword resume should be rejected");

    assert!(err.to_string().contains("inconsistent file ids"));
}

fn make_paper(id: &str) -> PaperText {
    PaperText {
        file_id: id.to_string(),
        path: PathBuf::from(format!("{id}.pdf")),
        extracted_text: "text".to_string(),
        llm_ready_text: "term list".to_string(),
        pages_read: 1,
    }
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
