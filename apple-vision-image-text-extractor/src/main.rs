// Demo driver for the Apple Vision OCR stage.
//
// Mirrors `updf-pdf-to-image-set/src/main.rs`: builds a job, runs it, prints a summary.
// The real value is the library API in the `vision` module.

use apple_vision_image_text_extractor::vision::{OcrJob, extract};

fn main() {
    // Point at the images produced by the PDF stage. Override with the first CLI arg.
    // Default is resolved relative to the crate so it works from any working directory.
    let dir = std::env::args().nth(1).unwrap_or_else(|| {
        format!(
            "{}/../books/processed/alan-watts/essentials",
            env!("CARGO_MANIFEST_DIR")
        )
    });

    println!("OCR'ing images in {dir} ...");
    let job = OcrJob::directory(&dir);

    match extract(&job) {
        Ok(results) => {
            println!("processed {} image(s)\n", results.len());
            for r in &results {
                match &r.error {
                    Some(e) => println!("  {:<16} ERROR: {e}", r.image_file_name),
                    None => println!(
                        "  {:<16} {} line(s), {} chars",
                        r.image_file_name,
                        r.lines.len(),
                        r.text_blob.chars().count()
                    ),
                }
            }
        }
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}
