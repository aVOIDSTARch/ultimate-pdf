# Plan: Apple Vision OCR pipeline (Swift helper + Rust orchestration)

## Context

The `ultimate-pdf` workspace is a PDF→text pipeline. `updf-pdf-to-image-set` renders PDF
pages to PNGs by shelling out to `gs` (see the builder + `render()` pattern in
`updf-pdf-to-image-set/src/ghostscript.rs`). This crate,
`apple-vision-image-text-extractor`, is the next stage: take a page image and return the
OCR'd text plus useful metadata as a `VisionResults` object.

Apple's Vision OCR framework (`VNRecognizeTextRequest`) is only reachable from
Swift/Obj-C, not Rust. The target system is a Mac Mini M4 on an up-to-date macOS, so
Vision is fully available. We support both **one-off** (single image) and **batch** (many
images) processing.

**Approach:** A small **Swift CLI tool** (`vision-ocr`) runs Vision OCR and emits JSON;
the Rust crate shells out to it and deserializes into `VisionResults` — mirroring how
`ghostscript.rs` drives `gs`. `VisionResults` is **rich** (per-line text/confidence/
bounding boxes + image geometry). Batch input accepts **either a directory or an explicit
list** of image paths.

## Architecture

```
PDF ──(gs)──▶ page-*.png ──▶ [Rust: apple-vision-image-text-extractor]
                                   │ spawns subprocess (like render() spawns gs)
                                   ▼
                          [Swift: vision-ocr CLI]  ── uses VNRecognizeTextRequest
                                   │ prints JSON (array of per-image results) to stdout
                                   ▼
                          Rust parses JSON ──▶ Vec<VisionResults>
```

One subprocess invocation can process an entire batch (the Swift tool loops internally and
emits a JSON array), avoiding per-image process-spawn overhead.

## Part 1 — Swift CLI tool (`vision-ocr`)

Location: `swift/` as a SwiftPM executable package (`Package.swift`,
`Sources/vision-ocr/main.swift`).

- Args: one or more image paths, OR `--dir <path>` to OCR every image in a folder (glob
  `png/jpg/jpeg/tiff/heic`). Flags: `--level accurate|fast` (default `accurate`),
  `--languages en-US,...`, `--no-correction`, `--min-confidence <f>`.
- Per image: load via `CGImageSource`, run a `VNRecognizeTextRequest` through a
  `VNImageRequestHandler` (`recognitionLevel = .accurate`, `usesLanguageCorrection = true`
  by default). For each `VNRecognizedTextObservation`, take `topCandidates(1)` → `.string`
  + `.confidence` and `.boundingBox` (normalized, origin bottom-left). Join lines into
  `textBlob`. Capture pixel width/height, languages, and per-image error.
- Emit a JSON array to stdout (one object per image). Per-image errors go in `error`;
  fatal/usage errors → stderr + nonzero exit.

JSON shape per image:
```json
{
  "imageFileName": "page-1.png",
  "imagePath": "/abs/path/page-1.png",
  "textBlob": "full joined text…",
  "pixelWidth": 1240, "pixelHeight": 1754,
  "recognizedLanguages": ["en-US"],
  "lines": [
    { "text": "…", "confidence": 0.98,
      "boundingBox": { "x": 0.1, "y": 0.8, "width": 0.5, "height": 0.03 } }
  ],
  "error": null
}
```

Build: `swift build -c release` inside `swift/`, producing `swift/.build/release/vision-ocr`.

## Part 2 — Rust crate wiring

- `src/types.rs` — owned, serde-derived `VisionResults` / `TextLine` / `BoundingBox`
  (`#[serde(rename_all = "camelCase")]` to match the Swift JSON).
- `src/vision.rs` — orchestration mirroring `ghostscript.rs`: `OcrJob` builder,
  `OcrInput { Files, Dir }`, `extract()` / `extract_one()`, and a `VisionError`
  (`Spawn` / `Tool { exit_code, stderr }` / `Parse`). Binary path defaults to the release
  build, overridable via env `VISION_OCR_BIN`.
- `src/main.rs` — thin demo driver pointing at the processed images dir.
- `Cargo.toml` — add `serde` (derive) + `serde_json`.

## Verification (end-to-end)

1. `cd swift && swift build -c release`; run on a page PNG, confirm JSON with non-empty
   `textBlob`, sensible confidences, pixel dims; `--dir` returns an array.
2. Rust one-off: `cargo run` prints `VisionResults`; text matches standalone run.
3. Rust batch: `OcrJob` over a directory → `Vec<VisionResults>` length == image count.
4. Error path: missing/non-image file → per-image `error` set (batch) or `VisionError::Tool`
   (fatal); no panic.
5. `cargo build` for the workspace stays green.

## Open considerations
- Local-file OCR from a terminal needs no special entitlement; revisit if sandboxed later.
- Batch is one subprocess call; can parallelize in Swift (`DispatchQueue.concurrentPerform`)
  if throughput matters.
- Vision bounding boxes are normalized, bottom-left origin — documented in `types.rs`.
