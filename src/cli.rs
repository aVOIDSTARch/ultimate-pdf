// Command-line surface for the Ultimate PDF toolkit.
//
// Parsing is defined here (separate from `main`) so it can be unit-tested with
// `Cli::try_parse_from`. Each subcommand maps onto a library call.

use std::path::PathBuf;

use apple_vision_image_text_extractor::vision::RecognitionLevel;
use clap::{Args, Parser, Subcommand};
use updf_pdf_to_image_set::ghostscript::OutputDevice;

/// Defaults resolve relative to this crate so they work from any working directory.
/// `books/` lives directly under the workspace root, which is this crate's manifest dir.
pub const DEFAULT_INPUT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/books/unprocessed");
pub const DEFAULT_OUTPUT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/books/processed");

#[derive(Debug, Parser)]
#[command(
    name = "updf",
    version,
    about = "Ultimate PDF toolkit: PDF -> page images -> OCR markdown -> corrected text",
    long_about = "Ultimate PDF toolkit.\n\n\
        Turns scanned/born-digital PDFs into clean markdown in stages you can run \
        end-to-end or one at a time:\n  \
        pdf -> page images (Ghostscript) -> OCR markdown (Apple Vision) -> \
        corrected markdown (Claude cleanup).\n\n\
        `serve` exposes the same stages over HTTP; `health` polls a running server.\n\n\
        Run `updf <command> --help` for the options of any stage.",
    propagate_version = true,
    after_help = "EXAMPLES:\n  \
        # One-shot: render, OCR, and (optionally) correct a single PDF\n  \
        updf pipeline book.pdf ./out --correct\n\n  \
        # Batch every PDF in the default unprocessed dir, stopping at raw OCR\n  \
        updf pipeline\n\n  \
        # Run the stages by hand\n  \
        updf pdf-to-images book.pdf ./out\n  \
        updf ocr ./out/book ./out/book/book.ocr.md\n  \
        updf correct ./out/book/book.ocr.md ./out/book/book.md\n"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Render a PDF (or every PDF in a directory) to one image per page via Ghostscript.
    ///
    /// Each PDF is written to <OUTPUT>/<pdf-stem>/page-N.<ext>.
    #[command(name = "pdf-to-images")]
    PdfToImages(PdfToImagesArgs),

    /// OCR a directory of page images into one page-ordered markdown file (Apple Vision).
    ///
    /// Images are ordered numerically by their `page-N` name and concatenated, each page
    /// preceded by a `<!-- page N -->` marker so downstream stages can split on it.
    Ocr(OcrArgs),

    /// Correct an OCR markdown file with the agent cleanup crate (offline regex + Claude).
    ///
    /// The input is split on `<!-- page N -->` markers and corrected one page at a time
    /// (the agent caps output per call), then rejoined. Requires ANTHROPIC_API_KEY.
    Correct(CorrectArgs),

    /// Full pipeline: PDF(s) -> page images -> OCR markdown -> (optional) corrected markdown.
    ///
    /// For each PDF, writes <OUTPUT>/<stem>/page-N.<ext>, <OUTPUT>/<stem>/<stem>.ocr.md, and
    /// (with --correct) <OUTPUT>/<stem>/<stem>.md.
    Pipeline(PipelineArgs),

    /// Serve the HTTP API: the same stages over POST endpoints, plus /health (in-process).
    ///
    /// Each endpoint spawns this same `updf` binary as a subprocess, so the CLI stays the
    /// single source of truth. Runs until stopped with Ctrl-C.
    Serve(ServeArgs),

    /// Poll a running server's /health endpoint over HTTP and print the report.
    Health(HealthArgs),
}

#[derive(Debug, Args)]
pub struct ServeArgs {
    /// Address the API server should bind to.
    #[arg(long, default_value = "127.0.0.1:8787")]
    pub addr: String,
}

