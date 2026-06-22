// Orchestration glue that wires the stage crates together.
//
// The OCR stage emits one page-ordered markdown document with a `<!-- page N -->`
// marker before each page. The correction stage splits the document back on those
// markers and corrects one page at a time — the agent caps its output per call, so a
// whole book can't be corrected in a single request — then rejoins, preserving the
// markers for traceability.
//
// The functions here are deliberately thin and free of clap/CLI types so they can be
// unit-tested without spawning the OCR helper or calling the Claude API. `main` builds
// the `OcrJob` / `CorrectionApi` and feeds them in.

use agent_text_cleanup::agent::FormatTarget;
use agent_text_cleanup::api::CorrectionApi;
use agent_text_cleanup::normalize::regex_repair;
use apple_vision_image_text_extractor::types::VisionResults;
use apple_vision_image_text_extractor::vision::{OcrJob, VisionError, extract};

/// Parse the page number out of a `page-N.<ext>` image file name.
///
/// Numeric (not lexical) so `page-2` sorts before `page-10`.
pub fn page_index(file_name: &str) -> Option<u32> {
    let stem = file_name.rsplit_once('.').map_or(file_name, |(s, _)| s);
    stem.strip_prefix("page-")?.parse().ok()
}

/// True if a line is one of our `<!-- page ... -->` page markers.
fn is_page_marker(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("<!-- page") && t.ends_with("-->")
}

/// Pull the page number out of a `<!-- page N -->` (or `<!-- page N: ... -->`) marker.
fn marker_page_no(marker: &str) -> Option<usize> {
    let after = marker.trim().strip_prefix("<!-- page")?.trim_start();
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Assemble per-image OCR results into one page-ordered markdown document.
///
/// Results are sorted numerically by their `page-N` name. Each page is preceded by a
/// `<!-- page N -->` marker; pages that failed OCR become a `<!-- page N: OCR ERROR -->`
/// marker so positions are preserved and the failure is visible.
pub fn assemble_markdown(results: &[VisionResults]) -> String {
    let mut sorted: Vec<&VisionResults> = results.iter().collect();
    sorted.sort_by_key(|r| page_index(&r.image_file_name).unwrap_or(u32::MAX));

    let mut blocks = Vec::with_capacity(sorted.len());
    for (i, r) in sorted.iter().enumerate() {
        let n = page_index(&r.image_file_name).unwrap_or((i + 1) as u32);
        match &r.error {
            Some(e) => {
                let msg = e.replace('\n', " ");
                blocks.push(format!("<!-- page {n}: OCR ERROR: {msg} -->"));
            }
            None => blocks.push(format!("<!-- page {n} -->\n\n{}", r.text_blob.trim())),
        }
    }

    let mut out = blocks.join("\n\n");
    out.push('\n');
    out
}

/// One page of a split markdown document: its marker line (if any) and its body text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// The `<!-- page N -->` marker line, verbatim, if this page had one.
    pub marker: Option<String>,
    /// The page text between this marker and the next, trimmed.
    pub body: String,
}

/// Split a page-delimited markdown document back into per-page chunks.
///
/// Inverse of [`assemble_markdown`] for the body text. Any content before the first
/// marker becomes a leading `Page` with `marker == None`.
pub fn split_pages(md: &str) -> Vec<Page> {
    let mut pages = Vec::new();
    let mut marker: Option<String> = None;
    let mut body = String::new();
    let mut started = false;

    for line in md.lines() {
        if is_page_marker(line) {
            if started {
                pages.push(Page { marker: marker.take(), body: body.trim().to_string() });
                body.clear();
            }
            marker = Some(line.trim().to_string());
            started = true;
        } else {
            started = true;
            body.push_str(line);
            body.push('\n');
        }
    }
    if started {
        pages.push(Page { marker, body: body.trim().to_string() });
    }
    pages
}

/// A page that could not be corrected by the agent (kept its pre-correction text).
#[derive(Debug, Clone)]
pub struct PageFailure {
    pub page: usize,
    pub error: String,
}

/// Result of correcting a whole document: the rejoined markdown plus any per-page
/// failures (those pages fall back to their normalized OCR text).
#[derive(Debug, Clone)]
pub struct CorrectionOutcome {
    pub markdown: String,
    pub failures: Vec<PageFailure>,
}

/// Run OCR over a prepared job and assemble the page-ordered markdown.
pub fn run_ocr(job: &OcrJob) -> Result<String, VisionError> {
    let results = extract(job)?;
    Ok(assemble_markdown(&results))
}

