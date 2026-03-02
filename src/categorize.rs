use std::collections::{BTreeSet, HashMap, HashSet};

use serde::Deserialize;

use crate::{
    error::{AppError, Result},
    llm::{LlmClient, call_json_with_retry},
    models::{CategoryTree, KeywordSet, PaperText},
};

const MAX_JSON_ATTEMPTS: usize = 3;
const MAX_SEMANTIC_ATTEMPTS: usize = 3;
const MAX_TEXT_CHARS_PER_FILE: usize = 4_000;
const MAX_TOTAL_BATCH_TEXT_CHARS: usize = 60_000;

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

    let unique_keywords = keyword_sets
        .iter()
        .flat_map(|set| set.keywords.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    let system =
        "You design hierarchical folder taxonomies for academic PDFs. Return strict JSON only.";
    let base_user = format!(
        "Return JSON with schema:\n{{\"categories\":[{{\"name\":\"...\",\"children\":[...]}}]}}\nRules:\n- category depth must be <= {category_depth}\n- names must be filesystem-friendly (letters, numbers, spaces, dashes)\n- avoid duplicate category names among siblings\n- output at least one top-level category\n\nkeywords:\n{}",
        serde_json::to_string_pretty(&unique_keywords).map_err(AppError::from)?
    );
    let mut user = base_user.clone();
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
                "excerpt": truncate_for_prompt(&paper.extracted_text, per_file_limit),
            })
        })
        .collect::<Vec<_>>();

    Ok(format!(
        "Return JSON with this exact schema:\n{{\"pairs\":[{{\"file_id\":\"...\",\"keywords\":[\"...\"]}}]}}\nRules:\n- Return exactly {} pairs\n- Include every file_id exactly once\n- Keep 5 to 12 keywords for each file\n- Keywords must be specific nouns or short noun phrases\n- No markdown\n\nfiles:\n{}",
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
    use super::{KeywordPair, validate_category_depth, validate_keyword_batch_response};
    use crate::models::{CategoryTree, PaperText};
    use std::path::PathBuf;

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
                pages_read: 1,
            },
            PaperText {
                file_id: "b".to_string(),
                path: PathBuf::from("b.pdf"),
                extracted_text: "text".to_string(),
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
                pages_read: 1,
            },
            PaperText {
                file_id: "b".to_string(),
                path: PathBuf::from("b.pdf"),
                extracted_text: "text".to_string(),
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
}