#[derive(Debug, Args)]
pub struct HealthArgs {
    /// Base URL of a running server (the /health path is appended).
    #[arg(long, default_value = "http://127.0.0.1:8787")]
    pub url: String,

    /// Request timeout in seconds.
    #[arg(long, default_value_t = 5)]
    pub timeout: u64,
}

#[derive(Debug, Args)]
pub struct PdfToImagesArgs {
    /// A .pdf file, or a directory containing .pdf files.
    #[arg(default_value = DEFAULT_INPUT)]
    pub input: PathBuf,

    /// Base directory for the per-PDF image folders.
    #[arg(default_value = DEFAULT_OUTPUT)]
    pub output: PathBuf,

    /// Render resolution in DPI.
    #[arg(long, default_value_t = 150, value_parser = clap::value_parser!(u32).range(1..))]
    pub dpi: u32,

    /// Output device: png16m, pnggray, pngmono, jpeg, or tiff24nc.
    #[arg(long, default_value = "png16m", value_parser = parse_device)]
    pub device: OutputDevice,

    /// First page to render (1-based). Defaults to the first page.
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pub first: Option<u32>,

    /// Last page to render (inclusive). Defaults to the last page.
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pub last: Option<u32>,
}

/// Shared OCR knobs, flattened into the `ocr` and `pipeline` commands.
#[derive(Debug, Args)]
pub struct OcrOptions {
    /// Recognition accuracy/speed trade-off.
    #[arg(long, default_value = "accurate", value_parser = parse_level)]
    pub level: RecognitionLevel,

    /// Comma-separated BCP-47 language tags to recognize (e.g. en-US,fr-FR).
    #[arg(long, value_delimiter = ',', default_value = "en-US")]
    pub languages: Vec<String>,

    /// Drop recognized lines below this confidence (0.0..=1.0).
    #[arg(long, default_value_t = 0.0, value_parser = parse_confidence)]
    pub min_confidence: f32,

    /// Disable Apple Vision's built-in language correction.
    #[arg(long)]
    pub no_language_correction: bool,
}

/// Shared correction knobs, flattened into the `correct` and `pipeline` commands.
#[derive(Debug, Args)]
pub struct CorrectOptions {
    /// Claude model id to use for correction.
    #[arg(long)]
    pub model: Option<String>,

    /// Optional FormatTarget JSON describing the desired output shape.
    #[arg(long)]
    pub target: Option<PathBuf>,

    /// Skip the offline regex normalize pass before the agent pass.
    #[arg(long)]
    pub no_normalize: bool,
}

#[derive(Debug, Args)]
pub struct OcrArgs {
    /// Directory of page images (e.g. the per-PDF folder from `pdf-to-images`).
    pub images_dir: PathBuf,

    /// Output markdown file. Defaults to <IMAGES_DIR>/<dir-name>.ocr.md.
    pub output: Option<PathBuf>,

    #[command(flatten)]
    pub ocr: OcrOptions,
}

#[derive(Debug, Args)]
pub struct CorrectArgs {
    /// Input markdown file (typically a *.ocr.md from the `ocr` stage).
    pub input: PathBuf,

    /// Output markdown file. Defaults to a sibling <stem>.md (or <stem>.corrected.md).
    pub output: Option<PathBuf>,

    #[command(flatten)]
    pub correct: CorrectOptions,
}

#[derive(Debug, Args)]
pub struct PipelineArgs {
    /// A .pdf file, or a directory containing .pdf files.
    #[arg(default_value = DEFAULT_INPUT)]
    pub input: PathBuf,

    /// Base directory for the per-PDF output folders.
    #[arg(default_value = DEFAULT_OUTPUT)]
    pub output: PathBuf,

    /// Render resolution in DPI.
    #[arg(long, default_value_t = 150, value_parser = clap::value_parser!(u32).range(1..))]
    pub dpi: u32,

    /// Output device: png16m, pnggray, pngmono, jpeg, or tiff24nc.
    #[arg(long, default_value = "png16m", value_parser = parse_device)]
    pub device: OutputDevice,

