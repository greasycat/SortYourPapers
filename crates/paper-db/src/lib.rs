use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use directories::BaseDirs;
use duckdb::{Connection, params};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, PaperDbError>;

const DEFAULT_DB_FILE: &str = "paper-db.duckdb";

#[derive(Debug, Error)]
pub enum PaperDbError {
    #[error("paper-db configuration error: {0}")]
    Config(String),

    #[error("paper-db validation error: {0}")]
    Validation(String),

    #[error("paper-db storage error: {0}")]
    Storage(#[from] duckdb::Error),

    #[error("paper-db io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("paper-db json error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingModelId {
    pub provider: String,
    pub model: String,
}

impl EmbeddingModelId {
    #[must_use]
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EmbeddingCallMetrics {
    pub provider: String,
    pub model: String,
    pub endpoint_kind: String,
    pub request_chars: u64,
    pub response_chars: u64,
    pub http_attempt_count: u64,
    pub json_retry_count: u64,
    pub semantic_retry_count: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingVector {
    pub values: Vec<f32>,
}

impl EmbeddingVector {
    #[must_use]
    pub fn dimensions(&self) -> usize {
        self.values.len()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingRequest {
    pub inputs: Vec<String>,
}

impl EmbeddingRequest {
    #[must_use]
    pub fn from_texts<I, S>(inputs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            inputs: inputs.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingResponse {
    pub embeddings: Vec<EmbeddingVector>,
    pub metrics: EmbeddingCallMetrics,
}

#[async_trait]
pub trait EmbeddingClient: Send + Sync {
    async fn embed(&self, request: &EmbeddingRequest) -> Result<EmbeddingResponse>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaperInput {
    pub file_id: String,
    pub source_path: PathBuf,
    pub extracted_text: String,
    pub llm_ready_text: String,
    pub pages_read: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PaperRecord {
    pub file_id: String,
    pub source_path: PathBuf,
    pub extracted_text: String,
    pub llm_ready_text: String,
    pub pages_read: u8,
    pub content_hash: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PaperEmbeddingRecord {
    pub file_id: String,
    pub provider: String,
    pub model: String,
    pub dimensions: usize,
    pub embedding: Vec<f32>,
    pub embedded_text_hash: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferencePaperInput {
    pub paper_id: String,
    pub title: String,
    pub category: String,
    pub subcategory: String,
    pub abstract_excerpt: String,
    pub embedding_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceSetInput {
    pub set_id: String,
    pub manifest_path: PathBuf,
    pub manifest_fingerprint: String,
    pub papers: Vec<ReferencePaperInput>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceSetRecord {
    pub set_id: String,
    pub provider: String,
    pub model: String,
    pub manifest_path: PathBuf,
    pub manifest_fingerprint: String,
    pub paper_count: usize,
    pub indexed_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceMatchRecord {
    pub set_id: String,
    pub paper_id: String,
    pub title: String,
    pub category: String,
    pub subcategory: String,
    pub abstract_excerpt: String,
    pub embedding: Vec<f32>,
    pub similarity: f32,
}

#[derive(Debug, Clone, Default)]
pub struct EmbeddingSyncResult {
    pub papers_upserted: usize,
    pub embeddings_upserted: usize,
    pub embeddings_skipped: usize,
    pub metrics: Option<EmbeddingCallMetrics>,
}

#[derive(Debug, Clone, Default)]
pub struct ReferenceIndexSyncResult {
    pub papers_indexed: usize,
    pub embeddings_upserted: usize,
    pub skipped: bool,
    pub metrics: Option<EmbeddingCallMetrics>,
}

pub struct PaperDb {
    conn: Connection,
}

impl PaperDb {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(path)?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    pub fn default_path() -> Result<PathBuf> {
        let Some(base) = BaseDirs::new() else {
            return Err(PaperDbError::Config(
                "could not resolve XDG data directory".to_string(),
            ));
        };
        Ok(base.data_dir().join("sortyourpapers").join(DEFAULT_DB_FILE))
    }

    pub fn open_default() -> Result<Self> {
        Self::open(Self::default_path()?)
    }

    pub fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS papers (
                file_id TEXT PRIMARY KEY,
                source_path TEXT NOT NULL,
                extracted_text TEXT NOT NULL,
                llm_ready_text TEXT NOT NULL,
                pages_read INTEGER NOT NULL,
                content_hash TEXT NOT NULL,
                created_at_ms BIGINT NOT NULL,
                updated_at_ms BIGINT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS paper_embeddings (
                file_id TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                dimensions INTEGER NOT NULL,
                embedding_json TEXT NOT NULL,
                embedded_text_hash TEXT NOT NULL,
                created_at_ms BIGINT NOT NULL,
                updated_at_ms BIGINT NOT NULL,
                PRIMARY KEY (file_id, provider, model)
            );

            CREATE TABLE IF NOT EXISTS reference_sets (
                set_id TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                manifest_path TEXT NOT NULL,
                manifest_fingerprint TEXT NOT NULL,
                paper_count INTEGER NOT NULL,
                indexed_at_ms BIGINT NOT NULL,
                updated_at_ms BIGINT NOT NULL,
                PRIMARY KEY (set_id, provider, model)
            );

            CREATE TABLE IF NOT EXISTS reference_papers (
                set_id TEXT NOT NULL,
                paper_id TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                title TEXT NOT NULL,
                category TEXT NOT NULL,
                subcategory TEXT NOT NULL,
                abstract_excerpt TEXT NOT NULL,
                embedding_text TEXT NOT NULL,
                created_at_ms BIGINT NOT NULL,
                updated_at_ms BIGINT NOT NULL,
                PRIMARY KEY (set_id, paper_id, provider, model)
            );

            CREATE TABLE IF NOT EXISTS reference_embeddings (
                set_id TEXT NOT NULL,
                paper_id TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                dimensions INTEGER NOT NULL,
                embedding_json TEXT NOT NULL,
                embedded_text_hash TEXT NOT NULL,
                created_at_ms BIGINT NOT NULL,
                updated_at_ms BIGINT NOT NULL,
                PRIMARY KEY (set_id, paper_id, provider, model)
            );
            "#,
        )?;

        Ok(())
    }

    pub fn upsert_paper(&self, paper: &PaperInput) -> Result<PaperRecord> {
        let now = unix_ms()?;
        let content_hash = hash_paper_text(paper);

        self.conn.execute(
            r#"
            INSERT INTO papers (
                file_id, source_path, extracted_text, llm_ready_text, pages_read, content_hash, created_at_ms, updated_at_ms
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(file_id) DO UPDATE SET
                source_path = excluded.source_path,
                extracted_text = excluded.extracted_text,
                llm_ready_text = excluded.llm_ready_text,
                pages_read = excluded.pages_read,
                content_hash = excluded.content_hash,
                updated_at_ms = excluded.updated_at_ms
            "#,
            params![
                paper.file_id,
                paper.source_path.display().to_string(),
                paper.extracted_text,
                paper.llm_ready_text,
                i64::from(paper.pages_read),
                content_hash,
                now,
                now,
            ],
        )?;

        self.get_paper(&paper.file_id)?
            .ok_or_else(|| PaperDbError::Config("paper row was not persisted".to_string()))
    }

    pub fn get_paper(&self, file_id: &str) -> Result<Option<PaperRecord>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT file_id, source_path, extracted_text, llm_ready_text, pages_read, content_hash, created_at_ms, updated_at_ms
            FROM papers
            WHERE file_id = ?
            "#,
        )?;
        let mut rows = stmt.query(params![file_id])?;

        if let Some(row) = rows.next()? {
            return Ok(Some(PaperRecord {
                file_id: row.get(0)?,
                source_path: PathBuf::from(row.get::<_, String>(1)?),
                extracted_text: row.get(2)?,
                llm_ready_text: row.get(3)?,
                pages_read: u8::try_from(row.get::<_, i64>(4)?).map_err(|_| {
                    PaperDbError::Config("stored pages_read exceeded u8 range".to_string())
                })?,
                content_hash: row.get(5)?,
                created_at_ms: row.get(6)?,
                updated_at_ms: row.get(7)?,
            }));
        }

        Ok(None)
    }

    pub fn get_embedding(
        &self,
        file_id: &str,
        provider: &str,
        model: &str,
    ) -> Result<Option<PaperEmbeddingRecord>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT file_id, provider, model, dimensions, embedding_json, embedded_text_hash, created_at_ms, updated_at_ms
            FROM paper_embeddings
            WHERE file_id = ? AND provider = ? AND model = ?
            "#,
        )?;
        let mut rows = stmt.query(params![file_id, provider, model])?;

        if let Some(row) = rows.next()? {
            let embedding_json: String = row.get(4)?;
            let embedding = parse_embedding_json(&embedding_json)?;
            return Ok(Some(PaperEmbeddingRecord {
                file_id: row.get(0)?,
                provider: row.get(1)?,
                model: row.get(2)?,
                dimensions: usize::try_from(row.get::<_, i64>(3)?).map_err(|_| {
                    PaperDbError::Config("stored dimensions exceeded usize range".to_string())
                })?,
                embedding,
                embedded_text_hash: row.get(5)?,
                created_at_ms: row.get(6)?,
                updated_at_ms: row.get(7)?,
            }));
        }

        Ok(None)
    }

    pub async fn sync_embeddings(
        &self,
        papers: &[PaperInput],
        client: &dyn EmbeddingClient,
        model_id: &EmbeddingModelId,
    ) -> Result<EmbeddingSyncResult> {
        if papers.is_empty() {
            return Ok(EmbeddingSyncResult::default());
        }

        let mut result = EmbeddingSyncResult::default();
        let mut pending = Vec::new();

        for paper in papers {
            self.upsert_paper(paper)?;
            result.papers_upserted += 1;

            let embedded_text_hash = hash_text(&paper.llm_ready_text);
            match self.get_embedding(&paper.file_id, &model_id.provider, &model_id.model)? {
                Some(existing) if existing.embedded_text_hash == embedded_text_hash => {
                    result.embeddings_skipped += 1;
                }
                _ => pending.push((paper, embedded_text_hash)),
            }
        }

        if pending.is_empty() {
            return Ok(result);
        }

        let request = EmbeddingRequest::from_texts(
            pending
                .iter()
                .map(|(paper, _)| paper.llm_ready_text.clone())
                .collect::<Vec<_>>(),
        );
        let response = client.embed(&request).await?;
        if response.embeddings.len() != pending.len() {
            return Err(PaperDbError::Validation(format!(
                "embedding response count {} did not match pending paper count {}",
                response.embeddings.len(),
                pending.len()
            )));
        }

        result.metrics = Some(response.metrics.clone());
        for ((paper, embedded_text_hash), vector) in pending.into_iter().zip(response.embeddings) {
            self.upsert_embedding(&paper.file_id, model_id, &embedded_text_hash, &vector)?;
            result.embeddings_upserted += 1;
        }

        Ok(result)
    }

    pub fn get_reference_set_status(
        &self,
        set_id: &str,
        model_id: &EmbeddingModelId,
    ) -> Result<Option<ReferenceSetRecord>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT set_id, provider, model, manifest_path, manifest_fingerprint, paper_count, indexed_at_ms, updated_at_ms
            FROM reference_sets
            WHERE set_id = ? AND provider = ? AND model = ?
            "#,
        )?;
        let mut rows = stmt.query(params![set_id, model_id.provider, model_id.model])?;

        if let Some(row) = rows.next()? {
            return Ok(Some(ReferenceSetRecord {
                set_id: row.get(0)?,
                provider: row.get(1)?,
                model: row.get(2)?,
                manifest_path: PathBuf::from(row.get::<_, String>(3)?),
                manifest_fingerprint: row.get(4)?,
                paper_count: usize::try_from(row.get::<_, i64>(5)?).map_err(|_| {
                    PaperDbError::Config("stored paper_count exceeded usize range".to_string())
                })?,
                indexed_at_ms: row.get(6)?,
                updated_at_ms: row.get(7)?,
            }));
        }

        Ok(None)
    }

    pub async fn sync_reference_set(
        &self,
        set: &ReferenceSetInput,
        client: &dyn EmbeddingClient,
        model_id: &EmbeddingModelId,
        force: bool,
    ) -> Result<ReferenceIndexSyncResult> {
        if set.papers.is_empty() {
            return Err(PaperDbError::Validation(
                "reference set must include at least one paper".to_string(),
            ));
        }

        if !force
            && let Some(status) = self.get_reference_set_status(&set.set_id, model_id)?
            && status.manifest_fingerprint == set.manifest_fingerprint
            && status.paper_count == set.papers.len()
        {
            return Ok(ReferenceIndexSyncResult {
                papers_indexed: status.paper_count,
                skipped: true,
                ..ReferenceIndexSyncResult::default()
            });
        }

        let request = EmbeddingRequest::from_texts(
            set.papers
                .iter()
                .map(|paper| paper.embedding_text.clone())
                .collect::<Vec<_>>(),
        );
        let response = client.embed(&request).await?;
        if response.embeddings.len() != set.papers.len() {
            return Err(PaperDbError::Validation(format!(
                "embedding response count {} did not match reference paper count {}",
                response.embeddings.len(),
                set.papers.len()
            )));
        }

        self.delete_reference_rows(&set.set_id, model_id)?;

        let now = unix_ms()?;
        self.conn.execute(
            r#"
            INSERT INTO reference_sets (
                set_id, provider, model, manifest_path, manifest_fingerprint, paper_count, indexed_at_ms, updated_at_ms
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(set_id, provider, model) DO UPDATE SET
                manifest_path = excluded.manifest_path,
                manifest_fingerprint = excluded.manifest_fingerprint,
                paper_count = excluded.paper_count,
                updated_at_ms = excluded.updated_at_ms,
                indexed_at_ms = excluded.indexed_at_ms
            "#,
            params![
                set.set_id,
                model_id.provider,
                model_id.model,
                set.manifest_path.display().to_string(),
                set.manifest_fingerprint,
                i64::try_from(set.papers.len()).map_err(|_| {
                    PaperDbError::Config("paper count exceeded i64 range".to_string())
                })?,
                now,
                now,
            ],
        )?;

        for (paper, vector) in set.papers.iter().zip(response.embeddings.iter()) {
            let embedded_text_hash = hash_text(&paper.embedding_text);
            self.conn.execute(
                r#"
                INSERT INTO reference_papers (
                    set_id, paper_id, provider, model, title, category, subcategory, abstract_excerpt, embedding_text, created_at_ms, updated_at_ms
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
                params![
                    set.set_id,
                    paper.paper_id,
                    model_id.provider,
                    model_id.model,
                    paper.title,
                    paper.category,
                    paper.subcategory,
                    paper.abstract_excerpt,
                    paper.embedding_text,
                    now,
                    now,
                ],
            )?;
            self.conn.execute(
                r#"
                INSERT INTO reference_embeddings (
                    set_id, paper_id, provider, model, dimensions, embedding_json, embedded_text_hash, created_at_ms, updated_at_ms
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
                params![
                    set.set_id,
                    paper.paper_id,
                    model_id.provider,
                    model_id.model,
                    i64::try_from(vector.dimensions()).map_err(|_| {
                        PaperDbError::Config("embedding dimensions exceeded i64 range".to_string())
                    })?,
                    serde_json::to_string(&vector.values)?,
                    embedded_text_hash,
                    now,
                    now,
                ],
            )?;
        }

        Ok(ReferenceIndexSyncResult {
            papers_indexed: set.papers.len(),
            embeddings_upserted: set.papers.len(),
            skipped: false,
            metrics: Some(response.metrics),
        })
    }

    pub fn nearest_reference_matches(
        &self,
        model_id: &EmbeddingModelId,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<ReferenceMatchRecord>> {
        if query_embedding.is_empty() {
            return Ok(Vec::new());
        }
        if limit == 0 {
            return Ok(Vec::new());
        }

        let mut stmt = self.conn.prepare(
            r#"
            SELECT rp.set_id, rp.paper_id, rp.title, rp.category, rp.subcategory, rp.abstract_excerpt, re.embedding_json
            FROM reference_papers rp
            JOIN reference_embeddings re
              ON rp.set_id = re.set_id
             AND rp.paper_id = re.paper_id
             AND rp.provider = re.provider
             AND rp.model = re.model
            WHERE rp.provider = ? AND rp.model = ?
            "#,
        )?;
        let mut rows = stmt.query(params![model_id.provider, model_id.model])?;
        let mut matches = Vec::new();

        while let Some(row) = rows.next()? {
            let embedding = parse_embedding_json(&row.get::<_, String>(6)?)?;
            if embedding.len() != query_embedding.len() {
                continue;
            }

            let similarity = cosine_similarity(query_embedding, &embedding);
            if similarity <= 0.0 {
                continue;
            }

            matches.push(ReferenceMatchRecord {
                set_id: row.get(0)?,
                paper_id: row.get(1)?,
                title: row.get(2)?,
                category: row.get(3)?,
                subcategory: row.get(4)?,
                abstract_excerpt: row.get(5)?,
                embedding,
                similarity,
            });
        }

        matches.sort_by(|left, right| {
            right
                .similarity
                .partial_cmp(&left.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.paper_id.cmp(&right.paper_id))
        });
        matches.truncate(limit);
        Ok(matches)
    }

    fn upsert_embedding(
        &self,
        file_id: &str,
        model_id: &EmbeddingModelId,
        embedded_text_hash: &str,
        vector: &EmbeddingVector,
    ) -> Result<()> {
        let now = unix_ms()?;
        let embedding_json = serde_json::to_string(&vector.values)?;

        self.conn.execute(
            r#"
            INSERT INTO paper_embeddings (
                file_id, provider, model, dimensions, embedding_json, embedded_text_hash, created_at_ms, updated_at_ms
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(file_id, provider, model) DO UPDATE SET
                dimensions = excluded.dimensions,
                embedding_json = excluded.embedding_json,
                embedded_text_hash = excluded.embedded_text_hash,
                updated_at_ms = excluded.updated_at_ms
            "#,
            params![
                file_id,
                model_id.provider,
                model_id.model,
                i64::try_from(vector.dimensions()).map_err(|_| {
                    PaperDbError::Config("embedding dimensions exceeded i64 range".to_string())
                })?,
                embedding_json,
                embedded_text_hash,
                now,
                now,
            ],
        )?;

        Ok(())
    }

    fn delete_reference_rows(&self, set_id: &str, model_id: &EmbeddingModelId) -> Result<()> {
        self.conn.execute(
            "DELETE FROM reference_embeddings WHERE set_id = ? AND provider = ? AND model = ?",
            params![set_id, model_id.provider, model_id.model],
        )?;
        self.conn.execute(
            "DELETE FROM reference_papers WHERE set_id = ? AND provider = ? AND model = ?",
            params![set_id, model_id.provider, model_id.model],
        )?;
        self.conn.execute(
            "DELETE FROM reference_sets WHERE set_id = ? AND provider = ? AND model = ?",
            params![set_id, model_id.provider, model_id.model],
        )?;
        Ok(())
    }
}

fn hash_paper_text(paper: &PaperInput) -> String {
    let mut hasher = DefaultHasher::new();
    paper.file_id.hash(&mut hasher);
    paper.source_path.hash(&mut hasher);
    paper.extracted_text.hash(&mut hasher);
    paper.llm_ready_text.hash(&mut hasher);
    paper.pages_read.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn hash_text(text: &str) -> String {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn parse_embedding_json(raw: &str) -> Result<Vec<f32>> {
    serde_json::from_str(raw).map_err(PaperDbError::from)
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    let mut dot = 0.0_f32;
    let mut left_norm = 0.0_f32;
    let mut right_norm = 0.0_f32;

    for (left_value, right_value) in left.iter().zip(right.iter()) {
        dot += left_value * right_value;
        left_norm += left_value * left_value;
        right_norm += right_value * right_value;
    }

    let denominator = left_norm.sqrt() * right_norm.sqrt();
    if denominator == 0.0 {
        0.0
    } else {
        dot / denominator
    }
}

fn unix_ms() -> Result<i64> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| PaperDbError::Config(format!("system clock error: {err}")))?;
    i64::try_from(now.as_millis())
        .map_err(|_| PaperDbError::Config("timestamp exceeded i64 range".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env,
        sync::{Mutex, OnceLock},
    };
    use tempfile::tempdir;

    fn sample_paper(text: &str) -> PaperInput {
        PaperInput {
            file_id: "paper-001".to_string(),
            source_path: PathBuf::from("/papers/paper-001.pdf"),
            extracted_text: format!("raw {text}"),
            llm_ready_text: text.to_string(),
            pages_read: 2,
        }
    }

    fn sample_model_id() -> EmbeddingModelId {
        EmbeddingModelId::new("openai", "text-embedding-3-small")
    }

    #[derive(Default)]
    struct StubEmbeddingClient {
        calls: Mutex<usize>,
    }

    #[async_trait]
    impl EmbeddingClient for StubEmbeddingClient {
        async fn embed(&self, request: &EmbeddingRequest) -> Result<EmbeddingResponse> {
            *self.calls.lock().expect("calls lock") += 1;
            Ok(EmbeddingResponse {
                embeddings: request
                    .inputs
                    .iter()
                    .map(|input| EmbeddingVector {
                        values: if input.contains("vision") {
                            vec![0.0, 1.0]
                        } else {
                            vec![1.0, 0.0]
                        },
                    })
                    .collect(),
                metrics: EmbeddingCallMetrics {
                    provider: "openai".to_string(),
                    model: "text-embedding-3-small".to_string(),
                    endpoint_kind: "embeddings".to_string(),
                    request_chars: request.inputs.iter().map(|input| input.len() as u64).sum(),
                    response_chars: 0,
                    ..EmbeddingCallMetrics::default()
                },
            })
        }
    }

    #[test]
    fn default_path_uses_xdg_data_home() {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let _guard = LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("xdg lock");

        let temp = tempdir().expect("tempdir");
        let previous = env::var_os("XDG_DATA_HOME");
        unsafe { env::set_var("XDG_DATA_HOME", temp.path()) };

        let path = PaperDb::default_path().expect("default path");

        match previous {
            Some(value) => unsafe { env::set_var("XDG_DATA_HOME", value) },
            None => unsafe { env::remove_var("XDG_DATA_HOME") },
        }

        assert_eq!(
            path,
            temp.path().join("sortyourpapers").join(DEFAULT_DB_FILE)
        );
    }

    #[test]
    fn upsert_paper_persists_round_trip_record() {
        let dir = tempdir().expect("tempdir");
        let db = PaperDb::open(dir.path().join("paper-db.duckdb")).expect("open db");
        let paper = sample_paper("embedding ready");

        let stored = db.upsert_paper(&paper).expect("upsert paper");

        assert_eq!(stored.file_id, paper.file_id);
        assert_eq!(stored.llm_ready_text, paper.llm_ready_text);
        assert_eq!(
            db.get_paper(&paper.file_id)
                .expect("get paper")
                .expect("stored paper"),
            stored
        );
    }

    #[tokio::test]
    async fn sync_embeddings_is_idempotent_for_unchanged_text() {
        let dir = tempdir().expect("tempdir");
        let db = PaperDb::open(dir.path().join("paper-db.duckdb")).expect("open db");
        let paper = sample_paper("embedding ready");
        let client = StubEmbeddingClient::default();
        let model_id = sample_model_id();

        let first = db
            .sync_embeddings(&[paper.clone()], &client, &model_id)
            .await
            .expect("first sync");
        let second = db
            .sync_embeddings(&[paper.clone()], &client, &model_id)
            .await
            .expect("second sync");

        assert_eq!(first.embeddings_upserted, 1);
        assert_eq!(second.embeddings_upserted, 0);
        assert_eq!(second.embeddings_skipped, 1);
        assert_eq!(*client.calls.lock().expect("calls lock"), 1);

        let stored = db
            .get_embedding(&paper.file_id, &model_id.provider, &model_id.model)
            .expect("get embedding")
            .expect("stored embedding");
        assert_eq!(stored.dimensions, 2);
    }

    #[tokio::test]
    async fn sync_embeddings_rewrites_stale_rows_when_text_changes() {
        let dir = tempdir().expect("tempdir");
        let db = PaperDb::open(dir.path().join("paper-db.duckdb")).expect("open db");
        let client = StubEmbeddingClient::default();
        let model_id = sample_model_id();
        let original = sample_paper("first text");
        let updated = sample_paper("second text");

        db.sync_embeddings(&[original.clone()], &client, &model_id)
            .await
            .expect("first sync");
        db.sync_embeddings(&[updated.clone()], &client, &model_id)
            .await
            .expect("second sync");

        assert_eq!(*client.calls.lock().expect("calls lock"), 2);
        let stored = db
            .get_embedding(&updated.file_id, &model_id.provider, &model_id.model)
            .expect("get embedding")
            .expect("stored embedding");
        assert_eq!(stored.embedding, vec![1.0, 0.0]);
    }

    #[tokio::test]
    async fn sync_reference_set_skips_unchanged_manifest() {
        let dir = tempdir().expect("tempdir");
        let db = PaperDb::open(dir.path().join("paper-db.duckdb")).expect("open db");
        let client = StubEmbeddingClient::default();
        let model_id = sample_model_id();
        let reference_set = ReferenceSetInput {
            set_id: "demo".to_string(),
            manifest_path: PathBuf::from("assets/testsets/demo.toml"),
            manifest_fingerprint: "abc123".to_string(),
            papers: vec![ReferencePaperInput {
                paper_id: "paper-a".to_string(),
                title: "Graph paper".to_string(),
                category: "Computer Science".to_string(),
                subcategory: "cs.LG".to_string(),
                abstract_excerpt: "Graph neural networks".to_string(),
                embedding_text: "graph neural networks".to_string(),
            }],
        };

        let first = db
            .sync_reference_set(&reference_set, &client, &model_id, false)
            .await
            .expect("first index");
        let second = db
            .sync_reference_set(&reference_set, &client, &model_id, false)
            .await
            .expect("second index");

        assert_eq!(first.papers_indexed, 1);
        assert!(second.skipped);
        assert_eq!(*client.calls.lock().expect("calls lock"), 1);
    }

    #[tokio::test]
    async fn nearest_reference_matches_returns_top_cosine_hits() {
        let dir = tempdir().expect("tempdir");
        let db = PaperDb::open(dir.path().join("paper-db.duckdb")).expect("open db");
        let client = StubEmbeddingClient::default();
        let model_id = sample_model_id();
        let reference_set = ReferenceSetInput {
            set_id: "demo".to_string(),
            manifest_path: PathBuf::from("assets/testsets/demo.toml"),
            manifest_fingerprint: "fingerprint".to_string(),
            papers: vec![
                ReferencePaperInput {
                    paper_id: "graph-paper".to_string(),
                    title: "Graph paper".to_string(),
                    category: "Computer Science".to_string(),
                    subcategory: "cs.LG".to_string(),
                    abstract_excerpt: "graph methods".to_string(),
                    embedding_text: "graph learning".to_string(),
                },
                ReferencePaperInput {
                    paper_id: "vision-paper".to_string(),
                    title: "Vision paper".to_string(),
                    category: "Computer Science".to_string(),
                    subcategory: "cs.CV".to_string(),
                    abstract_excerpt: "vision methods".to_string(),
                    embedding_text: "vision transformer".to_string(),
                },
            ],
        };

        db.sync_reference_set(&reference_set, &client, &model_id, true)
            .await
            .expect("index reference set");

        let matches = db
            .nearest_reference_matches(&model_id, &[0.9, 0.1], 1)
            .expect("nearest matches");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].paper_id, "graph-paper");
        assert!(matches[0].similarity > 0.9);
    }
}
