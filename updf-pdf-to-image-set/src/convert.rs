// Path/batch orchestration on top of the Ghostscript renderer.
//
// Takes a single PDF or a directory of PDFs and renders each into its own output
// directory (named after the PDF), one image per page (`page-1.png`, `page-2.png`, …),
// via the `gs`-backed `render()` in `ghostscript.rs`.

use std::path::{Path, PathBuf};

use crate::ghostscript::{GhostscriptError, OutputDevice, RenderJob, render};

/// Rendering knobs shared across a conversion run.
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    pub device: OutputDevice,
    pub dpi: u32,
    /// First page to render (1-based). `None` = from the start.
    pub first_page: Option<u32>,
    /// Last page to render (inclusive). `None` = to the end.
    pub last_page: Option<u32>,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            device: OutputDevice::Png16m,
            dpi: 150,
            first_page: None,
            last_page: None,
        }
    }
}

/// Errors raised while preparing/driving a conversion (the `gs` failure itself is
/// wrapped in `Ghostscript`).
#[derive(Debug)]
pub enum ConvertError {
    /// Input path does not exist or is neither a file nor a directory.
    NotFound(PathBuf),
    /// A directory input contained no `*.pdf` files.
    NoPdfs(PathBuf),
    /// Could not create the output directory / read the input directory.
    Io(std::io::Error),
    /// Ghostscript rejected the job.
    Ghostscript(GhostscriptError),
}

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(p) => write!(f, "input path not found: {}", p.display()),
            Self::NoPdfs(p) => write!(f, "no .pdf files in directory: {}", p.display()),
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Ghostscript(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ConvertError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Ghostscript(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ConvertError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<GhostscriptError> for ConvertError {
    fn from(e: GhostscriptError) -> Self {
        Self::Ghostscript(e)
    }
}

/// Outcome for one PDF in a (possibly batch) run.
#[derive(Debug)]
pub struct PdfOutcome {
    pub pdf: PathBuf,
    /// On success, the directory the page images were written to.
    pub result: Result<PathBuf, ConvertError>,
}

/// Render a single PDF into `<output_base>/<pdf_stem>/page-N.<ext>`, creating the
/// directory if needed. Returns the directory that was populated.
pub fn convert_pdf(
    pdf: &Path,
    output_base: &Path,
    opts: &ConvertOptions,
) -> Result<PathBuf, ConvertError> {
    if !pdf.is_file() {
        return Err(ConvertError::NotFound(pdf.to_path_buf()));
    }
    let stem = pdf
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| ConvertError::NotFound(pdf.to_path_buf()))?;

    let out_dir = output_base.join(stem);
    std::fs::create_dir_all(&out_dir)?;

    // gs expands `%d` to the (1-based) page number.
    let pattern = out_dir.join(format!("page-%d.{}", opts.device.file_extension()));
    let mut job = RenderJob::new(pdf, pattern)
        .device(opts.device)
        .dpi(opts.dpi);
    job = match (opts.first_page, opts.last_page) {
        (Some(first), Some(last)) => job.pages(first, last),
        (Some(first), None) => job.first_page(first),
        (None, Some(last)) => job.last_page(last),
        (None, None) => job,
    };

    render(&job)?;
    Ok(out_dir)
}

/// Collect `*.pdf` files directly inside `dir` (non-recursive), sorted by path.
fn collect_pdfs(dir: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut pdfs = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let is_pdf = path.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
        if is_pdf {
            pdfs.push(path);
        }
    }
    pdfs.sort();
    Ok(pdfs)
}

