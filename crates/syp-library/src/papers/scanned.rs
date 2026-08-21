//! Reading PDFs that carry no text layer.
//!
//! A scanned paper parses as a PDF but every page is an image, so both text
//! extractors come back empty. The pages are rendered and handed to the
//! configured model, which writes the summary that stands in for the missing
//! text.
//!
//! The summary does not need to be a faithful transcription: everything
//! downstream reduces the text to topical terms anyway, so a title plus an
//! abstract-style paragraph carries the signal the pipeline actually uses.

use std::{path::Path, process::Command};

use syp_ai::llm::{LlmClient, PageImage};

use crate::error::{AppError, Result};

/// Render resolution, the usual floor for reading scanned text reliably.
const RENDER_DPI: u16 = 150;

const SYSTEM_PROMPT: &str = "You read page images of scanned academic papers and describe them in plain text. You never invent details that are not visible on the page.";

const USER_PROMPT: &str = "These are the first page images of a scanned academic PDF, in order. Write, in plain text and with no markdown:\n\
1. the title on its own line\n\
2. the authors, if legible\n\
3. one paragraph in the style of an abstract, covering the subject, method, and contribution\n\
4. a final line listing the key topical terms, separated by commas\n\
If a page is unreadable, describe only what is legible.";

/// Asks the configured model to describe the rendered pages of a scanned PDF.
///
/// # Errors
/// Returns an error when the pages cannot be rendered, the model cannot read
/// images, or the model returns nothing usable.
pub async fn summarize_scanned_pdf(
    client: &dyn LlmClient,
    path: &Path,
    page_cutoff: u8,
) -> Result<String> {
    let images = render_pages(path, page_cutoff)?;
    let response = client
        .chat_with_images(SYSTEM_PROMPT, USER_PROMPT, &images)
        .await
        .map_err(|err| {
            AppError::Pdf(format!(
                "reading scanned {} with the configured model failed: {err}",
                path.display()
            ))
        })?;

    let summary = response.content.trim().to_string();
    if summary.is_empty() {
        return Err(AppError::Pdf(format!(
            "the model returned no description for scanned {}",
            path.display()
        )));
    }
    Ok(summary)
}

/// Renders the first `page_cutoff` pages to PNG.
///
/// Stops at the first page that will not render, which is how a PDF shorter
/// than the cutoff ends: the pages already rendered are returned.
///
/// # Errors
/// Returns an error when not even the first page renders.
pub fn render_pages(path: &Path, page_cutoff: u8) -> Result<Vec<PageImage>> {
    let mut images = Vec::new();
    for page in 1..=page_cutoff.max(1) {
        match render_page(path, page) {
            Ok(image) => images.push(image),
            Err(err) if page == 1 => return Err(err),
            Err(_) => break,
        }
    }
    Ok(images)
}

