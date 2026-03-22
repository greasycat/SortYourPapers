use std::collections::BTreeMap;

use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use serde::Deserialize;
use syp_core::error::{AppError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetSplitSource {
    pub split_name: String,
    pub path: String,
}

impl DatasetSplitSource {
    #[must_use]
    pub fn new(split_name: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            split_name: split_name.into(),
            path: path.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SciJudgeBenchSourceConfig {
    pub dataset_repo: String,
    pub revision: String,
    pub base_url: String,
    pub hf_token: Option<String>,
    pub split_sources: Vec<DatasetSplitSource>,
}

impl Default for SciJudgeBenchSourceConfig {
    fn default() -> Self {
        Self {
            dataset_repo: "OpenMOSS-Team/SciJudgeBench".to_string(),
            revision: "main".to_string(),
            base_url: "https://huggingface.co/datasets".to_string(),
            hf_token: None,
            split_sources: vec![
                DatasetSplitSource::new("train", "train.jsonl"),
                DatasetSplitSource::new("test", "test.jsonl"),
                DatasetSplitSource::new("test_ood_iclr", "test_ood_iclr.jsonl"),
                DatasetSplitSource::new("test_ood_year", "test_ood_year.jsonl"),
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SciJudgePaperCandidate {
    pub arxiv_id: String,
    pub title: String,
    pub abstract_text: String,
    pub category: String,
    pub subcategory: String,
    pub citations: u64,
    pub date: Option<String>,
    pub source_splits: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct DatasetRow {
    #[serde(default)]
    paper_a_title: Option<String>,
    #[serde(default)]
    paper_b_title: Option<String>,
    #[serde(default)]
    paper_a_abstract: Option<String>,
    #[serde(default)]
    paper_b_abstract: Option<String>,
    #[serde(default)]
    paper_a_arxiv_id: Option<String>,
    #[serde(default)]
    paper_b_arxiv_id: Option<String>,
    #[serde(default)]
    paper_a_category: Option<String>,
    #[serde(default)]
    paper_b_category: Option<String>,
    #[serde(default)]
    paper_a_subcategory: Option<String>,
    #[serde(default)]
    paper_b_subcategory: Option<String>,
    #[serde(default)]
    paper_a_citations: Option<u64>,
    #[serde(default)]
    paper_b_citations: Option<u64>,
    #[serde(default)]
    paper_a_date: Option<String>,
    #[serde(default)]
    paper_b_date: Option<String>,
}

#[derive(Debug, Clone)]
struct PartialPaperRecord {
    arxiv_id: String,
    title: String,
    abstract_text: String,
    category: String,
    subcategory: String,
    citations: u64,
    date: Option<String>,
}

#[derive(Debug, Default)]
struct CandidateAccumulator {
    title: String,
    abstract_text: String,
    category: String,
    subcategory: String,
    citations: u64,
    date: Option<String>,
    source_splits: Vec<String>,
}

pub async fn load_scijudgebench_catalog(
    config: &SciJudgeBenchSourceConfig,
) -> Result<Vec<SciJudgePaperCandidate>> {
    validate_source_config(config)?;

    let client = build_client(config)?;
    let mut candidates = BTreeMap::<String, CandidateAccumulator>::new();

    for split in &config.split_sources {
        let url = split_url(config, split);
        let body = client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        let rows = parse_dataset_rows(&body)?;
        for row in rows {
            for partial in row.into_partial_records() {
                let arxiv_id = partial.arxiv_id.clone();
                let entry = candidates.entry(arxiv_id).or_default();
                merge_candidate(entry, partial, &split.split_name);
            }
        }
    }

    Ok(candidates
        .into_iter()
        .map(|(arxiv_id, candidate)| SciJudgePaperCandidate {
            arxiv_id,
            title: candidate.title,
            abstract_text: candidate.abstract_text,
            category: candidate.category,
            subcategory: candidate.subcategory,
            citations: candidate.citations,
            date: candidate.date,
            source_splits: candidate.source_splits,
        })
        .collect())
}

fn validate_source_config(config: &SciJudgeBenchSourceConfig) -> Result<()> {
    if config.dataset_repo.trim().is_empty() {
        return Err(AppError::Config(
            "dataset repo must not be empty".to_string(),
        ));
    }
    if config.revision.trim().is_empty() {
        return Err(AppError::Config(
            "dataset revision must not be empty".to_string(),
        ));
    }
    if config.base_url.trim().is_empty() {
        return Err(AppError::Config(
            "dataset base url must not be empty".to_string(),
        ));
    }
    if config.split_sources.is_empty() {
        return Err(AppError::Config(
            "at least one dataset split source is required".to_string(),
        ));
    }

    Ok(())
}

fn build_client(config: &SciJudgeBenchSourceConfig) -> Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("sortyourpapers-paperfetch/0.1"),
    );

    if let Some(token) = &config.hf_token {
        let value = format!("Bearer {}", token.trim());
        let header = HeaderValue::from_str(&value).map_err(|err| {
            AppError::Config(format!("invalid hugging face token header value: {err}"))
        })?;
        headers.insert(AUTHORIZATION, header);
    }

    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .map_err(AppError::from)
}

fn split_url(config: &SciJudgeBenchSourceConfig, split: &DatasetSplitSource) -> String {
    format!(
        "{}/{}/resolve/{}/{}",
        config.base_url.trim_end_matches('/'),
        config.dataset_repo.trim_matches('/'),
        config.revision.trim_matches('/'),
        split.path.trim_start_matches('/')
    )
}

fn parse_dataset_rows(raw: &str) -> Result<Vec<DatasetRow>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    if trimmed.starts_with('[') {
        return serde_json::from_str(trimmed).map_err(AppError::from);
    }

    trimmed
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<DatasetRow>(line).map_err(AppError::from))
        .collect()
}

fn merge_candidate(
    candidate: &mut CandidateAccumulator,
    partial: PartialPaperRecord,
    split_name: &str,
) {
    if candidate.title.is_empty() {
        candidate.title = partial.title;
    }
    if candidate.abstract_text.is_empty() {
        candidate.abstract_text = partial.abstract_text;
    }
    if candidate.category.is_empty() {
        candidate.category = partial.category;
    }
    if candidate.subcategory.is_empty() {
        candidate.subcategory = partial.subcategory;
    }
    if candidate.date.is_none() {
        candidate.date = partial.date;
    }
    candidate.citations = candidate.citations.max(partial.citations);
    if !candidate
        .source_splits
        .iter()
        .any(|item| item == split_name)
    {
        candidate.source_splits.push(split_name.to_string());
        candidate.source_splits.sort();
    }
}

impl DatasetRow {
    fn into_partial_records(self) -> Vec<PartialPaperRecord> {
        let mut out = Vec::new();
        if let Some(record) = partial_from_fields(
            self.paper_a_arxiv_id,
            self.paper_a_title,
            self.paper_a_abstract,
            self.paper_a_category,
            self.paper_a_subcategory,
            self.paper_a_citations,
            self.paper_a_date,
        ) {
            out.push(record);
        }
        if let Some(record) = partial_from_fields(
            self.paper_b_arxiv_id,
            self.paper_b_title,
            self.paper_b_abstract,
            self.paper_b_category,
            self.paper_b_subcategory,
            self.paper_b_citations,
            self.paper_b_date,
        ) {
            out.push(record);
        }
        out
    }
}

fn partial_from_fields(
    arxiv_id: Option<String>,
    title: Option<String>,
    abstract_text: Option<String>,
    category: Option<String>,
    subcategory: Option<String>,
    citations: Option<u64>,
    date: Option<String>,
) -> Option<PartialPaperRecord> {
    let arxiv_id = normalize_arxiv_id(arxiv_id?)?;
    Some(PartialPaperRecord {
        arxiv_id,
        title: clean_text(title),
        abstract_text: clean_text(abstract_text),
        category: clean_text(category),
        subcategory: clean_text(subcategory),
        citations: citations.unwrap_or(0),
        date: date
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
    })
}

fn clean_text(value: Option<String>) -> String {
    value
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .unwrap_or_default()
}

fn normalize_arxiv_id(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let normalized = trimmed
        .trim_start_matches("https://arxiv.org/abs/")
        .trim_start_matches("http://arxiv.org/abs/")
        .trim_start_matches("https://arxiv.org/pdf/")
        .trim_start_matches("http://arxiv.org/pdf/")
        .trim_end_matches(".pdf")
        .trim_matches('/');

    if normalized.is_empty() {
        return None;
    }

    Some(normalized.to_string())
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    use super::{
        DatasetRow, DatasetSplitSource, SciJudgeBenchSourceConfig, load_scijudgebench_catalog,
        parse_dataset_rows,
    };

    #[test]
    fn jsonl_rows_parse() {
        let raw = r#"{"paper_a_arxiv_id":"1234.5678","paper_a_title":"A","paper_a_category":"CS","paper_a_subcategory":"cs.AI","paper_a_citations":10}
{"paper_b_arxiv_id":"2345.6789","paper_b_title":"B","paper_b_category":"Math","paper_b_subcategory":"math.CO","paper_b_citations":4}"#;

        let rows = parse_dataset_rows(raw).expect("parse rows");

        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn array_rows_parse() {
        let raw = r#"[{"paper_a_arxiv_id":"1234.5678","paper_a_title":"A","paper_a_category":"CS","paper_a_subcategory":"cs.AI","paper_a_citations":10}]"#;

        let rows = parse_dataset_rows(raw).expect("parse rows");

        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn dataset_row_flattens_both_papers() {
        let row = DatasetRow {
            paper_a_title: Some("Paper A".to_string()),
            paper_b_title: Some("Paper B".to_string()),
            paper_a_abstract: Some("Abstract A".to_string()),
            paper_b_abstract: Some("Abstract B".to_string()),
            paper_a_arxiv_id: Some("1234.5678".to_string()),
            paper_b_arxiv_id: Some("https://arxiv.org/pdf/2345.6789.pdf".to_string()),
            paper_a_category: Some("CS".to_string()),
            paper_b_category: Some("Math".to_string()),
            paper_a_subcategory: Some("cs.AI".to_string()),
            paper_b_subcategory: Some("math.CO".to_string()),
            paper_a_citations: Some(10),
            paper_b_citations: Some(4),
            paper_a_date: Some("2024-01-01T00:00:00".to_string()),
            paper_b_date: None,
        };

        let flattened = row.into_partial_records();

        assert_eq!(flattened.len(), 2);
        assert_eq!(flattened[1].arxiv_id, "2345.6789");
    }

    #[tokio::test]
    async fn catalog_load_dedupes_across_splits() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept");
                let mut request = [0_u8; 4096];
                let size = stream.read(&mut request).expect("read request");
                let request = String::from_utf8_lossy(&request[..size]);
                let line = request.lines().next().expect("request line");
                let path = line.split_whitespace().nth(1).expect("request path");
                let body = if path.ends_with("/train.jsonl") {
                    r#"{"paper_a_arxiv_id":"1234.5678","paper_a_title":"Paper A","paper_a_category":"CS","paper_a_subcategory":"cs.AI","paper_a_citations":10}"#
                } else {
                    r#"{"paper_b_arxiv_id":"1234.5678","paper_b_title":"Paper A","paper_b_category":"CS","paper_b_subcategory":"cs.AI","paper_b_citations":25}"#
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
            }
        });

        let catalog = load_scijudgebench_catalog(&SciJudgeBenchSourceConfig {
            dataset_repo: "OpenMOSS-Team/SciJudgeBench".to_string(),
            revision: "main".to_string(),
            base_url: format!("http://{addr}/datasets"),
            hf_token: None,
            split_sources: vec![
                DatasetSplitSource::new("train", "train.jsonl"),
                DatasetSplitSource::new("test", "test.jsonl"),
            ],
        })
        .await
        .expect("load catalog");

        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].citations, 25);
        assert_eq!(
            catalog[0].source_splits,
            vec!["test".to_string(), "train".to_string()]
        );
    }
}