/// Convert a single PDF file, or every `*.pdf` in a directory (non-recursive).
///
/// In directory mode the batch continues past individual failures; inspect each
/// [`PdfOutcome::result`]. Returns `Err` only for problems with the input itself
/// (missing path, empty directory).
pub fn convert_path(
    input: &Path,
    output_base: &Path,
    opts: &ConvertOptions,
) -> Result<Vec<PdfOutcome>, ConvertError> {
    if input.is_file() {
        let result = convert_pdf(input, output_base, opts);
        return Ok(vec![PdfOutcome {
            pdf: input.to_path_buf(),
            result,
        }]);
    }

    if input.is_dir() {
        let pdfs = collect_pdfs(input)?;
        if pdfs.is_empty() {
            return Err(ConvertError::NoPdfs(input.to_path_buf()));
        }
        let outcomes = pdfs
            .into_iter()
            .map(|pdf| {
                let result = convert_pdf(&pdf, output_base, opts);
                PdfOutcome { pdf, result }
            })
            .collect();
        return Ok(outcomes);
    }

    Err(ConvertError::NotFound(input.to_path_buf()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_input_is_not_found() {
        let err = convert_pdf(
            Path::new("/no/such/file.pdf"),
            Path::new("/tmp"),
            &ConvertOptions::default(),
        )
        .unwrap_err();
        assert!(matches!(err, ConvertError::NotFound(_)));
    }

    #[test]
    fn convert_path_rejects_missing_input() {
        let err = convert_path(
            Path::new("/no/such/path"),
            Path::new("/tmp"),
            &ConvertOptions::default(),
        )
        .unwrap_err();
        assert!(matches!(err, ConvertError::NotFound(_)));
    }

    #[test]
    fn empty_dir_reports_no_pdfs() {
        let dir = std::env::temp_dir().join(format!("updf-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let err = convert_path(&dir, Path::new("/tmp"), &ConvertOptions::default()).unwrap_err();
        assert!(matches!(err, ConvertError::NoPdfs(_)));
        let _ = std::fs::remove_dir(&dir);
    }

    fn gs_available() -> bool {
        std::process::Command::new("gs")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Build a minimal valid PDF with `pages` blank pages (each draws a filled rectangle).
    /// Ghostscript reconstructs the cross-reference table, so we omit xref/trailer. This
    /// keeps the tests self-contained — no reliance on sample files in the repo.
    fn make_pdf_bytes(pages: usize) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"%PDF-1.4\n");
        let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 3 + i)).collect();
        out.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        out.extend_from_slice(
            format!(
                "2 0 obj\n<< /Type /Pages /Kids [{}] /Count {} >>\nendobj\n",
                kids.join(" "),
                pages
            )
            .as_bytes(),
        );
        for i in 0..pages {
            out.extend_from_slice(
                format!(
                    "{} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents {} 0 R >>\nendobj\n",
                    3 + i,
                    3 + pages + i
                )
                .as_bytes(),
            );
        }
        for i in 0..pages {
            let stream = b"0 0 100 100 re f";
            out.extend_from_slice(
                format!("{} 0 obj\n<< /Length {} >>\nstream\n", 3 + pages + i, stream.len())
                    .as_bytes(),
            );
            out.extend_from_slice(stream);
            out.extend_from_slice(b"\nendstream\nendobj\n");
        }
        out.extend_from_slice(b"%%EOF\n");
        out
    }

    /// Write a fixture PDF with `pages` pages and return its path.
    fn write_fixture_pdf(path: &Path, pages: usize) -> PathBuf {
        std::fs::write(path, make_pdf_bytes(pages)).unwrap();
        path.to_path_buf()
    }

    /// A unique, freshly-empty scratch directory for a test.
    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "updf-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn count_files(dir: &Path) -> usize {
        std::fs::read_dir(dir)
            .map(|rd| rd.filter_map(Result::ok).filter(|e| e.path().is_file()).count())
            .unwrap_or(0)
    }

    #[test]
    fn renders_single_page() {
        if !gs_available() {
            eprintln!("SKIP: gs not installed");
            return;
        }
        let base = scratch("render");
        let pdf = write_fixture_pdf(&base.join("doc.pdf"), 3);
        let out_base = base.join("out");

        let opts = ConvertOptions {
            first_page: Some(1),
            last_page: Some(1),
            ..Default::default()
        };
        let out_dir = convert_pdf(&pdf, &out_base, &opts).expect("render should succeed");

        // Output dir is named after the PDF stem and contains a non-empty page-1.png.
        assert_eq!(out_dir.file_name(), pdf.file_stem());
        let meta = std::fs::metadata(out_dir.join("page-1.png")).expect("page-1.png");
        assert!(meta.len() > 0, "page-1.png should be non-empty");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn page_range_produces_one_image_per_page() {
        if !gs_available() {
            eprintln!("SKIP: gs not installed");
            return;
        }
        let base = scratch("range");
        let pdf = write_fixture_pdf(&base.join("doc.pdf"), 5);
        let out_base = base.join("out");

        let opts = ConvertOptions {
            first_page: Some(2),
            last_page: Some(4),
            ..Default::default()
        };
        let out_dir = convert_pdf(&pdf, &out_base, &opts).expect("render should succeed");

        // Three pages requested -> exactly three images, numbered from 1.
        assert_eq!(count_files(&out_dir), 3, "expected one image per requested page");
        for n in 1..=3 {
            assert!(out_dir.join(format!("page-{n}.png")).is_file());
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn device_choice_controls_file_extension() {
        if !gs_available() {
            eprintln!("SKIP: gs not installed");
            return;
        }
        let base = scratch("jpeg");
        let pdf = write_fixture_pdf(&base.join("doc.pdf"), 2);
        let out_base = base.join("out");

        let opts = ConvertOptions {
            device: OutputDevice::Jpeg,
            first_page: Some(1),
            last_page: Some(1),
            ..Default::default()
        };
        let out_dir = convert_pdf(&pdf, &out_base, &opts).expect("render should succeed");
        assert!(out_dir.join("page-1.jpg").is_file(), "jpeg device should emit .jpg");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn directory_batch_gives_each_pdf_its_own_folder() {
        if !gs_available() {
            eprintln!("SKIP: gs not installed");
            return;
        }
        let base = scratch("batch");
        let in_dir = base.join("in");
        let out_base = base.join("out");
        std::fs::create_dir_all(&in_dir).unwrap();
        write_fixture_pdf(&in_dir.join("alpha.pdf"), 2);
        write_fixture_pdf(&in_dir.join("beta.pdf"), 2);

        let opts = ConvertOptions {
            first_page: Some(1),
            last_page: Some(1),
            ..Default::default()
        };
        let outcomes = convert_path(&in_dir, &out_base, &opts).expect("batch should run");

        assert_eq!(outcomes.len(), 2, "one outcome per pdf");
        for outcome in &outcomes {
            assert!(outcome.result.is_ok(), "{:?} failed: {:?}", outcome.pdf, outcome.result);
        }
        assert!(out_base.join("alpha/page-1.png").is_file());
        assert!(out_base.join("beta/page-1.png").is_file());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn collect_pdfs_filters_and_sorts() {
        let dir = std::env::temp_dir().join(format!("updf-collect-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.pdf"), b"%PDF").unwrap();
        std::fs::write(dir.join("a.PDF"), b"%PDF").unwrap();
        std::fs::write(dir.join("note.txt"), b"x").unwrap();

        let pdfs = collect_pdfs(&dir).unwrap();
        let names: Vec<_> = pdfs
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.PDF", "b.pdf"]);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
