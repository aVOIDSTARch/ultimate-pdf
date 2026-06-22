// Apple Vision OCR pipeline stage.
//
// Public API: build an `OcrJob`, call `vision::extract` (batch) or `vision::extract_one`
// (single image), and receive `VisionResults` per image. The actual Apple Vision work is
// done by the bundled `vision-ocr` Swift helper, which this crate drives as a subprocess.

pub mod types;

#[path = "vision-swift-integration.rs"]
pub mod vision;
