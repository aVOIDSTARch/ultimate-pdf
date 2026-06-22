// updf — unified command-line front-end for the Ultimate PDF toolkit.
//
// Argument parsing lives in `cli`; this file dispatches each subcommand to the relevant
// library and maps results onto process exit codes:
//   0  success
//   1  a conversion failed at runtime
//   2  invalid usage (clap parse errors, or semantic validation here)

mod cli;

use std::process::ExitCode;

use clap::Parser;

use cli::{Cli, Command, PdfToImagesArgs};
use updf_pdf_to_image_set::convert::{ConvertOptions, convert_path};

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::PdfToImages(args) => run_pdf_to_images(args),
    }
}

fn run_pdf_to_images(args: PdfToImagesArgs) -> ExitCode {
    if let (Some(first), Some(last)) = (args.first, args.last) {
        if first > last {
            eprintln!("error: --first ({first}) must not exceed --last ({last})");
            return ExitCode::from(2);
        }
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
