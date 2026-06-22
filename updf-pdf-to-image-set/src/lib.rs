// updf-pdf-to-image-set
//
// Library for rendering a PDF (or a directory of PDFs) to one image per page using
// Ghostscript. The command-line front-end lives in the `updf-cli` crate.
//
// Public API:
//   - `convert::convert_path` / `convert::convert_pdf` — the orchestration entry points
//   - `convert::ConvertOptions` / `convert::ConvertError` / `convert::PdfOutcome`
//   - `ghostscript::{OutputDevice, RenderJob, render, GhostscriptError}` — the gs layer

pub mod ghostscript;
pub mod convert;
