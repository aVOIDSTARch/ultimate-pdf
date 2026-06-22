// Types for the Vision Pipeline
//
// These mirror the JSON emitted by the `vision-ocr` Swift helper. Field names are
// camelCase on the wire (Swift), snake_case in Rust via `rename_all`.

use serde::{Deserialize, Serialize};

/// Holds the OCR output and metadata for a single image, ready for the next pipeline stage.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisionResults {
    /// File name only, e.g. "page-1.png".
    pub image_file_name: String,
    /// Absolute path the helper read.
    pub image_path: String,
    /// All recognized lines joined with '\n', in reading order.
    pub text_blob: String,
    pub pixel_width: u32,
    pub pixel_height: u32,
    /// Languages the recognizer was configured with.
    pub recognized_languages: Vec<String>,
    /// Per-line recognized text with confidence and geometry.
    pub lines: Vec<TextLine>,
    /// Set when this image failed to process (the batch continues regardless).
    pub error: Option<String>,
}

/// One recognized line of text.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextLine {
    pub text: String,
    /// Recognizer confidence, 0.0..=1.0.
    pub confidence: f32,
    pub bounding_box: BoundingBox,
}

/// Normalized bounding box (0.0..=1.0) with **bottom-left origin**, as Vision reports it.
/// Downstream layout code must flip `y` if it expects a top-left origin.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BoundingBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
