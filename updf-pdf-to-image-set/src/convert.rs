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

    /// A real PDF to render, if one is present in the repo. Tests skip when absent.
    fn sample_pdf() -> Option<PathBuf> {
        [
            concat!(env!("CARGO_MANIFEST_DIR"), "/repaired.pdf"),
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../AlanWatts-EssentialAlanWatts(AlanWatts)(Z-Library) 2.pdf"
            ),
        ]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
    }

    #[test]
    fn renders_first_page_of_real_pdf() {
        if !gs_available() {
            eprintln!("SKIP: gs not installed");
            return;
        }
        let Some(pdf) = sample_pdf() else {
            eprintln!("SKIP: no sample pdf in repo");
            return;
        };

        let out_base =
            std::env::temp_dir().join(format!("updf-render-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out_base);

        // Render only page 1 to keep the test fast.
        let opts = ConvertOptions {
            first_page: Some(1),
            last_page: Some(1),
            ..Default::default()
        };
        let out_dir = convert_pdf(&pdf, &out_base, &opts).expect("render should succeed");

        // Output dir is named after the PDF stem and contains page-1.png.
        assert_eq!(out_dir.file_name(), pdf.file_stem());
        let page1 = out_dir.join("page-1.png");
        let meta = std::fs::metadata(&page1).expect("page-1.png should exist");
        assert!(meta.len() > 0, "page-1.png should be non-empty");

        let _ = std::fs::remove_dir_all(&out_base);
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
