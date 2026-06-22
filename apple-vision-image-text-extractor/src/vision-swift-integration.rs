// Orchestration for the `vision-ocr` Swift helper.
//
// Mirrors the shape of `updf-pdf-to-image-set/src/ghostscript.rs`: a builder describing
// the job, a typed error, and a single function that shells out and returns parsed
// results. The heavy lifting (Apple Vision) lives in the Swift CLI; Rust just drives it
// and deserializes its JSON.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::types::VisionResults;

/// Recognition accuracy/speed trade-off, forwarded to the Swift `--level` flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecognitionLevel {
    Accurate,
    Fast,
}

impl RecognitionLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Accurate => "accurate",
            Self::Fast => "fast",
        }
    }
}

/// What to OCR: an explicit set of files, or every image in a directory.
#[derive(Debug, Clone)]
pub enum OcrInput {
    Files(Vec<PathBuf>),
    Dir(PathBuf),
}

/// Default location of the release-built helper, relative to this crate.
fn default_binary() -> PathBuf {
    if let Ok(p) = std::env::var("VISION_OCR_BIN") {
        return PathBuf::from(p);
    }
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/swift/.build/release/vision-ocr"
    ))
}

/// Builder describing an OCR run (analogous to `RenderJob`).
#[derive(Debug, Clone)]
pub struct OcrJob {
    pub input: OcrInput,
    pub level: RecognitionLevel,
    pub languages: Vec<String>,
    pub use_language_correction: bool,
    pub min_confidence: f32,
    /// Path to the `vision-ocr` binary. Defaults to the release build / `VISION_OCR_BIN`.
    pub binary: PathBuf,
}

impl OcrJob {
    pub fn new(input: OcrInput) -> Self {
        Self {
            input,
            level: RecognitionLevel::Accurate,
            languages: vec!["en-US".to_string()],
            use_language_correction: true,
            min_confidence: 0.0,
            binary: default_binary(),
        }
    }

    /// Convenience constructor for the one-off case.
    pub fn single(image: impl Into<PathBuf>) -> Self {
        Self::new(OcrInput::Files(vec![image.into()]))
    }

    /// Convenience constructor for a directory of images.
    pub fn directory(dir: impl Into<PathBuf>) -> Self {
        Self::new(OcrInput::Dir(dir.into()))
    }

    pub fn level(mut self, level: RecognitionLevel) -> Self {
        self.level = level;
        self
    }

    pub fn languages(mut self, langs: impl IntoIterator<Item = String>) -> Self {
        self.languages = langs.into_iter().collect();
        self
    }

    pub fn use_language_correction(mut self, yes: bool) -> Self {
        self.use_language_correction = yes;
        self
    }

    pub fn min_confidence(mut self, min: f32) -> Self {
        self.min_confidence = min;
        self
    }

    pub fn binary(mut self, path: impl Into<PathBuf>) -> Self {
        self.binary = path.into();
        self
    }

    /// Assemble the CLI arguments for this job (excluding the program name).
    /// Factored out so it can be unit-tested without spawning a process.
    fn build_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        match &self.input {
            OcrInput::Files(files) => {
                for f in files {
                    args.push(f.to_string_lossy().into_owned());
                }
            }
            OcrInput::Dir(dir) => {
                args.push("--dir".to_string());
                args.push(dir.to_string_lossy().into_owned());
            }
        }
        args.push("--level".to_string());
        args.push(self.level.as_str().to_string());
        if !self.languages.is_empty() {
            args.push("--languages".to_string());
            args.push(self.languages.join(","));
        }
        if !self.use_language_correction {
            args.push("--no-correction".to_string());
        }
        if self.min_confidence > 0.0 {
            args.push("--min-confidence".to_string());
            args.push(self.min_confidence.to_string());
        }
        args
    }
}

/// Failure modes when driving the Swift helper.
#[derive(Debug)]
pub enum VisionError {
    /// Could not spawn the helper (missing binary, etc.).
    Spawn { binary: PathBuf, source: std::io::Error },
    /// Helper exited nonzero.
    Tool { exit_code: Option<i32>, stderr: String },
    /// Helper output could not be parsed as the expected JSON.
    Parse(serde_json::Error),
}

