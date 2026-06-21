mod ghostscript;
use ghostscript::{OutputDevice, RenderJob, render};

mod directory;

const base_input_dir: &str = "../books/unprocessed/";
const base_output_dir: &str = "../books/processed/";

fn main() {
    let job = RenderJob::new(
        "../books/unprocessed/alan-watts-essentials.pdf",
        "../books/processed/alan-watts/essentials/page-%d.png",
    )
    .device(OutputDevice::Png16m)
    .dpi(150)
    .pages(1, 10);  // optional — omit for all pages

    if let Err(e) = render(&job) {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