    /// First page to render (1-based). Defaults to the first page.
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pub first: Option<u32>,

    /// Last page to render (inclusive). Defaults to the last page.
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pub last: Option<u32>,

    /// Run the (paid) Claude correction stage. Without this the pipeline stops at raw OCR.
    #[arg(long)]
    pub correct: bool,

    #[command(flatten)]
    pub ocr: OcrOptions,

    #[command(flatten)]
    pub correct_opts: CorrectOptions,
}

/// clap value parser that reuses the library's device parsing.
fn parse_device(s: &str) -> Result<OutputDevice, String> {
    OutputDevice::parse(s).ok_or_else(|| {
        format!("unknown device '{s}' (expected png16m, pnggray, pngmono, jpeg, or tiff24nc)")
    })
}

/// clap value parser for the OCR recognition level.
fn parse_level(s: &str) -> Result<RecognitionLevel, String> {
    match s.to_ascii_lowercase().as_str() {
        "accurate" => Ok(RecognitionLevel::Accurate),
        "fast" => Ok(RecognitionLevel::Fast),
        _ => Err(format!("unknown level '{s}' (expected accurate or fast)")),
    }
}

/// clap value parser for a confidence threshold in 0.0..=1.0.
fn parse_confidence(s: &str) -> Result<f32, String> {
    let v: f32 = s.parse().map_err(|_| format!("'{s}' is not a number"))?;
    if (0.0..=1.0).contains(&v) {
        Ok(v)
    } else {
        Err(format!("confidence {v} is out of range (expected 0.0..=1.0)"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("updf").chain(args.iter().copied()))
    }

    fn pdf_args(args: &[&str]) -> PdfToImagesArgs {
        match parse(args).unwrap().command {
            Command::PdfToImages(a) => a,
            other => panic!("expected pdf-to-images, got {other:?}"),
        }
    }

    #[test]
    fn pdf_to_images_applies_defaults() {
        let a = pdf_args(&["pdf-to-images"]);
        assert_eq!(a.dpi, 150);
        assert!(matches!(a.device, OutputDevice::Png16m));
        assert_eq!(a.first, None);
        assert_eq!(a.last, None);
        assert_eq!(a.input, PathBuf::from(DEFAULT_INPUT));
        assert_eq!(a.output, PathBuf::from(DEFAULT_OUTPUT));
    }

    #[test]
    fn pdf_to_images_parses_all_options() {
        let a = pdf_args(&[
            "pdf-to-images", "in.pdf", "out", "--dpi", "300", "--device", "jpeg", "--first",
            "2", "--last", "5",
        ]);
        assert_eq!(a.input, PathBuf::from("in.pdf"));
        assert_eq!(a.output, PathBuf::from("out"));
        assert_eq!(a.dpi, 300);
        assert!(matches!(a.device, OutputDevice::Jpeg));
        assert_eq!(a.first, Some(2));
        assert_eq!(a.last, Some(5));
    }

    #[test]
    fn device_aliases_are_accepted() {
        let a = pdf_args(&["pdf-to-images", "--device", "jpg"]);
        assert!(matches!(a.device, OutputDevice::Jpeg));
    }

    #[test]
    fn unknown_device_is_rejected() {
        let err = parse(&["pdf-to-images", "--device", "webp"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn zero_dpi_is_rejected() {
        let err = parse(&["pdf-to-images", "--dpi", "0"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn non_numeric_dpi_is_rejected() {
        let err = parse(&["pdf-to-images", "--dpi", "abc"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn missing_subcommand_is_rejected() {
        let err = parse(&[]).unwrap_err();
        // clap surfaces an omitted subcommand by displaying help with a non-zero exit.
        assert_eq!(
            err.kind(),
            clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
    }

    #[test]
    fn unknown_subcommand_is_rejected() {
        let err = parse(&["frobnicate"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidSubcommand);
    }

    #[test]
    fn ocr_applies_defaults() {
        let cli = parse(&["ocr", "imgs"]).unwrap();
        let Command::Ocr(a) = cli.command else { panic!("expected ocr") };
        assert_eq!(a.images_dir, PathBuf::from("imgs"));
        assert_eq!(a.output, None);
        assert!(matches!(a.ocr.level, RecognitionLevel::Accurate));
        assert_eq!(a.ocr.languages, vec!["en-US".to_string()]);
        assert_eq!(a.ocr.min_confidence, 0.0);
        assert!(!a.ocr.no_language_correction);
    }

    #[test]
    fn ocr_parses_options() {
        let cli = parse(&[
            "ocr", "imgs", "out.md", "--level", "fast", "--languages", "en-US,fr-FR",
            "--min-confidence", "0.4", "--no-language-correction",
        ])
        .unwrap();
        let Command::Ocr(a) = cli.command else { panic!("expected ocr") };
        assert_eq!(a.output, Some(PathBuf::from("out.md")));
        assert!(matches!(a.ocr.level, RecognitionLevel::Fast));
        assert_eq!(a.ocr.languages, vec!["en-US".to_string(), "fr-FR".to_string()]);
        assert_eq!(a.ocr.min_confidence, 0.4);
        assert!(a.ocr.no_language_correction);
    }

    #[test]
    fn out_of_range_confidence_is_rejected() {
        let err = parse(&["ocr", "imgs", "--min-confidence", "1.5"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn unknown_level_is_rejected() {
        let err = parse(&["ocr", "imgs", "--level", "blurry"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn correct_applies_defaults() {
        let cli = parse(&["correct", "in.ocr.md"]).unwrap();
        let Command::Correct(a) = cli.command else { panic!("expected correct") };
        assert_eq!(a.input, PathBuf::from("in.ocr.md"));
        assert_eq!(a.output, None);
        assert_eq!(a.correct.model, None);
        assert_eq!(a.correct.target, None);
        assert!(!a.correct.no_normalize);
    }

    #[test]
    fn correct_parses_options() {
        let cli = parse(&[
            "correct", "in.md", "out.md", "--model", "claude-sonnet-4-6", "--target",
            "t.json", "--no-normalize",
        ])
        .unwrap();
        let Command::Correct(a) = cli.command else { panic!("expected correct") };
        assert_eq!(a.output, Some(PathBuf::from("out.md")));
        assert_eq!(a.correct.model.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(a.correct.target, Some(PathBuf::from("t.json")));
        assert!(a.correct.no_normalize);
    }

    #[test]
    fn pipeline_applies_defaults() {
        let cli = parse(&["pipeline"]).unwrap();
        let Command::Pipeline(a) = cli.command else { panic!("expected pipeline") };
        assert_eq!(a.input, PathBuf::from(DEFAULT_INPUT));
        assert_eq!(a.output, PathBuf::from(DEFAULT_OUTPUT));
        assert_eq!(a.dpi, 150);
        assert!(!a.correct);
        assert!(matches!(a.ocr.level, RecognitionLevel::Accurate));
    }

    #[test]
    fn pipeline_parses_correct_flag_and_stage_options() {
        let cli = parse(&[
            "pipeline", "book.pdf", "out", "--dpi", "200", "--device", "pnggray", "--first",
            "1", "--last", "3", "--correct", "--level", "fast", "--model", "claude-opus-4-8",
        ])
        .unwrap();
        let Command::Pipeline(a) = cli.command else { panic!("expected pipeline") };
        assert_eq!(a.input, PathBuf::from("book.pdf"));
        assert_eq!(a.dpi, 200);
        assert!(matches!(a.device, OutputDevice::PngGray));
        assert_eq!(a.first, Some(1));
        assert_eq!(a.last, Some(3));
        assert!(a.correct);
        assert!(matches!(a.ocr.level, RecognitionLevel::Fast));
        assert_eq!(a.correct_opts.model.as_deref(), Some("claude-opus-4-8"));
    }
}
