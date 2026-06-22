// updf — unified command-line front-end for the Ultimate PDF toolkit.
//
// Argument parsing lives in `cli`; stage orchestration lives in `pipeline`. This file
// maps each subcommand onto those and onto process exit codes:
//   0  success
//   1  a stage failed at runtime
//   2  invalid usage (clap parse errors, or semantic validation here)

mod cli;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;

use agent_text_cleanup::agent::{ClaudeClient, FormatTarget};
use agent_text_cleanup::api::{CorrectionApi, load_target};
use apple_vision_image_text_extractor::vision::OcrJob;
use ultimate_pdf::pipeline;
use updf_pdf_to_image_set::convert::{ConvertOptions, convert_path};

use cli::{
    Cli, Command, CorrectArgs, CorrectOptions, OcrArgs, OcrOptions, PdfToImagesArgs, PipelineArgs,
};

#[tokio::main]
async fn main() -> ExitCode {
    // Pick up ANTHROPIC_API_KEY (and anything else) from a local .env if present.
    let _ = dotenvy::dotenv();

    let cli = Cli::parse();
    match cli.command {
        Command::PdfToImages(args) => run_pdf_to_images(args),
        Command::Ocr(args) => run_ocr_cmd(args),
        Command::Correct(args) => run_correct_cmd(args).await,
        Command::Pipeline(args) => run_pipeline(args).await,
    }
}

// ---------------------------------------------------------------------------
// pdf-to-images
// ---------------------------------------------------------------------------

fn run_pdf_to_images(args: PdfToImagesArgs) -> ExitCode {
    if let Some(code) = reject_inverted_range(args.first, args.last) {
        return code;
    }

    let options = ConvertOptions {
        device: args.device,
        dpi: args.dpi,
        first_page: args.first,
        last_page: args.last,
    };

    println!(
        "Rendering {} -> {} (device: {}, {} dpi)",
        args.input.display(),
        args.output.display(),
        options.device.file_extension(),
        options.dpi,
    );

    let outcomes = match convert_path(&args.input, &args.output, &options) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut failures = 0;
    for outcome in &outcomes {
        let name = pdf_label(&outcome.pdf);
        match &outcome.result {
            Ok(dir) => println!("  ok    {name} -> {}", dir.display()),
            Err(e) => {
                failures += 1;
                eprintln!("  FAIL  {name}: {e}");
            }
        }
    }

    print_batch_summary(outcomes.len(), failures);
    exit_code(failures == 0)
}

// ---------------------------------------------------------------------------
// ocr
// ---------------------------------------------------------------------------

