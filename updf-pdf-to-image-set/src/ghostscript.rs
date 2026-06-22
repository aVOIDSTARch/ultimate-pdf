// src/ghostscript.rs

use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy)]
pub enum OutputDevice {
    Png16m,
    PngGray,
    PngMono,
    Jpeg,
    Tiff24nc,
}

impl OutputDevice {
    fn as_gs_str(&self) -> &'static str {
        match self {
            Self::Png16m   => "png16m",
            Self::PngGray  => "pnggray",
            Self::PngMono  => "pngmono",
            Self::Jpeg     => "jpeg",
            Self::Tiff24nc => "tiff24nc",
        }
    }

    /// File extension to use for the rendered images of this device.
    pub fn file_extension(&self) -> &'static str {
        match self {
            Self::Png16m | Self::PngGray | Self::PngMono => "png",
            Self::Jpeg     => "jpg",
            Self::Tiff24nc => "tiff",
        }
    }

    /// Parse a device from a user-supplied name (e.g. a CLI flag). Case-insensitive,
    /// accepts the common aliases `jpg`/`tiff`.
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "png16m"          => Some(Self::Png16m),
            "pnggray"         => Some(Self::PngGray),
            "pngmono"         => Some(Self::PngMono),
            "jpeg" | "jpg"    => Some(Self::Jpeg),
            "tiff24nc" | "tiff" => Some(Self::Tiff24nc),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RenderJob {
    pub input:          PathBuf,
    pub output_pattern: PathBuf,   // e.g. "out/page-%d.png"
    pub device:         OutputDevice,
    pub dpi:            u32,
    pub first_page:     Option<u32>,
    pub last_page:      Option<u32>,
}

impl RenderJob {
    pub fn new(input: impl Into<PathBuf>, output_pattern: impl Into<PathBuf>) -> Self {
        Self {
            input:          input.into(),
            output_pattern: output_pattern.into(),
            device:         OutputDevice::Png16m,
            dpi:            150,
            first_page:     None,
            last_page:      None,
        }
    }

    pub fn device(mut self, device: OutputDevice) -> Self {
        self.device = device;
        self
    }

    pub fn dpi(mut self, dpi: u32) -> Self {
        self.dpi = dpi;
        self
    }

    pub fn pages(mut self, first: u32, last: u32) -> Self {
        self.first_page = Some(first);
        self.last_page  = Some(last);
        self
    }

    pub fn first_page(mut self, page: u32) -> Self {
        self.first_page = Some(page);
        self
    }

    pub fn last_page(mut self, page: u32) -> Self {
        self.last_page = Some(page);
        self
    }
}

#[derive(Debug)]
pub struct GhostscriptError {
    pub exit_code: Option<i32>,
    pub stderr:    String,
}

impl std::fmt::Display for GhostscriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ghostscript failed (exit: {:?}):\n{}",
            self.exit_code, self.stderr
        )
    }
}

impl std::error::Error for GhostscriptError {}

pub fn render(job: &RenderJob) -> Result<(), GhostscriptError> {
    let mut cmd = Command::new("gs");

    cmd.args(["-dBATCH", "-dNOPAUSE", "-dSAFER"]);
    cmd.arg(format!("-sDEVICE={}", job.device.as_gs_str()));
    cmd.arg(format!("-r{}", job.dpi));
    cmd.arg(format!("-sOutputFile={}", job.output_pattern.display()));

    if let Some(p) = job.first_page {
        cmd.arg(format!("-dFirstPage={p}"));
    }
    if let Some(p) = job.last_page {
        cmd.arg(format!("-dLastPage={p}"));
    }

    cmd.arg(&job.input);
    cmd.stderr(Stdio::piped());

    let output = cmd.output().map_err(|e| GhostscriptError {
        exit_code: None,
        stderr:    format!("failed to spawn gs: {e}"),
    })?;

    if !output.status.success() {
        return Err(GhostscriptError {
            exit_code: output.status.code(),
            stderr:    String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_canonical_names_and_aliases() {
        assert!(matches!(OutputDevice::parse("png16m"), Some(OutputDevice::Png16m)));
        assert!(matches!(OutputDevice::parse("pnggray"), Some(OutputDevice::PngGray)));
        assert!(matches!(OutputDevice::parse("pngmono"), Some(OutputDevice::PngMono)));
        assert!(matches!(OutputDevice::parse("jpeg"), Some(OutputDevice::Jpeg)));
        assert!(matches!(OutputDevice::parse("jpg"), Some(OutputDevice::Jpeg)));
        assert!(matches!(OutputDevice::parse("tiff24nc"), Some(OutputDevice::Tiff24nc)));
        assert!(matches!(OutputDevice::parse("tiff"), Some(OutputDevice::Tiff24nc)));
    }

    #[test]
    fn parse_is_case_insensitive() {
        assert!(matches!(OutputDevice::parse("PNG16M"), Some(OutputDevice::Png16m)));
        assert!(matches!(OutputDevice::parse("JpEg"), Some(OutputDevice::Jpeg)));
    }

    #[test]
    fn parse_rejects_unknown() {
        assert!(OutputDevice::parse("").is_none());
        assert!(OutputDevice::parse("bmp").is_none());
        assert!(OutputDevice::parse("png").is_none());
    }

    #[test]
    fn file_extension_matches_device() {
        assert_eq!(OutputDevice::Png16m.file_extension(), "png");
        assert_eq!(OutputDevice::PngGray.file_extension(), "png");
        assert_eq!(OutputDevice::PngMono.file_extension(), "png");
        assert_eq!(OutputDevice::Jpeg.file_extension(), "jpg");
        assert_eq!(OutputDevice::Tiff24nc.file_extension(), "tiff");
    }

    #[test]
    fn builder_sets_page_range() {
        let job = RenderJob::new("in.pdf", "out/page-%d.png").pages(2, 5);
        assert_eq!(job.first_page, Some(2));
        assert_eq!(job.last_page, Some(5));
    }
}