impl std::fmt::Display for VisionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn { binary, source } => write!(
                f,
                "failed to spawn vision-ocr ({}): {source}\n\
                 hint: build it with `cd swift && swift build -c release`, \
                 or set VISION_OCR_BIN",
                binary.display()
            ),
            Self::Tool { exit_code, stderr } => {
                write!(f, "vision-ocr failed (exit: {exit_code:?}):\n{stderr}")
            }
            Self::Parse(e) => write!(f, "could not parse vision-ocr output: {e}"),
        }
    }
}

impl std::error::Error for VisionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn { source, .. } => Some(source),
            Self::Parse(e) => Some(e),
            Self::Tool { .. } => None,
        }
    }
}

/// Run OCR over the job's input and return one `VisionResults` per image.
pub fn extract(job: &OcrJob) -> Result<Vec<VisionResults>, VisionError> {
    let output = Command::new(&job.binary)
        .args(job.build_args())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| VisionError::Spawn {
            binary: job.binary.clone(),
            source: e,
        })?;

    if !output.status.success() {
        return Err(VisionError::Tool {
            exit_code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    serde_json::from_slice(&output.stdout).map_err(VisionError::Parse)
}

/// One-off convenience: OCR a single image and return its result.
pub fn extract_one(image: impl AsRef<Path>, job: &OcrJob) -> Result<VisionResults, VisionError> {
    let single = OcrJob {
        input: OcrInput::Files(vec![image.as_ref().to_path_buf()]),
        ..job.clone()
    };
    let mut results = extract(&single)?;
    // The helper always emits one object per input image.
    Ok(results.remove(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_args_for_files() {
        let job = OcrJob::new(OcrInput::Files(vec![
            PathBuf::from("a.png"),
            PathBuf::from("b.png"),
        ]));
        let args = job.build_args();
        assert_eq!(args[0], "a.png");
        assert_eq!(args[1], "b.png");
        assert!(args.iter().any(|a| a == "--level"));
        assert!(args.iter().any(|a| a == "accurate"));
        // defaults: correction on, no min-confidence flag
        assert!(!args.iter().any(|a| a == "--no-correction"));
        assert!(!args.iter().any(|a| a == "--min-confidence"));
    }

    #[test]
    fn build_args_for_dir_with_options() {
        let job = OcrJob::directory("imgs/")
            .level(RecognitionLevel::Fast)
            .use_language_correction(false)
            .min_confidence(0.5)
            .languages(["en-US".to_string(), "fr-FR".to_string()]);
        let args = job.build_args();
        assert_eq!(args[0], "--dir");
        assert_eq!(args[1], "imgs/");
        assert!(args.windows(2).any(|w| w[0] == "--level" && w[1] == "fast"));
        assert!(args.windows(2).any(|w| w[0] == "--languages" && w[1] == "en-US,fr-FR"));
        assert!(args.iter().any(|a| a == "--no-correction"));
        assert!(args.windows(2).any(|w| w[0] == "--min-confidence" && w[1] == "0.5"));
    }

    #[test]
    fn deserializes_helper_json() {
        let json = r#"[
          {
            "imageFileName": "page-1.png",
            "imagePath": "/tmp/page-1.png",
            "textBlob": "hello\nworld",
            "pixelWidth": 1240,
            "pixelHeight": 1754,
            "recognizedLanguages": ["en-US"],
            "lines": [
              {"text": "hello", "confidence": 0.98,
               "boundingBox": {"x": 0.1, "y": 0.8, "width": 0.5, "height": 0.03}}
            ],
            "error": null
          }
        ]"#;
        let results: Vec<VisionResults> = serde_json::from_str(json).unwrap();
        assert_eq!(results.len(), 1);
        let r = &results[0];
        assert_eq!(r.image_file_name, "page-1.png");
        assert_eq!(r.text_blob, "hello\nworld");
        assert_eq!(r.pixel_width, 1240);
        assert_eq!(r.lines.len(), 1);
        assert!((r.lines[0].confidence - 0.98).abs() < 1e-6);
        assert!(r.error.is_none());
    }

    #[test]
    fn deserializes_per_image_error() {
        let json = r#"[
          {"imageFileName": "bad.png", "imagePath": "/tmp/bad.png", "textBlob": "",
           "pixelWidth": 0, "pixelHeight": 0, "recognizedLanguages": ["en-US"],
           "lines": [], "error": "could not load image"}
        ]"#;
        let results: Vec<VisionResults> = serde_json::from_str(json).unwrap();
        assert_eq!(results[0].error.as_deref(), Some("could not load image"));
    }
}