/// Correct a page-delimited markdown document one page at a time.
///
/// Each page is optionally run through the offline [`regex_repair`] pass and then the
/// agent. A page that the agent rejects keeps its normalized text and is recorded in
/// [`CorrectionOutcome::failures`] so a single failure never aborts the document.
pub async fn correct_document(
    api: &CorrectionApi,
    md: &str,
    target: Option<&FormatTarget>,
    normalize: bool,
) -> CorrectionOutcome {
    let pages = split_pages(md);
    let mut failures = Vec::new();
    let mut blocks = Vec::with_capacity(pages.len());

    for (idx, page) in pages.iter().enumerate() {
        let page_no = page
            .marker
            .as_deref()
            .and_then(marker_page_no)
            .unwrap_or(idx + 1);

        let normalized = if normalize {
            regex_repair(&page.body)
        } else {
            page.body.clone()
        };

        let corrected = if normalized.trim().is_empty() {
            normalized
        } else {
            match api.correct_markdown(&normalized, target).await {
                Ok(c) => c,
                Err(e) => {
                    failures.push(PageFailure { page: page_no, error: e.to_string() });
                    normalized
                }
            }
        };

        match &page.marker {
            Some(m) => blocks.push(format!("{m}\n\n{}", corrected.trim())),
            None => blocks.push(corrected.trim().to_string()),
        }
    }

    let mut markdown = blocks.join("\n\n");
    markdown.push('\n');
    CorrectionOutcome { markdown, failures }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_result(name: &str, text: &str) -> VisionResults {
        VisionResults {
            image_file_name: name.to_string(),
            image_path: format!("/tmp/{name}"),
            text_blob: text.to_string(),
            pixel_width: 100,
            pixel_height: 100,
            recognized_languages: vec!["en-US".to_string()],
            lines: Vec::new(),
            error: None,
        }
    }

    #[test]
    fn page_index_parses_numeric_suffix() {
        assert_eq!(page_index("page-7.png"), Some(7));
        assert_eq!(page_index("page-12.jpg"), Some(12));
        assert_eq!(page_index("page-03.tiff"), Some(3));
        assert_eq!(page_index("cover.png"), None);
        assert_eq!(page_index("page-x.png"), None);
    }

    #[test]
    fn assemble_orders_pages_numerically_not_lexically() {
        // Deliberately out of order, with page-10 to catch lexical sorting.
        let results = vec![
            ok_result("page-10.png", "ten"),
            ok_result("page-2.png", "two"),
            ok_result("page-1.png", "one"),
        ];
        let md = assemble_markdown(&results);
        let one = md.find("one").unwrap();
        let two = md.find("two").unwrap();
        let ten = md.find("ten").unwrap();
        assert!(one < two && two < ten, "pages should be 1, 2, 10 in order:\n{md}");
        assert!(md.contains("<!-- page 1 -->"));
        assert!(md.contains("<!-- page 10 -->"));
    }

    #[test]
    fn assemble_marks_errored_pages() {
        let mut bad = ok_result("page-2.png", "");
        bad.error = Some("could not load image".to_string());
        let results = vec![ok_result("page-1.png", "hello"), bad];
        let md = assemble_markdown(&results);
        assert!(md.contains("<!-- page 2: OCR ERROR: could not load image -->"), "{md}");
    }

    #[test]
    fn split_round_trips_assembled_bodies() {
        let results = vec![
            ok_result("page-1.png", "hello\nworld"),
            ok_result("page-2.png", "foo bar"),
        ];
        let md = assemble_markdown(&results);
        let pages = split_pages(&md);
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].marker.as_deref(), Some("<!-- page 1 -->"));
        assert_eq!(pages[0].body, "hello\nworld");
        assert_eq!(pages[1].marker.as_deref(), Some("<!-- page 2 -->"));
        assert_eq!(pages[1].body, "foo bar");
    }

    #[test]
    fn split_keeps_leading_content_without_a_marker() {
        let pages = split_pages("preamble text\n\n<!-- page 1 -->\n\nbody");
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].marker, None);
        assert_eq!(pages[0].body, "preamble text");
        assert_eq!(pages[1].marker.as_deref(), Some("<!-- page 1 -->"));
        assert_eq!(pages[1].body, "body");
    }

    #[test]
    fn marker_page_no_reads_plain_and_error_markers() {
        assert_eq!(marker_page_no("<!-- page 12 -->"), Some(12));
        assert_eq!(marker_page_no("<!-- page 3: OCR ERROR: boom -->"), Some(3));
        assert_eq!(marker_page_no("<!-- not a page -->"), None);
    }
}
