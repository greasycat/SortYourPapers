use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use walkdir::WalkDir;

use crate::{error::Result, papers::PdfCandidate};

pub fn discover_pdf_candidates(root: &Path, recursive: bool) -> Result<Vec<PdfCandidate>> {
    let mut out = Vec::new();
    let max_depth = if recursive { usize::MAX } else { 1 };

    for entry in WalkDir::new(root).follow_links(false).max_depth(max_depth) {
        let entry = entry?;
        let path = entry.path();

        if !entry.file_type().is_file() || !is_pdf(path) {
            continue;
        }

        let metadata = fs::metadata(path)?;
        out.push(PdfCandidate {
            path: path.to_path_buf(),
            size_bytes: metadata.len(),
        });
    }

    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

pub fn dedupe_candidates(candidates: Vec<PdfCandidate>) -> Vec<PdfCandidate> {
    let mut seen = HashSet::<PathBuf>::new();
    let mut deduped = Vec::new();

    for candidate in candidates {
        let key = candidate
            .path
            .canonicalize()
            .unwrap_or_else(|_| candidate.path.clone());
        if seen.insert(key) {
            deduped.push(candidate);
        }
    }

    deduped
}

pub fn split_by_size(
    candidates: Vec<PdfCandidate>,
    max_file_size_mb: u64,
) -> (Vec<PdfCandidate>, Vec<PdfCandidate>) {
    let max_size_bytes = max_file_size_mb.saturating_mul(1024 * 1024);

    let mut accepted = Vec::new();
    let mut skipped = Vec::new();

    for candidate in candidates {
        if candidate.size_bytes <= max_size_bytes {
            accepted.push(candidate);
        } else {
            skipped.push(candidate);
        }
    }

    (accepted, skipped)
}

fn is_pdf(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{discover_pdf_candidates, split_by_size};

    #[test]
    fn non_recursive_skips_nested_files() {
        let dir = tempdir().expect("tempdir");
        let root_pdf = dir.path().join("a.pdf");
        let nested_dir = dir.path().join("nested");
        let nested_pdf = nested_dir.join("b.pdf");

        fs::write(&root_pdf, b"root").expect("write root");
        fs::create_dir_all(&nested_dir).expect("mkdir");
        fs::write(&nested_pdf, b"nested").expect("write nested");

        let non_recursive =
            discover_pdf_candidates(dir.path(), false).expect("discover non-recursive");
        let recursive = discover_pdf_candidates(dir.path(), true).expect("discover recursive");

        assert_eq!(non_recursive.len(), 1);
        assert_eq!(recursive.len(), 2);
    }

    #[test]
    fn max_size_includes_exact_limit() {
        let accepted = vec![
            crate::papers::PdfCandidate {
                path: "a.pdf".into(),
                size_bytes: 8 * 1024 * 1024,
            },
            crate::papers::PdfCandidate {
                path: "b.pdf".into(),
                size_bytes: 8 * 1024 * 1024 + 1,
            },
        ];

        let (ok, skipped) = split_by_size(accepted, 8);
        assert_eq!(ok.len(), 1);
        assert_eq!(skipped.len(), 1);
    }
}
