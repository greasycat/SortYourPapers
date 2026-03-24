use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use duckdb::{Connection, params};
use serde_json::Value;
use syp_core::{
    config,
    llm::{
        EmbeddingClient, EmbeddingConfig, EmbeddingRequest, EmbeddingVector, LlmCallMetrics,
        LlmProvider,
    },
    papers::PaperText,
};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, PaperDbError>;

const DEFAULT_DB_FILE: &str = "paperdb.duckdb";

#[derive(Debug, Error)]
pub enum PaperDbError {
    #[error("paperdb configuration error: {0}")]
    Config(String),

    #[error("paperdb validation error: {0}")]
    Validation(String),

    #[error("paperdb storage error: {0}")]
    Storage(#[from] duckdb::Error),

    #[error("paperdb io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("paperdb json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("paperdb llm error: {0}")]
    Llm(#[from] syp_core::error::AppError),
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

#[derive(Debug, Clone, Default)]
pub struct EmbeddingSyncResult {
    pub papers_upserted: usize,
    pub embeddings_upserted: usize,
    pub embeddings_skipped: usize,
    pub metrics: Option<LlmCallMetrics>,
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
        let Some(root) = config::xdg_data_dir() else {
            return Err(PaperDbError::Config(
                "could not resolve XDG data directory".to_string(),
            ));
        };
        Ok(root.join(DEFAULT_DB_FILE))
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
            "#,
        )?;

        Ok(())
    }

    pub fn upsert_paper(&self, paper: &PaperText) -> Result<PaperRecord> {
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
                paper.path.display().to_string(),
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
        papers: &[PaperText],
        client: &dyn EmbeddingClient,
        config: &EmbeddingConfig,
    ) -> Result<EmbeddingSyncResult> {
        if papers.is_empty() {
            return Ok(EmbeddingSyncResult::default());
        }

        let provider = provider_name(config.provider);
        let mut result = EmbeddingSyncResult::default();
        let mut pending = Vec::new();

        for paper in papers {
            self.upsert_paper(paper)?;
            result.papers_upserted += 1;

            let embedded_text_hash = hash_text(&paper.llm_ready_text);
            match self.get_embedding(&paper.file_id, provider, &config.model)? {
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
            self.upsert_embedding(
                &paper.file_id,
                provider,
                &config.model,
                &embedded_text_hash,
                &vector,
            )?;
            result.embeddings_upserted += 1;
        }

        Ok(result)
    }

    fn upsert_embedding(
        &self,
        file_id: &str,
        provider: &str,
        model: &str,
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
                provider,
                model,
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
}

fn hash_paper_text(paper: &PaperText) -> String {
    let mut hasher = DefaultHasher::new();
    paper.file_id.hash(&mut hasher);
    paper.path.hash(&mut hasher);
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
    let values = serde_json::from_str::<Vec<Value>>(raw)?;
    values
        .into_iter()
        .map(|value| {
            value.as_f64().map(|number| number as f32).ok_or_else(|| {
                PaperDbError::Config("embedding payload contained non-number".to_string())
            })
        })
        .collect()
}

fn provider_name(provider: LlmProvider) -> &'static str {
    match provider {
        LlmProvider::Openai => "openai",
        LlmProvider::Ollama => "ollama",
        LlmProvider::Gemini => "gemini",
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
    use async_trait::async_trait;
    use std::{
        env,
        sync::{Mutex, OnceLock},
    };
    use tempfile::tempdir;

    fn sample_paper(text: &str) -> PaperText {
        PaperText {
            file_id: "paper-001".to_string(),
            path: PathBuf::from("/papers/paper-001.pdf"),
            extracted_text: format!("raw {text}"),
            llm_ready_text: text.to_string(),
            pages_read: 2,
        }
    }

    fn sample_embedding_config() -> EmbeddingConfig {
        EmbeddingConfig {
            provider: LlmProvider::Openai,
            model: "text-embedding-3-small".to_string(),
            base_url: None,
            api_key: None,
        }
    }

    #[derive(Default)]
    struct StubEmbeddingClient {
        calls: Mutex<usize>,
    }

    #[async_trait]
    impl EmbeddingClient for StubEmbeddingClient {
        async fn embed(
            &self,
            request: &EmbeddingRequest,
        ) -> syp_core::error::Result<syp_core::llm::EmbeddingResponse> {
            *self.calls.lock().expect("calls lock") += 1;
            Ok(syp_core::llm::EmbeddingResponse {
                embeddings: request
                    .inputs
                    .iter()
                    .map(|input| EmbeddingVector {
                        values: vec![input.text.len() as f32, 1.0],
                    })
                    .collect(),
                metrics: LlmCallMetrics {
                    provider: "openai".to_string(),
                    model: "text-embedding-3-small".to_string(),
                    endpoint_kind: "embeddings".to_string(),
                    request_chars: request
                        .inputs
                        .iter()
                        .map(|input| input.text.len() as u64)
                        .sum(),
                    response_chars: 0,
                    ..LlmCallMetrics::default()
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
        // SAFETY: test serializes XDG env access with a process-wide mutex.
        unsafe { env::set_var("XDG_DATA_HOME", temp.path()) };

        let path = PaperDb::default_path().expect("default path");

        match previous {
            // SAFETY: test serializes XDG env access with a process-wide mutex.
            Some(value) => unsafe { env::set_var("XDG_DATA_HOME", value) },
            // SAFETY: test serializes XDG env access with a process-wide mutex.
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
        let db = PaperDb::open(dir.path().join("paperdb.duckdb")).expect("open db");
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
        let db = PaperDb::open(dir.path().join("paperdb.duckdb")).expect("open db");
        let paper = sample_paper("embedding ready");
        let client = StubEmbeddingClient::default();
        let config = sample_embedding_config();

        let first = db
            .sync_embeddings(&[paper.clone()], &client, &config)
            .await
            .expect("first sync");
        let second = db
            .sync_embeddings(&[paper.clone()], &client, &config)
            .await
            .expect("second sync");

        assert_eq!(first.embeddings_upserted, 1);
        assert_eq!(second.embeddings_upserted, 0);
        assert_eq!(second.embeddings_skipped, 1);
        assert_eq!(*client.calls.lock().expect("calls lock"), 1);

        let stored = db
            .get_embedding(&paper.file_id, "openai", &config.model)
            .expect("get embedding")
            .expect("stored embedding");
        assert_eq!(stored.dimensions, 2);
    }

    #[tokio::test]
    async fn sync_embeddings_rewrites_stale_rows_when_text_changes() {
        let dir = tempdir().expect("tempdir");
        let db = PaperDb::open(dir.path().join("paperdb.duckdb")).expect("open db");
        let client = StubEmbeddingClient::default();
        let config = sample_embedding_config();
        let original = sample_paper("first text");
        let updated = sample_paper("second text");

        db.sync_embeddings(&[original.clone()], &client, &config)
            .await
            .expect("first sync");
        db.sync_embeddings(&[updated.clone()], &client, &config)
            .await
            .expect("second sync");

        assert_eq!(*client.calls.lock().expect("calls lock"), 2);
        let stored = db
            .get_embedding(&updated.file_id, "openai", &config.model)
            .expect("get embedding")
            .expect("stored embedding");
        assert_eq!(stored.embedding[0], updated.llm_ready_text.len() as f32);
    }
}
