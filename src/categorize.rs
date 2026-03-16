use std::{
    collections::{BTreeSet, HashMap, HashSet},
    io::{IsTerminal, stderr},
    sync::Arc,
};

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use serde::{Deserialize, Serialize};
use tokio::task::JoinSet;

use crate::{
    error::{AppError, Result},
    llm::{LlmClient, call_json_with_retry},
    models::{CategoryTree, KeywordSet, PaperText},
};

const MAX_JSON_ATTEMPTS: usize = 3;
const MAX_SEMANTIC_ATTEMPTS: usize = 3;
const MAX_TEXT_CHARS_PER_FILE: usize = 4_000;
const MAX_TOTAL_BATCH_TEXT_CHARS: usize = 60_000;
const MAX_CONCURRENT_TAXONOMY_BATCH_REQUESTS: usize = 4;

#[derive(Debug, Deserialize)]
struct KeywordPair {
    file_id: String,
    keywords: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct KeywordBatchResponse {
    pairs: Vec<KeywordPair>,
}

#[derive(Debug, Deserialize)]
struct CategoryResponse {
    categories: Vec<CategoryTree>,
}

#[derive(Debug, Clone, Serialize)]
struct CategoryBatchSummary {
    batch_index: usize,
    file_ids: Vec<String>,
    keywords: Vec<String>,
    categories: Vec<CategoryTree>,
}

#[derive(Debug, Clone)]
struct PreparedCategoryBatch {
    batch_index: usize,
    file_ids: Vec<String>,
    keyword_sets: Vec<KeywordSet>,
}

pub async fn extract_keywords(
    client: &dyn LlmClient,
    papers: &[PaperText],
    keyword_batch_size: usize,
) -> Result<Vec<KeywordSet>> {
    if papers.is_empty() {
        return Ok(Vec::new());
    }
    if keyword_batch_size == 0 {
        return Err(AppError::Validation(
            "keyword_batch_size must be greater than 0".to_string(),
        ));
    }

    let mut sets = Vec::with_capacity(papers.len());
    for batch in papers.chunks(keyword_batch_size) {
        let system = "You extract concise research keywords from academic paper excerpts. Return strict JSON only.";
        let base_user = build_batch_keyword_prompt(batch)?;
        let mut user = base_user.clone();
        let mut accepted: Option<Vec<KeywordSet>> = None;
        let mut last_issue = String::new();

        for attempt in 1..=MAX_SEMANTIC_ATTEMPTS {
            let response: KeywordBatchResponse =
                call_json_with_retry(client, system, &user, MAX_JSON_ATTEMPTS).await?;

            match validate_keyword_batch_response(&response.pairs, batch) {
                Ok(batch_sets) => {
                    accepted = Some(batch_sets);
                    break;
                }
                Err(err) => {
                    last_issue = err.to_string();
                }
            }

            if attempt < MAX_SEMANTIC_ATTEMPTS {
                user = format!(
                    "{base_user}\n\nYour previous response had this issue: {last_issue}.\nReturn JSON again.\nImportant: return exactly one pair for every file_id."
                );
            }
        }

        let Some(batch_sets) = accepted else {
            let batch_span = format_batch_span(batch);
            return Err(AppError::Validation(format!(
                "failed keyword extraction validation for batch {}: {}",
                batch_span, last_issue
            )));
        };
        sets.extend(batch_sets);
    }

    Ok(sets)
}

pub async fn synthesize_categories(
    client: &dyn LlmClient,
    keyword_sets: &[KeywordSet],
    category_depth: u8,
) -> Result<Vec<CategoryTree>> {
    if keyword_sets.is_empty() {
        return Ok(Vec::new());
    }

    let unique_keywords = collect_unique_keywords(keyword_sets);
    let base_user = build_category_prompt(&unique_keywords, category_depth)?;
    request_validated_categories(
        client,
        "You design hierarchical folder taxonomies for academic PDFs. Return strict JSON only.",
        &base_user,
        category_depth,
    )
    .await
}

pub async fn synthesize_categories_batch_merged(
    client: Arc<dyn LlmClient>,
    papers: &[PaperText],
    keyword_sets: &[KeywordSet],
    category_depth: u8,
    taxonomy_batch_size: usize,
) -> Result<Vec<CategoryTree>> {
    if papers.is_empty() || keyword_sets.is_empty() {
        return Ok(Vec::new());
    }
    if taxonomy_batch_size == 0 {
        return Err(AppError::Validation(
            "taxonomy_batch_size must be greater than 0".to_string(),
        ));
    }

    let keyword_map = keyword_sets
        .iter()
        .map(|set| (set.file_id.as_str(), set))
        .collect::<HashMap<_, _>>();
    let prepared_batches = papers
        .chunks(taxonomy_batch_size)
        .enumerate()
        .map(|(batch_index, paper_batch)| {
            let keyword_sets = paper_batch
                .iter()
                .map(|paper| {
                    keyword_map
                        .get(paper.file_id.as_str())
                        .copied()
                        .cloned()
                        .ok_or_else(|| {
                            AppError::Validation(format!(
                                "missing keyword set for expected file_id '{}'",
                                paper.file_id
                            ))
                        })
                })
                .collect::<Result<Vec<_>>>()?;

            Ok(PreparedCategoryBatch {
                batch_index: batch_index + 1,
                file_ids: paper_batch
                    .iter()
                    .map(|paper| paper.file_id.clone())
                    .collect(),
                keyword_sets,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let mut batch_summaries =
        run_category_batches_concurrently(Arc::clone(&client), prepared_batches, category_depth)
            .await?;
    batch_summaries.sort_by_key(|summary| summary.batch_index);

    let base_user = build_merge_category_prompt(&batch_summaries, category_depth)?;
    request_validated_categories(
        client.as_ref(),
        "You merge partial folder taxonomies for academic PDFs into one final taxonomy. Return strict JSON only.",
        &base_user,
        category_depth,
    )
    .await
}

async fn run_category_batches_concurrently(
    client: Arc<dyn LlmClient>,
    prepared_batches: Vec<PreparedCategoryBatch>,
    category_depth: u8,
) -> Result<Vec<CategoryBatchSummary>> {
    let progress = new_batch_progress_bar(prepared_batches.len());
    let max_in_flight = MAX_CONCURRENT_TAXONOMY_BATCH_REQUESTS.max(1);
    let mut pending_batches = prepared_batches.into_iter();
    let mut in_flight = JoinSet::new();
    let mut batch_summaries = Vec::new();

    for _ in 0..max_in_flight {
        let Some(batch) = pending_batches.next() else {
            break;
        };
        spawn_category_batch(&mut in_flight, Arc::clone(&client), batch, category_depth);
    }

    while let Some(join_result) = in_flight.join_next().await {
        let summary = match join_result {
            Ok(Ok(summary)) => summary,
            Ok(Err(err)) => {
                progress.abandon();
                return Err(err);
            }
            Err(err) => {
                progress.abandon();
                return Err(AppError::Execution(format!(
                    "taxonomy batch task failed: {err}"
                )));
            }
        };
        batch_summaries.push(summary);
        progress.inc(1);

        if let Some(batch) = pending_batches.next() {
            spawn_category_batch(&mut in_flight, Arc::clone(&client), batch, category_depth);
        }
    }

    progress.finish_and_clear();

    Ok(batch_summaries)
}

fn spawn_category_batch(
    join_set: &mut JoinSet<Result<CategoryBatchSummary>>,
    client: Arc<dyn LlmClient>,
    batch: PreparedCategoryBatch,
    category_depth: u8,
) {
    join_set.spawn(async move {
        let categories =
            synthesize_categories(client.as_ref(), &batch.keyword_sets, category_depth).await?;
        Ok(CategoryBatchSummary {
            batch_index: batch.batch_index,
            file_ids: batch.file_ids,
            keywords: collect_unique_keywords(&batch.keyword_sets),
            categories,
        })
    });
}

fn new_batch_progress_bar(batch_count: usize) -> ProgressBar {
    if batch_count <= 1 || !stderr().is_terminal() {
        return ProgressBar::hidden();
    }

    let progress = ProgressBar::with_draw_target(
        Some(u64::try_from(batch_count).unwrap_or(u64::MAX)),
        ProgressDrawTarget::stderr(),
    );
    progress.set_style(
        ProgressStyle::with_template(
            "{spinner:.cyan} taxonomy batches [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len}",
        )
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .progress_chars("##-"),
    );
    progress.set_message("taxonomy batches");
    progress
}

pub fn validate_category_depth(categories: &[CategoryTree], max_depth: u8) -> Result<()> {
    for category in categories {
        let depth = tree_depth(category);
        if depth > usize::from(max_depth) {
            return Err(AppError::Validation(format!(
                "category '{}' depth {} exceeds allowed {}",
                category.name, depth, max_depth
            )));
        }
    }
    Ok(())
}

fn validate_category_names(categories: &[CategoryTree]) -> Result<()> {
    let mut sibling_names = HashSet::new();
    for cat in categories {
        let normalized = normalize_folder_name(&cat.name);
        if normalized.is_empty() {
            return Err(AppError::Validation(
                "category names cannot be empty".to_string(),
            ));
        }
        if !sibling_names.insert(normalized.clone()) {
            return Err(AppError::Validation(format!(
                "duplicate sibling category name '{}'",
                cat.name
            )));
        }
        validate_category_names(&cat.children)?;
    }
    Ok(())
}

fn tree_depth(node: &CategoryTree) -> usize {
    if node.children.is_empty() {
        1
    } else {
        1 + node.children.iter().map(tree_depth).max().unwrap_or(0)
    }
}

fn normalize_keyword(raw: &str) -> String {
    raw.trim().replace('\n', " ")
}

fn collect_unique_keywords(keyword_sets: &[KeywordSet]) -> Vec<String> {
    keyword_sets
        .iter()
        .flat_map(|set| set.keywords.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
}

fn build_category_prompt(unique_keywords: &[String], category_depth: u8) -> Result<String> {
    Ok(format!(
        "Return JSON with schema:\n{{\"categories\":[{{\"name\":\"...\",\"children\":[...]}}]}}\nRules:\n- category depth must be <= {category_depth}\n- names must be filesystem-friendly (letters, numbers, spaces, dashes)\n- avoid duplicate category names among siblings\n- output at least one top-level category\n\nkeywords:\n{}",
        serde_json::to_string_pretty(unique_keywords).map_err(AppError::from)?
    ))
}

fn build_merge_category_prompt(
    batch_summaries: &[CategoryBatchSummary],
    category_depth: u8,
) -> Result<String> {
    Ok(format!(
        "Return JSON with schema:\n{{\"categories\":[{{\"name\":\"...\",\"children\":[...]}}]}}\nRules:\n- merge the partial taxonomies below into one final taxonomy\n- category depth must be <= {category_depth}\n- names must be filesystem-friendly (letters, numbers, spaces, dashes)\n- avoid duplicate category names among siblings\n- output at least one top-level category\n- preserve strong concepts that recur across batches\n\nbatch_results:\n{}",
        serde_json::to_string_pretty(batch_summaries).map_err(AppError::from)?
    ))
}

async fn request_validated_categories(
    client: &dyn LlmClient,
    system: &str,
    base_user: &str,
    category_depth: u8,
) -> Result<Vec<CategoryTree>> {
    let mut user = base_user.to_string();
    let mut last_issue = String::new();

    for attempt in 1..=MAX_SEMANTIC_ATTEMPTS {
        let response: CategoryResponse =
            call_json_with_retry(client, system, &user, MAX_JSON_ATTEMPTS).await?;

        if response.categories.is_empty() {
            last_issue = "category synthesis returned no categories".to_string();
        } else if let Err(err) = validate_category_depth(&response.categories, category_depth) {
            last_issue = err.to_string();
        } else if let Err(err) = validate_category_names(&response.categories) {
            last_issue = err.to_string();
        } else {
            return Ok(response.categories);
        }

        if attempt < MAX_SEMANTIC_ATTEMPTS {
            user = format!(
                "{base_user}\n\nYour previous response failed validation with this error: {last_issue}.\nReturn corrected JSON that satisfies all rules."
            );
        }
    }

    Err(AppError::Validation(format!(
        "failed category synthesis validation: {last_issue}"
    )))
}

fn build_batch_keyword_prompt(batch: &[PaperText]) -> Result<String> {
    let per_file_limit =
        (MAX_TOTAL_BATCH_TEXT_CHARS / batch.len().max(1)).clamp(400, MAX_TEXT_CHARS_PER_FILE);

    let files = batch
        .iter()
        .map(|paper| {
            serde_json::json!({
                "file_id": paper.file_id,
                "path": paper.path.to_string_lossy(),
                "pages_read": paper.pages_read,
                "terms": truncate_for_prompt(&paper.llm_ready_text, per_file_limit),
            })
        })
        .collect::<Vec<_>>();

    Ok(format!(
        "Return JSON with this exact schema:\n{{\"pairs\":[{{\"file_id\":\"...\",\"keywords\":[\"...\"]}}]}}\nRules:\n- Return exactly {} pairs\n- Include every file_id exactly once\n- Keep 5 to 12 keywords for each file\n- Keywords must be specific nouns or short noun phrases\n- The `terms` field is a preprocessed deduplicated term list derived from the paper, not raw prose\n- No markdown\n\nfiles:\n{}",
        batch.len(),
        serde_json::to_string_pretty(&files).map_err(AppError::from)?
    ))
}

fn validate_keyword_batch_response(
    pairs: &[KeywordPair],
    batch: &[PaperText],
) -> Result<Vec<KeywordSet>> {
    if pairs.len() != batch.len() {
        return Err(AppError::Validation(format!(
            "pair count mismatch: expected {}, got {}",
            batch.len(),
            pairs.len()
        )));
    }

    let expected = batch
        .iter()
        .map(|paper| paper.file_id.as_str())
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    let mut keyword_map = HashMap::<String, Vec<String>>::new();

    for pair in pairs {
        if !expected.contains(pair.file_id.as_str()) {
            return Err(AppError::Validation(format!(
                "response contains unknown file_id '{}'",
                pair.file_id
            )));
        }
        if !seen.insert(pair.file_id.as_str()) {
            return Err(AppError::Validation(format!(
                "duplicate file_id '{}' in response",
                pair.file_id
            )));
        }

        let mut deduped = Vec::new();
        let mut seen_keywords = HashSet::new();
        for keyword in &pair.keywords {
            let k = normalize_keyword(keyword);
            if !k.is_empty() && seen_keywords.insert(k.clone()) {
                deduped.push(k);
            }
        }

        if deduped.is_empty() {
            return Err(AppError::Validation(format!(
                "keywords for file_id '{}' are empty after normalization",
                pair.file_id
            )));
        }

        keyword_map.insert(pair.file_id.clone(), deduped);
    }

    let mut out = Vec::with_capacity(batch.len());
    for paper in batch {
        let Some(keywords) = keyword_map.remove(&paper.file_id) else {
            return Err(AppError::Validation(format!(
                "missing keywords for expected file_id '{}'",
                paper.file_id
            )));
        };
        out.push(KeywordSet {
            file_id: paper.file_id.clone(),
            keywords,
        });
    }

    Ok(out)
}

fn format_batch_span(batch: &[PaperText]) -> String {
    let Some(first) = batch.first() else {
        return "<empty>".to_string();
    };
    let Some(last) = batch.last() else {
        return "<empty>".to_string();
    };
    format!(
        "{}..{} ({} files)",
        first.path.display(),
        last.path.display(),
        batch.len()
    )
}

fn normalize_folder_name(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == ' ' || *c == '-')
        .collect::<String>()
        .trim()
        .to_string()
}

fn truncate_for_prompt(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    input.chars().take(max_chars).collect::<String>()
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use async_trait::async_trait;

    use super::build_batch_keyword_prompt;
    use super::{
        KeywordPair, build_merge_category_prompt, synthesize_categories_batch_merged,
        validate_category_depth, validate_keyword_batch_response,
    };
    use crate::error::Result;
    use crate::llm::LlmClient;
    use crate::models::{CategoryTree, KeywordSet, PaperText};

    struct RoutingFakeClient {
        prompts: Mutex<Vec<String>>,
        merge_calls: AtomicUsize,
    }

    #[async_trait]
    impl LlmClient for RoutingFakeClient {
        async fn chat(&self, _system_prompt: &str, user_prompt: &str) -> Result<String> {
            self.prompts
                .lock()
                .expect("prompts lock")
                .push(user_prompt.to_string());

            if user_prompt.contains("batch_results") {
                self.merge_calls.fetch_add(1, Ordering::SeqCst);
                let batch_one = user_prompt.find("\"Batch One\"");
                let batch_two = user_prompt.find("\"Batch Two\"");
                assert!(matches!((batch_one, batch_two), (Some(a), Some(b)) if a < b));
                return Ok("{\"categories\":[{\"name\":\"Merged\",\"children\":[]}]}".to_string());
            }

            if user_prompt.contains("\"transformer\"") {
                std::thread::sleep(Duration::from_millis(40));
                return Ok(
                    "{\"categories\":[{\"name\":\"Batch One\",\"children\":[]}]}".to_string(),
                );
            }

            if user_prompt.contains("\"gan\"") {
                return Ok(
                    "{\"categories\":[{\"name\":\"Batch Two\",\"children\":[]}]}".to_string(),
                );
            }

            Err(crate::error::AppError::Llm(
                "unexpected fake prompt".to_string(),
            ))
        }
    }

    struct FailingBatchClient {
        prompts: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl LlmClient for FailingBatchClient {
        async fn chat(&self, _system_prompt: &str, user_prompt: &str) -> Result<String> {
            self.prompts
                .lock()
                .expect("prompts lock")
                .push(user_prompt.to_string());

            if user_prompt.contains("batch_results") {
                return Err(crate::error::AppError::Llm(
                    "merge should not be called after batch failure".to_string(),
                ));
            }

            if user_prompt.contains("\"gan\"") {
                return Err(crate::error::AppError::Llm(
                    "simulated batch failure".to_string(),
                ));
            }

            Ok("{\"categories\":[{\"name\":\"Batch One\",\"children\":[]}]}".to_string())
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
    fn validates_keyword_pairs_for_batch() {
        let batch = vec![
            PaperText {
                file_id: "a".to_string(),
                path: PathBuf::from("a.pdf"),
                extracted_text: "text".to_string(),
                llm_ready_text: "alpha beta".to_string(),
                pages_read: 1,
            },
            PaperText {
                file_id: "b".to_string(),
                path: PathBuf::from("b.pdf"),
                extracted_text: "text".to_string(),
                llm_ready_text: "gamma delta".to_string(),
                pages_read: 1,
            },
        ];

        let response = vec![
            KeywordPair {
                file_id: "a".to_string(),
                keywords: vec!["A".to_string(), "A".to_string(), "B".to_string()],
            },
            KeywordPair {
                file_id: "b".to_string(),
                keywords: vec!["C".to_string()],
            },
        ];

        let sets = validate_keyword_batch_response(&response, &batch).expect("valid pairs");
        assert_eq!(sets.len(), 2);
        assert_eq!(sets[0].file_id, "a");
        assert_eq!(sets[0].keywords, vec!["A".to_string(), "B".to_string()]);
        assert_eq!(sets[1].file_id, "b");
    }

    #[test]
    fn rejects_missing_file_id_in_keyword_pairs() {
        let batch = vec![
            PaperText {
                file_id: "a".to_string(),
                path: PathBuf::from("a.pdf"),
                extracted_text: "text".to_string(),
                llm_ready_text: "alpha beta".to_string(),
                pages_read: 1,
            },
            PaperText {
                file_id: "b".to_string(),
                path: PathBuf::from("b.pdf"),
                extracted_text: "text".to_string(),
                llm_ready_text: "gamma delta".to_string(),
                pages_read: 1,
            },
        ];

        let response = vec![KeywordPair {
            file_id: "a".to_string(),
            keywords: vec!["A".to_string()],
        }];

        let err = validate_keyword_batch_response(&response, &batch).expect_err("must fail");
        assert!(err.to_string().contains("pair count mismatch"));
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
        assert!(prompt.contains("\"terms\": \"graph neural network, node classification\""));
        assert!(!prompt.contains("raw prose sentence"));
    }

    #[test]
    fn merge_prompt_includes_partial_batch_taxonomies() {
        let prompt = build_merge_category_prompt(
            &[super::CategoryBatchSummary {
                batch_index: 1,
                file_ids: vec!["a".to_string(), "b".to_string()],
                keywords: vec!["transformer".to_string()],
                categories: vec![CategoryTree {
                    name: "Attention".to_string(),
                    children: vec![],
                }],
            }],
            2,
        )
        .expect("prompt");

        assert!(prompt.contains("\"batch_index\": 1"));
        assert!(prompt.contains("\"Attention\""));
        assert!(prompt.contains("\"transformer\""));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn batch_merge_mode_calls_final_merge_request_in_batch_order() {
        let raw_client = Arc::new(RoutingFakeClient {
            prompts: Mutex::new(Vec::new()),
            merge_calls: AtomicUsize::new(0),
        });
        let client: Arc<dyn LlmClient> = raw_client.clone();

        let papers = vec![
            make_paper("a"),
            make_paper("b"),
            make_paper("c"),
            make_paper("d"),
        ];
        let keyword_sets = vec![
            make_keyword_set("a", &["transformer"]),
            make_keyword_set("b", &["attention"]),
            make_keyword_set("c", &["gan"]),
            make_keyword_set("d", &["generator"]),
        ];

        let categories = synthesize_categories_batch_merged(client, &papers, &keyword_sets, 2, 2)
            .await
            .expect("batch merge categories");

        assert_eq!(categories.len(), 1);
        assert_eq!(categories[0].name, "Merged");
        assert_eq!(raw_client.merge_calls.load(Ordering::SeqCst), 1);

        let prompts = raw_client.prompts.lock().expect("prompts");
        assert_eq!(prompts.len(), 3);
        assert!(
            prompts
                .iter()
                .any(|prompt| prompt.contains("\"transformer\""))
        );
        assert!(prompts.iter().any(|prompt| prompt.contains("\"gan\"")));
        assert!(
            prompts
                .iter()
                .any(|prompt| prompt.contains("batch_results"))
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn batch_merge_mode_aborts_before_final_merge_on_batch_failure() {
        let raw_client = Arc::new(FailingBatchClient {
            prompts: Mutex::new(Vec::new()),
        });
        let client: Arc<dyn LlmClient> = raw_client.clone();

        let papers = vec![
            make_paper("a"),
            make_paper("b"),
            make_paper("c"),
            make_paper("d"),
        ];
        let keyword_sets = vec![
            make_keyword_set("a", &["transformer"]),
            make_keyword_set("b", &["attention"]),
            make_keyword_set("c", &["gan"]),
            make_keyword_set("d", &["generator"]),
        ];

        let err = synthesize_categories_batch_merged(client, &papers, &keyword_sets, 2, 2)
            .await
            .expect_err("batch merge should fail");

        assert!(err.to_string().contains("simulated batch failure"));

        let prompts = raw_client.prompts.lock().expect("prompts");
        assert!(
            !prompts
                .iter()
                .any(|prompt| prompt.contains("batch_results"))
        );
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

    fn make_keyword_set(id: &str, keywords: &[&str]) -> KeywordSet {
        KeywordSet {
            file_id: id.to_string(),
            keywords: keywords.iter().map(|keyword| keyword.to_string()).collect(),
        }
    }
}
