use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use reqwest::header::USER_AGENT;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use syp_core::{
    config,
    error::{AppError, Result},
};

use crate::{
    CuratedTestSet,
    manifest::{CuratedPaperEntry, save_test_set, validate_test_set},
};

#[derive(Debug, Clone, Default)]
pub struct MaterializeOptions {
    pub cache_dir: Option<PathBuf>,
    pub force_download: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedPaper {
    pub paper_id: String,
    pub arxiv_id: String,
    pub source_url: String,
    pub path: PathBuf,
    pub sha256: String,
    pub byte_size: u64,
    pub downloaded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializeReport {
    pub set_id: String,
    pub cache_dir: PathBuf,
    pub papers: Vec<MaterializedPaper>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheState {
    papers: BTreeMap<String, CachedPaperState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedPaperState {
    arxiv_id: String,
    source_url: String,
    sha256: String,
    byte_size: u64,
    etag: Option<String>,
    verified_at_ms: i64,
}

pub async fn materialize_test_set(
    set: &CuratedTestSet,
    options: &MaterializeOptions,
) -> Result<MaterializeReport> {
    validate_test_set(set)?;

    let cache_root = resolve_cache_root(options)?;
    let set_dir = cache_root.join(&set.id);
    let files_dir = set_dir.join("files");
    fs::create_dir_all(&files_dir)?;

    let client = reqwest::Client::builder()
        .user_agent("sortyourpapers-paperfetch/0.1")
        .build()?;

    let state_path = set_dir.join("state.json");
    let mut cache_state = load_cache_state(&state_path)?;
    let mut cached_manifest = set.clone();
    let mut papers = Vec::with_capacity(set.papers.len());

    for entry in &set.papers {
        let materialized =
            materialize_one(&client, entry, &files_dir, options.force_download).await?;
        cache_state.papers.insert(
            entry.paper_id.clone(),
            CachedPaperState {
                arxiv_id: entry.arxiv_id.clone(),
                source_url: entry.canonical_pdf_url.clone(),
                sha256: materialized.sha256.clone(),
                byte_size: materialized.byte_size,
                etag: None,
                verified_at_ms: now_unix_ms()?,
            },
        );
        if let Some(target) = cached_manifest
            .papers
            .iter_mut()
            .find(|paper| paper.paper_id == entry.paper_id)
        {
            target.sha256 = Some(materialized.sha256.clone());
            target.byte_size = Some(materialized.byte_size);
        }
        papers.push(materialized);
    }

    save_cache_state(&state_path, &cache_state)?;
    save_test_set(set_dir.join("manifest.toml"), &cached_manifest)?;

    Ok(MaterializeReport {
        set_id: set.id.clone(),
        cache_dir: set_dir,
        papers,
    })
}

pub fn export_test_set(
    report: &MaterializeReport,
    target_dir: impl AsRef<Path>,
) -> Result<Vec<PathBuf>> {
    let target_dir = target_dir.as_ref();
    fs::create_dir_all(target_dir)?;

    let mut exported = Vec::with_capacity(report.papers.len());
    for paper in &report.papers {
        let destination = target_dir.join(format!("{}.pdf", paper.paper_id));
        fs::copy(&paper.path, &destination)?;
        exported.push(destination);
    }

    Ok(exported)
}

async fn materialize_one(
    client: &reqwest::Client,
    entry: &CuratedPaperEntry,
    files_dir: &Path,
    force_download: bool,
) -> Result<MaterializedPaper> {
    let path = files_dir.join(format!("{}.pdf", entry.paper_id));

    if !force_download && path.exists() {
        let bytes = fs::read(&path)?;
        let sha256 = sha256_hex(&bytes);
        let byte_size = u64::try_from(bytes.len())
            .map_err(|_| AppError::Execution("pdf exceeded u64 byte size range".to_string()))?;
        if checksum_matches(entry, &sha256) {
            return Ok(MaterializedPaper {
                paper_id: entry.paper_id.clone(),
                arxiv_id: entry.arxiv_id.clone(),
                source_url: entry.canonical_pdf_url.clone(),
                path,
                sha256,
                byte_size,
                downloaded: false,
            });
        }
    }

    let response = client
        .get(&entry.canonical_pdf_url)
        .header(USER_AGENT, "sortyourpapers-paperfetch/0.1")
        .send()
        .await?
        .error_for_status()?;
    let bytes = response.bytes().await?;
    let sha256 = sha256_hex(&bytes);
    let byte_size = u64::try_from(bytes.len())
        .map_err(|_| AppError::Execution("pdf exceeded u64 byte size range".to_string()))?;

    if !checksum_matches(entry, &sha256) {
        return Err(AppError::Validation(format!(
            "downloaded checksum for {} did not match manifest",
            entry.paper_id
        )));
    }

    let tmp_path = files_dir.join(format!("{}.tmp", entry.paper_id));
    fs::write(&tmp_path, &bytes)?;
    fs::rename(&tmp_path, &path)?;

    Ok(MaterializedPaper {
        paper_id: entry.paper_id.clone(),
        arxiv_id: entry.arxiv_id.clone(),
        source_url: entry.canonical_pdf_url.clone(),
        path,
        sha256,
        byte_size,
        downloaded: true,
    })
}

fn checksum_matches(entry: &CuratedPaperEntry, actual: &str) -> bool {
    entry
        .sha256
        .as_deref()
        .map(|expected| expected == actual)
        .unwrap_or(true)
}

fn resolve_cache_root(options: &MaterializeOptions) -> Result<PathBuf> {
    if let Some(path) = &options.cache_dir {
        return Ok(path.clone());
    }

    config::xdg_testset_cache_dir().ok_or_else(|| {
        AppError::Config("could not resolve XDG test-set cache directory".to_string())
    })
}

fn load_cache_state(path: &Path) -> Result<CacheState> {
    if !path.exists() {
        return Ok(CacheState::default());
    }

    let raw = fs::read_to_string(path)?;
    serde_json::from_str(&raw)
        .map_err(|err| AppError::Execution(format!("invalid cache state: {err}")))
}

fn save_cache_state(path: &Path, state: &CacheState) -> Result<()> {
    let raw = serde_json::to_string_pretty(state)
        .map_err(|err| AppError::Execution(format!("failed to serialize cache state: {err}")))?;
    fs::write(path, raw)?;
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn now_unix_ms() -> Result<i64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| AppError::Execution(format!("system clock error: {err}")))?;
    i64::try_from(elapsed.as_millis())
        .map_err(|_| AppError::Execution("timestamp exceeded i64 range".to_string()))
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        thread,
    };

    use tempfile::tempdir;

    use super::{MaterializeOptions, export_test_set, materialize_test_set};
    use crate::{CuratedPaperEntry, CuratedTestSet, SamplingBucket, SamplingPolicy};

    #[tokio::test]
    async fn materialize_reuses_cached_files() {
        let hit_count = Arc::new(AtomicUsize::new(0));
        let server = spawn_server(
            vec![(
                "/paper.pdf".to_string(),
                b"%PDF-demo".to_vec(),
                Some("\"etag-1\"".to_string()),
            )],
            hit_count.clone(),
        );
        let dir = tempdir().expect("tempdir");
        let set = sample_set(format!("{}/paper.pdf", server.base_url()));

        let first = materialize_test_set(
            &set,
            &MaterializeOptions {
                cache_dir: Some(dir.path().to_path_buf()),
                force_download: false,
            },
        )
        .await
        .expect("first materialize");
        let second = materialize_test_set(
            &set,
            &MaterializeOptions {
                cache_dir: Some(dir.path().to_path_buf()),
                force_download: false,
            },
        )
        .await
        .expect("second materialize");

        assert!(first.papers[0].downloaded);
        assert!(!second.papers[0].downloaded);
        assert_eq!(hit_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn export_copies_materialized_files() {
        let hit_count = Arc::new(AtomicUsize::new(0));
        let server = spawn_server(
            vec![(
                "/paper.pdf".to_string(),
                b"%PDF-demo".to_vec(),
                Some("\"etag-2\"".to_string()),
            )],
            hit_count,
        );
        let dir = tempdir().expect("tempdir");
        let export_dir = tempdir().expect("export tempdir");
        let set = sample_set(format!("{}/paper.pdf", server.base_url()));

        let report = materialize_test_set(
            &set,
            &MaterializeOptions {
                cache_dir: Some(dir.path().to_path_buf()),
                force_download: false,
            },
        )
        .await
        .expect("materialize");
        let exported = export_test_set(&report, export_dir.path()).expect("export");

        assert_eq!(exported.len(), 1);
        assert!(exported[0].exists());
    }

    fn sample_set(url: String) -> CuratedTestSet {
        CuratedTestSet {
            id: "demo-set".to_string(),
            description: "Demo".to_string(),
            source_dataset: "OpenMOSS-Team/SciJudgeBench".to_string(),
            selection_policy: SamplingPolicy::default(),
            generated_at_ms: 1,
            papers: vec![CuratedPaperEntry {
                paper_id: "arxiv-1234.5678".to_string(),
                arxiv_id: "1234.5678".to_string(),
                canonical_pdf_url: url,
                title: "Title".to_string(),
                category: "CS".to_string(),
                subcategory: "cs.AI".to_string(),
                citations: 10,
                date: Some("2024-01-01".to_string()),
                abstract_excerpt: "Excerpt".to_string(),
                selection_bucket: SamplingBucket::Top,
                sha256: None,
                byte_size: None,
            }],
        }
    }

    struct TestServer {
        addr: String,
    }

    impl TestServer {
        fn base_url(&self) -> String {
            format!("http://{}", self.addr)
        }
    }

    fn spawn_server(
        responses: Vec<(String, Vec<u8>, Option<String>)>,
        hit_count: Arc<AtomicUsize>,
    ) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        thread::spawn(move || {
            for (expected_path, body, etag) in responses {
                let (mut stream, _) = listener.accept().expect("accept");
                let mut request = [0_u8; 4096];
                let size = stream.read(&mut request).expect("read request");
                let request = String::from_utf8_lossy(&request[..size]);
                let line = request.lines().next().expect("request line");
                let path = line.split_whitespace().nth(1).expect("request path");
                assert_eq!(path, expected_path);
                hit_count.fetch_add(1, Ordering::SeqCst);

                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/pdf\r\n{}\r\n",
                    body.len(),
                    etag.as_ref()
                        .map(|value| format!("ETag: {value}\r\n"))
                        .unwrap_or_default()
                );
                stream.write_all(headers.as_bytes()).expect("write headers");
                stream.write_all(&body).expect("write body");
            }
        });

        TestServer {
            addr: addr.to_string(),
        }
    }
}