fn run_ocr_cmd(args: OcrArgs) -> ExitCode {
    if !args.images_dir.is_dir() {
        eprintln!("error: not a directory: {}", args.images_dir.display());
        return ExitCode::from(2);
    }

    let output = args
        .output
        .clone()
        .unwrap_or_else(|| default_ocr_path(&args.images_dir));

    let job = build_ocr_job(&args.images_dir, &args.ocr);
    println!("OCR'ing images in {} ...", args.images_dir.display());

    let markdown = match pipeline::run_ocr(&job) {
        Ok(md) => md,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    match write_output(&output, &markdown) {
        Ok(()) => {
            println!("  ok    wrote {}", output.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: could not write {}: {e}", output.display());
            ExitCode::FAILURE
        }
    }
}

// ---------------------------------------------------------------------------
// correct
// ---------------------------------------------------------------------------

async fn run_correct_cmd(args: CorrectArgs) -> ExitCode {
    let markdown = match std::fs::read_to_string(&args.input) {
        Ok(md) => md,
        Err(e) => {
            eprintln!("error: could not read {}: {e}", args.input.display());
            return ExitCode::FAILURE;
        }
    };

    let (api, target) = match build_correction(&args.correct) {
        Ok(pair) => pair,
        Err(code) => return code,
    };

    let output = args
        .output
        .clone()
        .unwrap_or_else(|| default_corrected_path(&args.input));

    println!("Correcting {} ...", args.input.display());
    let outcome =
        pipeline::correct_document(&api, &markdown, target.as_ref(), !args.correct.no_normalize)
            .await;

    if let Err(e) = write_output(&output, &outcome.markdown) {
        eprintln!("error: could not write {}: {e}", output.display());
        return ExitCode::FAILURE;
    }
    println!("  ok    wrote {}", output.display());

    report_failures(&outcome.failures)
}

// ---------------------------------------------------------------------------
// pipeline
// ---------------------------------------------------------------------------

async fn run_pipeline(args: PipelineArgs) -> ExitCode {
    if let Some(code) = reject_inverted_range(args.first, args.last) {
        return code;
    }

    // Fail fast on a missing/invalid key or target *before* doing any OCR work.
    let correction = if args.correct {
        match build_correction(&args.correct_opts) {
            Ok(pair) => Some(pair),
            Err(code) => return code,
        }
    } else {
        None
    };

    let options = ConvertOptions {
        device: args.device,
        dpi: args.dpi,
        first_page: args.first,
        last_page: args.last,
    };

    println!(
        "Pipeline {} -> {} (device: {}, {} dpi, correct: {})",
        args.input.display(),
        args.output.display(),
        options.device.file_extension(),
        options.dpi,
        args.correct,
    );

    let outcomes = match convert_path(&args.input, &args.output, &options) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut failures = 0;
    for outcome in &outcomes {
        let name = pdf_label(&outcome.pdf);
        let dir = match &outcome.result {
            Ok(dir) => dir,
            Err(e) => {
                failures += 1;
                eprintln!("  FAIL  {name}: render: {e}");
                continue;
            }
        };

        match process_one(dir, &args.ocr, correction.as_ref()).await {
            Ok(()) => println!("  ok    {name} -> {}", dir.display()),
            Err(e) => {
                failures += 1;
                eprintln!("  FAIL  {name}: {e}");
            }
        }
    }

    print_batch_summary(outcomes.len(), failures);
    exit_code(failures == 0)
}

/// Run the OCR (and optional correction) stages for one already-rendered PDF folder.
async fn process_one(
    dir: &Path,
    ocr_opts: &OcrOptions,
    correction: Option<&(CorrectionApi, Option<FormatTarget>)>,
) -> Result<(), String> {
    let stem = dir
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "output directory has no name".to_string())?;

    let job = build_ocr_job(dir, ocr_opts);
    let ocr_md = pipeline::run_ocr(&job).map_err(|e| format!("ocr: {e}"))?;

    let ocr_path = dir.join(format!("{stem}.ocr.md"));
    write_output(&ocr_path, &ocr_md).map_err(|e| format!("write {}: {e}", ocr_path.display()))?;

    if let Some((api, target)) = correction {
        // The pipeline always runs the offline normalize pass before the agent.
        let outcome = pipeline::correct_document(api, &ocr_md, target.as_ref(), true).await;
        let md_path = dir.join(format!("{stem}.md"));
        write_output(&md_path, &outcome.markdown)
            .map_err(|e| format!("write {}: {e}", md_path.display()))?;
        if !outcome.failures.is_empty() {
            return Err(format!(
                "{} page(s) failed correction (first: page {}: {})",
                outcome.failures.len(),
                outcome.failures[0].page,
                outcome.failures[0].error,
            ));
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

/// Build an `OcrJob` for a directory of page images from the shared OCR options.
fn build_ocr_job(images_dir: &Path, opts: &OcrOptions) -> OcrJob {
    OcrJob::directory(images_dir)
        .level(opts.level)
        .languages(opts.languages.clone())
        .use_language_correction(!opts.no_language_correction)
        .min_confidence(opts.min_confidence)
}

/// Build a configured correction surface (and optional format target) from CLI options.
/// On failure, prints the error and returns the exit code to propagate.
fn build_correction(
    opts: &CorrectOptions,
) -> Result<(CorrectionApi, Option<FormatTarget>), ExitCode> {
    let mut client = match ClaudeClient::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return Err(ExitCode::FAILURE);
        }
    };
    if let Some(model) = &opts.model {
        client = client.with_model(model.clone());
    }

    let target = match &opts.target {
        Some(path) => match load_target(path) {
            Ok(t) => Some(t),
            Err(e) => {
                eprintln!("error: {e}");
                return Err(ExitCode::FAILURE);
            }
        },
        None => None,
    };

    Ok((CorrectionApi::new(client), target))
}

/// Default OCR output: `<images_dir>/<dir-name>.ocr.md`.
fn default_ocr_path(images_dir: &Path) -> PathBuf {
    let name = images_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("ocr");
    images_dir.join(format!("{name}.ocr.md"))
}

/// Default corrected output: sibling `<stem>.md`, or `<stem>.corrected.md` if that
/// would collide with the input file.
fn default_corrected_path(input: &Path) -> PathBuf {
    let name = input.file_name().and_then(|s| s.to_str()).unwrap_or("output");
    let base = name
        .strip_suffix(".ocr.md")
        .or_else(|| name.strip_suffix(".md"))
        .unwrap_or(name);
    let candidate = input.with_file_name(format!("{base}.md"));
    if candidate == input {
        input.with_file_name(format!("{base}.corrected.md"))
    } else {
        candidate
    }
}

/// Write `contents` to `path`, creating parent directories as needed.
fn write_output(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(path, contents)
}

/// Print per-page correction failures and return the appropriate exit code.
fn report_failures(failures: &[pipeline::PageFailure]) -> ExitCode {
    if failures.is_empty() {
        return ExitCode::SUCCESS;
    }
    eprintln!("warning: {} page(s) could not be corrected:", failures.len());
    for f in failures {
        eprintln!("  page {}: {}", f.page, f.error);
    }
    ExitCode::FAILURE
}

fn pdf_label(pdf: &Path) -> String {
    pdf.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| pdf.display().to_string())
}

fn reject_inverted_range(first: Option<u32>, last: Option<u32>) -> Option<ExitCode> {
    if let (Some(first), Some(last)) = (first, last) {
        if first > last {
            eprintln!("error: --first ({first}) must not exceed --last ({last})");
            return Some(ExitCode::from(2));
        }
    }
    None
}

fn print_batch_summary(total: usize, failures: usize) {
    println!(
        "\nDone: {} ok, {} failed ({} pdf{} total)",
        total - failures,
        failures,
        total,
        if total == 1 { "" } else { "s" },
    );
}

fn exit_code(ok: bool) -> ExitCode {
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
