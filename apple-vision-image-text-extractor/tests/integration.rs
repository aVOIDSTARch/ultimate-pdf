// End-to-end tests that drive the real Swift `vision-ocr` helper against the sample page
// images in `books/processed/alan-watts/essentials`.
//
// These require the helper to be built (`cd swift && swift build -c release`). If it is
// missing they SKIP (print a notice and pass) rather than fail, so `cargo test` stays
// green on machines where the Swift tool hasn't been built yet. Run on a Mac with the
// helper built to actually exercise Apple Vision.

use std::path::{Path, PathBuf};

use apple_vision_image_text_extractor::vision::{
    OcrInput, OcrJob, RecognitionLevel, extract, extract_one,
};

fn helper_path() -> PathBuf {
    if let Ok(p) = std::env::var("VISION_OCR_BIN") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("swift/.build/release/vision-ocr")
}

fn images_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../books/processed/alan-watts/essentials")
}

/// Returns true if prerequisites are present; otherwise prints a skip notice.
fn ready() -> bool {
    if !helper_path().exists() {
        eprintln!(
            "SKIP: vision-ocr not built at {} (run `cd swift && swift build -c release`)",
            helper_path().display()
        );
        return false;
    }
    if !images_dir().exists() {
        eprintln!("SKIP: sample images not found at {}", images_dir().display());
        return false;
    }
    true
}

/// Pick a sample image deterministically.
fn sample_image() -> Option<PathBuf> {
    let mut pngs: Vec<PathBuf> = std::fs::read_dir(images_dir())
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("png"))
        .collect();
    pngs.sort();
    pngs.into_iter().next()
}

#[test]
fn one_off_extracts_text() {
    if !ready() {
        return;
    }
    let img = sample_image().expect("at least one png present");
    let job = OcrJob::single(img.clone());
    let result = extract_one(&img, &job).expect("extract_one should succeed");

    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert!(result.pixel_width > 0 && result.pixel_height > 0);
    assert_eq!(
        result.image_file_name,
        img.file_name().unwrap().to_string_lossy()
    );
    // A scanned book page should yield some text.
    assert!(
        !result.text_blob.trim().is_empty(),
        "expected non-empty text from {}",
        img.display()
    );
    // Line geometry should be normalized (0..=1).
    for line in &result.lines {
        assert!((0.0..=1.0).contains(&line.bounding_box.x));
        assert!((0.0..=1.0).contains(&line.bounding_box.y));
        assert!((0.0..=1.0).contains(&line.confidence));
    }
}

#[test]
fn batch_over_explicit_file_list() {
    if !ready() {
        return;
    }
    let mut pngs: Vec<PathBuf> = std::fs::read_dir(images_dir())
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("png"))
        .collect();
    pngs.sort();
    let subset: Vec<PathBuf> = pngs.into_iter().take(3).collect();
    assert!(!subset.is_empty(), "need sample images");

    let job = OcrJob::new(OcrInput::Files(subset.clone()));
    let results = extract(&job).expect("batch extract should succeed");

    assert_eq!(results.len(), subset.len(), "one result per input image");
    for r in &results {
        assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
    }
}

#[test]
fn batch_over_directory_covers_all_images() {
    if !ready() {
        return;
    }
    let expected = std::fs::read_dir(images_dir())
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            matches!(
                p.extension().and_then(|s| s.to_str()),
                Some("png" | "jpg" | "jpeg" | "tiff" | "tif")
            )
        })
        .count();

    let job = OcrJob::directory(images_dir()).level(RecognitionLevel::Fast);
    let results = extract(&job).expect("directory extract should succeed");

    assert_eq!(
        results.len(),
        expected,
        "directory mode should return one result per image file"
    );
}

#[test]
fn missing_image_reports_per_image_error_not_panic() {
    if !ready() {
        return;
    }
    let bogus = images_dir().join("does-not-exist.png");
    let job = OcrJob::single(&bogus);
    // The helper emits a result object with `error` set; the batch (of one) still succeeds.
    let result = extract_one(&bogus, &job).expect("helper should exit 0 with per-image error");
    assert!(
        result.error.is_some(),
        "expected per-image error for missing file"
    );
}

#[test]
fn missing_binary_yields_spawn_error() {
    // Independent of `ready()` — verifies the error path when the helper is absent.
    let job = OcrJob::single(Path::new("whatever.png"))
        .binary("/nonexistent/path/to/vision-ocr-xyz");
    let err = extract(&job).expect_err("should fail to spawn");
    let msg = err.to_string();
    assert!(msg.contains("spawn"), "unexpected error message: {msg}");
}
