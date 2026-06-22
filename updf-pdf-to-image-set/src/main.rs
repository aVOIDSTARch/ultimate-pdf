// updf-pdf-to-image-set
//
// Render a PDF (or every PDF in a directory) to one image per page, using Ghostscript.
// Each PDF gets its own output directory named after the file:
//   <output>/<pdf-stem>/page-1.png, page-2.png, …
//
// Usage:
//   updf-pdf-to-image-set [INPUT] [OUTPUT] [--dpi N] [--device png16m|pnggray|jpeg|...]
//     INPUT   a .pdf file or a directory of .pdf files   (default: ../books/unprocessed)
//     OUTPUT  base directory for the per-PDF folders      (default: ../books/processed)

mod ghostscript;
mod directory;
mod convert;

use std::path::PathBuf;
use std::process::ExitCode;

use convert::{ConvertOptions, convert_path};
use ghostscript::OutputDevice;

/// Defaults are resolved relative to the crate so they work from any working directory.
fn default_input() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../books/unprocessed"))
}
fn default_output() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../books/processed"))
}

struct Cli {
    input: PathBuf,
    output: PathBuf,
    options: ConvertOptions,
}

fn parse_args() -> Result<Cli, String> {
    let mut positionals: Vec<String> = Vec::new();
    let mut options = ConvertOptions::default();

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dpi" => {
                let v = args.next().ok_or("--dpi requires a value")?;
                options.dpi = v.parse().map_err(|_| format!("invalid --dpi: {v}"))?;
            }
            "--device" => {
                let v = args.next().ok_or("--device requires a value")?;
                options.device = OutputDevice::parse(&v)
                    .ok_or_else(|| format!("unknown --device: {v}"))?;
            }
            "--first" => {
                let v = args.next().ok_or("--first requires a value")?;
                options.first_page = Some(v.parse().map_err(|_| format!("invalid --first: {v}"))?);
            }
            "--last" => {
                let v = args.next().ok_or("--last requires a value")?;
                options.last_page = Some(v.parse().map_err(|_| format!("invalid --last: {v}"))?);
            }
            "-h" | "--help" => return Err("help".to_string()),
            other if other.starts_with('-') => {
                return Err(format!("unknown flag: {other}"));
            }
            other => positionals.push(other.to_string()),
        }
    }

    if positionals.len() > 2 {
        return Err("too many positional arguments".to_string());
    }

    let input = positionals
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(default_input);
    let output = positionals
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(default_output);

    Ok(Cli { input, output, options })
}

const USAGE: &str = "\
usage: updf-pdf-to-image-set [INPUT] [OUTPUT] [--dpi N] [--device NAME]
  INPUT   a .pdf file or a directory of .pdf files   (default: ../books/unprocessed)
  OUTPUT  base directory for per-PDF image folders   (default: ../books/processed)
  --dpi N         render resolution (default: 150)
  --device NAME   png16m | pnggray | pngmono | jpeg | tiff24nc (default: png16m)
  --first N       first page to render (default: 1)
  --last N        last page to render (default: last)
Each PDF is written to <OUTPUT>/<pdf-stem>/page-N.<ext>, one image per page.";

fn main() -> ExitCode {
    let cli = match parse_args() {
        Ok(cli) => cli,
        Err(msg) => {
            if msg == "help" {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            eprintln!("error: {msg}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    println!(
        "Rendering {} -> {} (device: {}, {} dpi)",
        cli.input.display(),
        cli.output.display(),
        cli.options.device.file_extension(),
        cli.options.dpi,
    );

    let outcomes = match convert_path(&cli.input, &cli.output, &cli.options) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut failures = 0;
    for outcome in &outcomes {
        let name = outcome
            .pdf
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| outcome.pdf.display().to_string());
        match &outcome.result {
            Ok(dir) => println!("  ok    {name} -> {}", dir.display()),
            Err(e) => {
                failures += 1;
                eprintln!("  FAIL  {name}: {e}");
            }
        }
    }

    println!(
        "\nDone: {} ok, {} failed ({} pdf{} total)",
        outcomes.len() - failures,
        failures,
        outcomes.len(),
        if outcomes.len() == 1 { "" } else { "s" },
    );

    if failures > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
