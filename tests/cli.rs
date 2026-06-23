// Black-box tests for the `updf` binary: run it as a subprocess and assert on
// stdout/stderr and exit codes. Cargo exposes the built binary via CARGO_BIN_EXE_updf.
//
// Render cases require `gs` and a sample PDF (kept in the updf-pdf-to-image-set crate);
// they skip cleanly when either is missing. Usage/validation cases always run.

use std::path::PathBuf;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_updf");

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("failed to run updf binary")
}

fn gs_available() -> bool {
    Command::new("gs")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Build a minimal valid multi-page PDF (gs reconstructs the xref), so the suite is
/// self-contained and does not depend on sample files living in the repo.
fn make_pdf_bytes(pages: usize) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n");
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 3 + i)).collect();
    out.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    out.extend_from_slice(
        format!(
            "2 0 obj\n<< /Type /Pages /Kids [{}] /Count {} >>\nendobj\n",
            kids.join(" "),
            pages
        )
        .as_bytes(),
    );
    for i in 0..pages {
        out.extend_from_slice(
            format!(
                "{} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents {} 0 R >>\nendobj\n",
                3 + i,
                3 + pages + i
            )
            .as_bytes(),
        );
    }
    for i in 0..pages {
        let stream = b"0 0 100 100 re f";
        out.extend_from_slice(
            format!("{} 0 obj\n<< /Length {} >>\nstream\n", 3 + pages + i, stream.len()).as_bytes(),
        );
        out.extend_from_slice(stream);
        out.extend_from_slice(b"\nendstream\nendobj\n");
    }
    out.extend_from_slice(b"%%EOF\n");
    out
}

fn write_fixture_pdf(path: &std::path::Path, pages: usize) {
    std::fs::write(path, make_pdf_bytes(pages)).unwrap();
}

fn tmp(label: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "updf-cli-{label}-{}-{}",
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

// ---- Usage / validation (always runs) ----

#[test]
fn help_succeeds_and_lists_subcommand() {
    let out = run(&["--help"]);
    assert!(out.status.success(), "exit: {:?}", out.status.code());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("pdf-to-images"), "stdout: {stdout}");
}

#[test]
fn version_succeeds() {
    let out = run(&["--version"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("updf"));
}

#[test]
fn subcommand_help_succeeds() {
    let out = run(&["pdf-to-images", "--help"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("--device"));
    assert!(stdout.contains("--dpi"));
}

#[test]
fn no_subcommand_exits_2() {
    let out = run(&[]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn unknown_subcommand_exits_2() {
    let out = run(&["frobnicate"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn unknown_device_exits_2() {
    let out = run(&["pdf-to-images", "in.pdf", "out", "--device", "webp"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown device"));
}

#[test]
fn zero_dpi_exits_2() {
    let out = run(&["pdf-to-images", "in.pdf", "out", "--dpi", "0"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn first_greater_than_last_exits_2() {
    let out = run(&["pdf-to-images", "in.pdf", "out", "--first", "5", "--last", "2"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--first"));
}

#[test]
fn missing_input_exits_1() {
    let out = run(&["pdf-to-images", "/no/such/file.pdf", "/tmp/whatever"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not found"));
}

#[test]
fn empty_directory_exits_1() {
    let empty = tmp("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let out = run(&[
        "pdf-to-images",
        empty.to_str().unwrap(),
        "/tmp/whatever",
    ]);
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
    let base = tmp("single");
    std::fs::create_dir_all(&base).unwrap();
    let pdf = base.join("doc.pdf");
    write_fixture_pdf(&pdf, 3);
    let out = base.join("out");

    let res = run(&[
        "pdf-to-images",
        pdf.to_str().unwrap(),
        out.to_str().unwrap(),
        "--first",
        "1",
        "--last",
        "2",
    ]);
    assert!(res.status.success(), "stderr: {}", String::from_utf8_lossy(&res.stderr));

    assert_eq!(count_pngs(&out.join("doc")), 2);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn directory_batch_renders_each_pdf() {
    if !gs_available() {
        eprintln!("SKIP: gs not installed");
        return;
    }
    let in_dir = tmp("batch-in");
    let out = tmp("batch-out");
    std::fs::create_dir_all(&in_dir).unwrap();
    write_fixture_pdf(&in_dir.join("one.pdf"), 2);
    write_fixture_pdf(&in_dir.join("two.pdf"), 2);

    let res = run(&[
        "pdf-to-images",
        in_dir.to_str().unwrap(),
        out.to_str().unwrap(),
        "--first",
        "1",
        "--last",
        "1",
        "--device",
        "pnggray",
    ]);
    assert!(res.status.success(), "stderr: {}", String::from_utf8_lossy(&res.stderr));
    assert!(out.join("one/page-1.png").is_file());
    assert!(out.join("two/page-1.png").is_file());
    assert!(String::from_utf8_lossy(&res.stdout).contains("2 ok"));

    let _ = std::fs::remove_dir_all(&in_dir);
    let _ = std::fs::remove_dir_all(&out);
}