fn render_page(path: &Path, page: u8) -> Result<PageImage> {
    // With no output prefix pdftoppm writes the image to stdout; passing "-"
    // would create a file literally named "-.png" instead.
    let output = Command::new("pdftoppm")
        .arg("-png")
        .arg("-r")
        .arg(RENDER_DPI.to_string())
        .arg("-f")
        .arg(page.to_string())
        .arg("-l")
        .arg(page.to_string())
        .arg("-singlefile")
        .arg(path)
        .output()
        .map_err(|err| {
            AppError::Pdf(format!(
                "pdftoppm is needed to read scanned PDFs but could not be run: {err}"
            ))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(AppError::Pdf(format!(
            "pdftoppm failed on page {page} of {}{}",
            path.display(),
            if stderr.is_empty() {
                String::new()
            } else {
                format!(": {stderr}")
            }
        )));
    }

    if output.stdout.is_empty() {
        return Err(AppError::Pdf(format!(
            "pdftoppm rendered no image for page {page} of {}",
            path.display()
        )));
    }

    Ok(PageImage::png(output.stdout))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{Arc, Mutex},
    };

    use syp_ai::llm::{LlmCallMetrics, LlmResponse};
    use tempfile::tempdir;

    use super::*;
    use crate::{
        papers::{
            PdfCandidate,
            extract::{ExtractorMode, extract_text_batch},
        },
        terminal::Verbosity,
    };

    /// A one-page PDF with an empty content stream: it parses, and it has no
    /// text layer, which is what a scanned page looks like to the extractors.
    fn write_textless_pdf(path: &Path) {
        let objects: [&[u8]; 4] = [
            b"<< /Type /Catalog /Pages 2 0 R >>",
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << >> >>",
            b"<< /Length 0 >>\nstream\n\nendstream",
        ];

        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            pdf.extend_from_slice(body);
            pdf.extend_from_slice(b"\nendobj\n");
        }

        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );

        fs::write(path, pdf).expect("write pdf");
    }

    fn pdftoppm_available() -> bool {
        Command::new("pdftoppm")
            .arg("-v")
            .output()
            .is_ok_and(|output| output.status.success() || !output.stderr.is_empty())
    }

    #[test]
    fn renders_a_page_without_a_text_layer_to_png() {
        if !pdftoppm_available() {
            return;
        }

        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("scanned.pdf");
        write_textless_pdf(&path);

        let images = render_pages(&path, 1).expect("render pages");

        assert_eq!(images.len(), 1);
        assert_eq!(images[0].mime_type, "image/png");
        assert!(
            images[0].data.starts_with(b"\x89PNG\r\n\x1a\n"),
            "rendered bytes should be a png"
        );
        assert!(!images[0].base64().is_empty());
    }

    /// Stands in for a vision-capable model, recording what it was asked.
    struct StubVisionClient {
        image_count: Arc<Mutex<usize>>,
    }

    #[async_trait::async_trait]
    impl LlmClient for StubVisionClient {
        async fn chat(
            &self,
            _system_prompt: &str,
            _user_prompt: &str,
        ) -> syp_ai::error::Result<LlmResponse> {
            panic!("a scanned pdf must be read with images, not plain chat")
        }

        async fn chat_with_images(
            &self,
            _system_prompt: &str,
            _user_prompt: &str,
            images: &[PageImage],
        ) -> syp_ai::error::Result<LlmResponse> {
            *self.image_count.lock().expect("lock") = images.len();
            Ok(LlmResponse {
                content: "Neural Speech Separation\n\nA scanned paper on speech separation with deep networks.\n\nspeech separation, deep learning".to_string(),
                metrics: LlmCallMetrics::default(),
            })
        }
    }

    #[tokio::test]
    async fn a_pdf_without_a_text_layer_is_read_from_its_page_images() {
        if !pdftoppm_available() {
            return;
        }

        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("scanned.pdf");
        write_textless_pdf(&path);
        let image_count = Arc::new(Mutex::new(0));
        let client: Arc<dyn LlmClient> = Arc::new(StubVisionClient {
            image_count: Arc::clone(&image_count),
        });
        let candidates = vec![PdfCandidate {
            path: path.clone(),
            size_bytes: 0,
        }];

        let (papers, failures) = extract_text_batch(
            &candidates,
            1,
            ExtractorMode::Auto,
            false,
            1,
            Verbosity::new(false, false, true),
            Some(&client),
        )
        .await;

        assert!(
            failures.is_empty(),
            "the scan should be recovered: {failures:?}"
        );
        assert_eq!(papers.len(), 1);
        assert!(papers[0].from_page_images);
        assert!(
            papers[0]
                .extracted_text
                .contains("Neural Speech Separation")
        );
        assert!(
            papers[0].llm_ready_text.contains("speech separation"),
            "the summary should survive preprocessing: {}",
            papers[0].llm_ready_text
        );
        assert_eq!(*image_count.lock().expect("lock"), 1);
    }

    #[tokio::test]
    async fn a_pdf_without_a_text_layer_fails_clearly_when_no_model_is_available() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("scanned.pdf");
        write_textless_pdf(&path);
        let candidates = vec![PdfCandidate {
            path,
            size_bytes: 0,
        }];

        let (papers, failures) = extract_text_batch(
            &candidates,
            1,
            ExtractorMode::Auto,
            false,
            1,
            Verbosity::new(false, false, true),
            None,
        )
        .await;

        assert!(papers.is_empty());
        assert_eq!(failures.len(), 1);
        assert!(
            failures[0].1.contains("no text layer"),
            "unexpected reason: {}",
            failures[0].1
        );
    }

    #[test]
    fn rendering_stops_after_the_last_page_of_a_short_pdf() {
        if !pdftoppm_available() {
            return;
        }

        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("scanned.pdf");
        write_textless_pdf(&path);

        let images = render_pages(&path, 5).expect("render pages");

        assert_eq!(images.len(), 1, "the pdf only has one page");
    }
}
