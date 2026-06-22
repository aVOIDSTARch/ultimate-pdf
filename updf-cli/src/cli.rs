// Command-line surface for the Ultimate PDF toolkit.
//
// Parsing is defined here (separate from `main`) so it can be unit-tested with
// `Cli::try_parse_from`. Each subcommand maps onto a library call.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use updf_pdf_to_image_set::ghostscript::OutputDevice;

/// Defaults resolve relative to this crate so they work from any working directory.
/// Both crates are siblings under the workspace root, so `../books` is the same target
/// the converter used previously.
pub const DEFAULT_INPUT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../books/unprocessed");
pub const DEFAULT_OUTPUT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../books/processed");

#[derive(Debug, Parser)]
#[command(
    name = "updf",
    version,
    about = "Ultimate PDF toolkit",
    propagate_version = true
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

/// clap value parser that reuses the library's device parsing.
fn parse_device(s: &str) -> Result<OutputDevice, String> {
    OutputDevice::parse(s).ok_or_else(|| {
        format!("unknown device '{s}' (expected png16m, pnggray, pngmono, jpeg, or tiff24nc)")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("updf").chain(args.iter().copied()))
    }

    #[test]
    fn pdf_to_images_applies_defaults() {
        let cli = parse(&["pdf-to-images"]).unwrap();
        let Command::PdfToImages(a) = cli.command;
        assert_eq!(a.dpi, 150);
        assert!(matches!(a.device, OutputDevice::Png16m));
        assert_eq!(a.first, None);
        assert_eq!(a.last, None);
        assert_eq!(a.input, PathBuf::from(DEFAULT_INPUT));
        assert_eq!(a.output, PathBuf::from(DEFAULT_OUTPUT));
    }

    #[test]
    fn pdf_to_images_parses_all_options() {
        let cli = parse(&[
            "pdf-to-images", "in.pdf", "out", "--dpi", "300", "--device", "jpeg", "--first",
            "2", "--last", "5",
        ])
        .unwrap();
        let Command::PdfToImages(a) = cli.command;
        assert_eq!(a.input, PathBuf::from("in.pdf"));
        assert_eq!(a.output, PathBuf::from("out"));
        assert_eq!(a.dpi, 300);
        assert!(matches!(a.device, OutputDevice::Jpeg));
        assert_eq!(a.first, Some(2));
        assert_eq!(a.last, Some(5));
    }

    #[test]
    fn device_aliases_are_accepted() {
        let cli = parse(&["pdf-to-images", "--device", "jpg"]).unwrap();
        let Command::PdfToImages(a) = cli.command;
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
}
