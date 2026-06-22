// Binary-level (black-box) tests: run the compiled CLI as a subprocess and assert on its
// stdout/stderr and exit codes. Cargo exposes the built binary path via
// CARGO_BIN_EXE_<name>, so these need no library target.
//
// Render-dependent cases require `gs` and a sample PDF in the repo; they skip cleanly when
// either is missing. Argument/validation cases always run.

use std::path::PathBuf;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_updf-pdf-to-image-set");

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("failed to run updf-pdf-to-image-set binary")
}

fn gs_available() -> bool {
    Command::new("gs")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn sample_pdf() -> Option<PathBuf> {
    [
        concat!(env!("CARGO_MANIFEST_DIR"), "/repaired.pdf"),
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../AlanWatts-EssentialAlanWatts(AlanWatts)(Z-Library) 2.pdf"
        ),
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
}

fn tmp(label: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "updf-cli-{label}-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&p);
    p
}

fn count_pngs(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("png"))
                .count()
        })
        .unwrap_or(0)
}

// ---- Argument / validation behavior (always runs) ----

#[test]
fn help_prints_usage_and_exits_zero() {
    let out = run(&["--help"]);
    assert!(out.status.success(), "exit: {:?}", out.status.code());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("usage:"), "stdout was: {stdout}");
    assert!(stdout.contains("--device"));
}

#[test]
fn unknown_flag_exits_with_code_2() {
    let out = run(&["--bogus"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("unknown flag"), "stderr was: {stderr}");
}

#[test]
fn invalid_dpi_exits_with_code_2() {
    let out = run(&["in.pdf", "out", "--dpi", "abc"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("invalid --dpi"));
}

#[test]
fn unknown_device_exits_with_code_2() {
    let out = run(&["in.pdf", "out", "--device", "webp"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown --device"));
}

#[test]
fn too_many_positionals_exits_with_code_2() {
    let out = run(&["a", "b", "c"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("too many"));
}

#[test]
fn missing_input_path_exits_failure() {
    let out = run(&["/no/such/path.pdf", "/tmp/whatever"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not found"));
}

#[test]
fn directory_with_no_pdfs_exits_failure() {
    let empty = tmp("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let out = run(&[empty.to_str().unwrap(), "/tmp/whatever"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no .pdf"));
    let _ = std::fs::remove_dir_all(&empty);
}

// ---- End-to-end rendering (skips without gs + sample pdf) ----

#[test]
fn single_file_renders_into_named_dir() {
    if !gs_available() {
        eprintln!("SKIP: gs not installed");
        return;
    }
    let Some(pdf) = sample_pdf() else {
        eprintln!("SKIP: no sample pdf");
        return;
    };
    let out = tmp("single");
    let status = run(&[
        pdf.to_str().unwrap(),
        out.to_str().unwrap(),
        "--first",
        "1",
        "--last",
        "2",
    ]);
    assert!(status.status.success(), "stderr: {}", String::from_utf8_lossy(&status.stderr));

    let stem = pdf.file_stem().unwrap().to_str().unwrap();
    let dir = out.join(stem);
    assert_eq!(count_pngs(&dir), 2, "expected 2 page images");
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn directory_batch_renders_each_pdf() {
    if !gs_available() {
        eprintln!("SKIP: gs not installed");
        return;
    }
    let Some(pdf) = sample_pdf() else {
        eprintln!("SKIP: no sample pdf");
        return;
    };
    let in_dir = tmp("batch-in");
    let out = tmp("batch-out");
    std::fs::create_dir_all(&in_dir).unwrap();
    std::fs::copy(&pdf, in_dir.join("one.pdf")).unwrap();
    std::fs::copy(&pdf, in_dir.join("two.pdf")).unwrap();

    let status = run(&[
        in_dir.to_str().unwrap(),
        out.to_str().unwrap(),
        "--first",
        "1",
        "--last",
        "1",
    ]);
    assert!(status.status.success(), "stderr: {}", String::from_utf8_lossy(&status.stderr));
    assert!(out.join("one/page-1.png").is_file());
    assert!(out.join("two/page-1.png").is_file());

    let stdout = String::from_utf8_lossy(&status.stdout);
    assert!(stdout.contains("2 ok"), "summary missing: {stdout}");

    let _ = std::fs::remove_dir_all(&in_dir);
    let _ = std::fs::remove_dir_all(&out);
}
