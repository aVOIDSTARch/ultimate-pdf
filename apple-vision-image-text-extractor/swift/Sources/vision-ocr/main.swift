// vision-ocr — Apple Vision OCR helper for the ultimate-pdf pipeline.
//
// Reads one or more image paths (or every image in a directory via --dir), runs
// VNRecognizeTextRequest on each, and prints a JSON array of results to stdout.
// One process invocation handles a whole batch. Per-image failures are captured in the
// `error` field so a single bad image never aborts the batch; usage/fatal errors go to
// stderr with a nonzero exit code.

import Foundation
import Vision
import ImageIO
import CoreGraphics

// MARK: - JSON output model

struct BoundingBox: Codable {
    let x: Double
    let y: Double
    let width: Double
    let height: Double
}

struct TextLine: Codable {
    let text: String
    let confidence: Double
    let boundingBox: BoundingBox
}

struct VisionResult: Codable {
    let imageFileName: String
    let imagePath: String
    let textBlob: String
    let pixelWidth: Int
    let pixelHeight: Int
    let recognizedLanguages: [String]
    let lines: [TextLine]
    let error: String?
}

// MARK: - Options

struct Options {
    var paths: [String] = []
    var dir: String? = nil
    var level: VNRequestTextRecognitionLevel = .accurate
    var languages: [String] = ["en-US"]
    var useLanguageCorrection: Bool = true
    var minConfidence: Double = 0.0
}

let imageExtensions: Set<String> = ["png", "jpg", "jpeg", "tiff", "tif", "heic", "bmp", "gif"]

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("vision-ocr: \(message)\n".utf8))
    exit(1)
}

func parseArgs(_ argv: [String]) -> Options {
    var opts = Options()
    var i = 0
    let args = Array(argv.dropFirst())
    while i < args.count {
        let arg = args[i]
        switch arg {
        case "--dir":
            i += 1
            guard i < args.count else { fail("--dir requires a path") }
            opts.dir = args[i]
        case "--level":
            i += 1
            guard i < args.count else { fail("--level requires accurate|fast") }
            switch args[i] {
            case "accurate": opts.level = .accurate
            case "fast": opts.level = .fast
            default: fail("--level must be 'accurate' or 'fast'")
            }
        case "--languages":
            i += 1
            guard i < args.count else { fail("--languages requires a comma-separated list") }
            opts.languages = args[i].split(separator: ",").map {
                $0.trimmingCharacters(in: .whitespaces)
            }
        case "--no-correction":
            opts.useLanguageCorrection = false
        case "--min-confidence":
            i += 1
            guard i < args.count, let v = Double(args[i]) else {
                fail("--min-confidence requires a number")
            }
            opts.minConfidence = v
        case "-h", "--help":
            print("""
            usage: vision-ocr [IMAGE ...] [--dir DIR] [options]
              --dir DIR             OCR every image in DIR
              --level accurate|fast recognition level (default: accurate)
              --languages a,b,c     recognition languages (default: en-US)
              --no-correction       disable language correction
              --min-confidence N    drop lines below confidence N (0..1)
            Prints a JSON array of per-image results to stdout.
            """)
            exit(0)
        default:
            opts.paths.append(arg)
        }
        i += 1
    }
    return opts
}

// MARK: - Image enumeration

func collectImages(_ opts: Options) -> [String] {
    var result = opts.paths
    if let dir = opts.dir {
        let fm = FileManager.default
        guard let entries = try? fm.contentsOfDirectory(atPath: dir) else {
            fail("could not read directory: \(dir)")
        }
        let matched = entries
            .filter { imageExtensions.contains(($0 as NSString).pathExtension.lowercased()) }
            .sorted()
            .map { (dir as NSString).appendingPathComponent($0) }
        result.append(contentsOf: matched)
    }
    return result
}

// MARK: - OCR

func loadCGImage(_ path: String) -> CGImage? {
    let url = URL(fileURLWithPath: path)
    guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
    return CGImageSourceCreateImageAtIndex(source, 0, nil)
}

func ocr(path: String, opts: Options) -> VisionResult {
    let fileName = (path as NSString).lastPathComponent
    let absPath = URL(fileURLWithPath: path).standardizedFileURL.path

    guard let cgImage = loadCGImage(path) else {
        return VisionResult(
            imageFileName: fileName, imagePath: absPath, textBlob: "",
            pixelWidth: 0, pixelHeight: 0, recognizedLanguages: opts.languages,
            lines: [], error: "could not load image")
    }

    let request = VNRecognizeTextRequest()
    request.recognitionLevel = opts.level
    request.usesLanguageCorrection = opts.useLanguageCorrection
    if !opts.languages.isEmpty {
        request.recognitionLanguages = opts.languages
    }

    let handler = VNImageRequestHandler(cgImage: cgImage, options: [:])
    do {
        try handler.perform([request])
    } catch {
        return VisionResult(
            imageFileName: fileName, imagePath: absPath, textBlob: "",
            pixelWidth: cgImage.width, pixelHeight: cgImage.height,
            recognizedLanguages: opts.languages, lines: [],
            error: "recognition failed: \(error.localizedDescription)")
    }

    let observations = request.results ?? []
    var lines: [TextLine] = []
    for obs in observations {
        guard let candidate = obs.topCandidates(1).first else { continue }
        let confidence = Double(candidate.confidence)
        if confidence < opts.minConfidence { continue }
        let box = obs.boundingBox // normalized, origin bottom-left
        lines.append(TextLine(
            text: candidate.string,
            confidence: confidence,
            boundingBox: BoundingBox(
                x: Double(box.origin.x), y: Double(box.origin.y),
                width: Double(box.size.width), height: Double(box.size.height))))
    }

    let textBlob = lines.map { $0.text }.joined(separator: "\n")

    return VisionResult(
        imageFileName: fileName, imagePath: absPath, textBlob: textBlob,
        pixelWidth: cgImage.width, pixelHeight: cgImage.height,
        recognizedLanguages: opts.languages, lines: lines, error: nil)
}

// MARK: - Main

let opts = parseArgs(CommandLine.arguments)
let images = collectImages(opts)
if images.isEmpty {
    fail("no images supplied (pass paths or --dir DIR; use --help for usage)")
}

let results = images.map { ocr(path: $0, opts: opts) }

let encoder = JSONEncoder()
encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
do {
    let data = try encoder.encode(results)
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data("\n".utf8))
} catch {
    fail("failed to encode JSON: \(error.localizedDescription)")
}
