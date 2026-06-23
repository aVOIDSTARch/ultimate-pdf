//! HTTP surface over the `updf` command-line tool (the `updf serve` subcommand).
//!
//! By design this is a *surface for the CLI*, not a second implementation: every
//! endpoint spawns the `updf` binary as a subprocess and returns its stdout /
//! stderr / exit status as JSON. Since `updf` is now the only binary, the server
//! simply spawns itself — one source of truth (the CLI), drivable and
//! health-checked remotely.
//!
//! Endpoints:
//! * `GET  /`               — service info and the endpoint list.
//! * `GET  /health`         — system health (is `updf` runnable? is `gs` present?).
//! * `POST /pdf-to-images`  — render a PDF (or directory) to page images.
//! * `POST /ocr`            — OCR a directory of page images to markdown.
//! * `POST /correct`        — correct an OCR markdown file with the agent.
//! * `POST /pipeline`       — the full PDF → images → OCR → (optional) correct flow.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

/// Server configuration for `updf serve`.
#[derive(Debug, Clone)]
pub struct Config {
    /// Address to bind the HTTP server to.
    pub addr: SocketAddr,
    /// Path to the `updf` CLI binary this API surfaces (normally this binary itself).
    pub updf_bin: PathBuf,
}

/// Locate the `updf` binary: `UPDF_BIN`, else a sibling of the current exe, else `updf` on PATH.
pub fn resolve_updf_bin() -> PathBuf {
    if let Ok(p) = std::env::var("UPDF_BIN") {
        return PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("updf");
            if candidate.exists() {
                return candidate;
            }
        }
    }
    PathBuf::from("updf")
}

struct AppState {
    updf_bin: PathBuf,
    started: Instant,
}

/// Build the router, wired to spawn the given `updf` binary.
pub fn router(updf_bin: PathBuf) -> Router {
    let state = Arc::new(AppState {
        updf_bin,
        started: Instant::now(),
    });
    Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/pdf-to-images", post(pdf_to_images))
        .route("/ocr", post(ocr))
        .route("/correct", post(correct))
        .route("/pipeline", post(pipeline))
        .with_state(state)
}

/// Bind and serve until the process is stopped.
pub async fn serve(config: Config) -> std::io::Result<()> {
    let app = router(config.updf_bin);
    let listener = tokio::net::TcpListener::bind(config.addr).await?;
    axum::serve(listener, app).await
}

// ---------------------------------------------------------------------------
// Info + health
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct Index {
    service: &'static str,
    description: &'static str,
    endpoints: Vec<&'static str>,
}

async fn index() -> Json<Index> {
    Json(Index {
        service: "updf",
        description: "HTTP surface over the updf CLI",
        endpoints: vec![
            "GET /health",
            "POST /pdf-to-images",
            "POST /ocr",
            "POST /correct",
            "POST /pipeline",
        ],
    })
}

/// Availability of an external tool the system relies on.
#[derive(Serialize)]
struct ToolStatus {
    available: bool,
    detail: String,
}

#[derive(Serialize)]
struct HealthReport {
    /// `ok` when the `updf` CLI is runnable, otherwise `degraded`.
    status: &'static str,
    uptime_seconds: u64,
    updf_cli: ToolStatus,
    ghostscript: ToolStatus,
    vision_ocr: ToolStatus,
}

async fn health(State(st): State<Arc<AppState>>) -> (StatusCode, Json<HealthReport>) {
    let updf_cli = check_tool(&st.updf_bin, &["--version"]).await;
    let ghostscript = check_tool(Path::new("gs"), &["--version"]).await;
    let vision_ocr = check_vision_ocr().await;

    // The system is healthy as long as its control surface (the CLI) works; the
    // other tools are reported but only matter once a job actually needs them.
    let healthy = updf_cli.available;
    let report = HealthReport {
        status: if healthy { "ok" } else { "degraded" },
        uptime_seconds: st.started.elapsed().as_secs(),
        updf_cli,
        ghostscript,
        vision_ocr,
    };
    let code = if healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (code, Json(report))
}

/// Probe a tool by running it and capturing the first line of stdout as a version string.
async fn check_tool(bin: &Path, args: &[&str]) -> ToolStatus {
    match Command::new(bin).args(args).output().await {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            ToolStatus {
                available: true,
                detail: text.lines().next().unwrap_or("").trim().to_string(),
            }
        }
        Ok(out) => ToolStatus {
            available: false,
            detail: format!("exited with status {:?}", out.status.code()),
        },
        Err(e) => ToolStatus {
            available: false,
            detail: e.to_string(),
        },
    }
}

/// The OCR helper isn't on PATH; report it only when `VISION_OCR_BIN` points at a real file.
async fn check_vision_ocr() -> ToolStatus {
    match std::env::var("VISION_OCR_BIN") {
        Ok(p) if Path::new(&p).exists() => ToolStatus {
            available: true,
            detail: p,
        },
        Ok(p) => ToolStatus {
            available: false,
            detail: format!("VISION_OCR_BIN set but not found: {p}"),
        },
        Err(_) => ToolStatus {
            available: false,
            detail: "not configured (set VISION_OCR_BIN to health-check the OCR helper)".to_string(),
        },
    }
}

// ---------------------------------------------------------------------------
// Command endpoints
// ---------------------------------------------------------------------------

/// The result of running an `updf` subcommand.
#[derive(Serialize)]
struct RunResponse {
    /// The full argument vector passed to `updf`, for traceability.
    command: Vec<String>,
    success: bool,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

type ApiResult = Result<Json<RunResponse>, (StatusCode, Json<ErrorBody>)>;

/// Spawn `updf` with `args`, returning its captured output (or a 502 if it can't be spawned).
async fn run_updf(bin: &Path, args: Vec<String>) -> ApiResult {
    match Command::new(bin).args(&args).output().await {
        Ok(out) => Ok(Json(RunResponse {
            command: args,
            success: out.status.success(),
            exit_code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })),
        Err(e) => Err((
            StatusCode::BAD_GATEWAY,
            Json(ErrorBody {
                error: format!("failed to run updf ({}): {e}", bin.display()),
            }),
        )),
    }
}

fn bad_request(msg: impl Into<String>) -> (StatusCode, Json<ErrorBody>) {
    (StatusCode::BAD_REQUEST, Json(ErrorBody { error: msg.into() }))
}

/// Append `input` (and optionally `output`) as positional args. `output` requires `input`.
fn push_positionals(
    args: &mut Vec<String>,
    input: Option<String>,
    output: Option<String>,
) -> Result<(), (StatusCode, Json<ErrorBody>)> {
    match (input, output) {
        (Some(i), Some(o)) => {
            args.push(i);
            args.push(o);
        }
        (Some(i), None) => args.push(i),
        (None, None) => {}
        (None, Some(_)) => return Err(bad_request("`output` requires `input`")),
    }
    Ok(())
}

/// Render knobs shared by `pdf-to-images` and `pipeline`.
fn push_render_flags(args: &mut Vec<String>, r: &RenderFields) {
    if let Some(dpi) = r.dpi {
        args.push("--dpi".into());
        args.push(dpi.to_string());
    }
    if let Some(device) = &r.device {
        args.push("--device".into());
        args.push(device.clone());
    }
    if let Some(first) = r.first {
        args.push("--first".into());
        args.push(first.to_string());
    }
    if let Some(last) = r.last {
        args.push("--last".into());
        args.push(last.to_string());
    }
}

/// OCR knobs shared by `ocr` and `pipeline`.
fn push_ocr_flags(args: &mut Vec<String>, o: &OcrFields) {
    if let Some(level) = &o.level {
        args.push("--level".into());
        args.push(level.clone());
    }
    if let Some(langs) = &o.languages {
        args.push("--languages".into());
        args.push(langs.clone());
    }
    if let Some(mc) = o.min_confidence {
        args.push("--min-confidence".into());
        args.push(mc.to_string());
    }
    if o.no_language_correction.unwrap_or(false) {
        args.push("--no-language-correction".into());
    }
}

/// Correction knobs shared by `correct` and `pipeline`.
fn push_correct_flags(args: &mut Vec<String>, c: &CorrectFields) {
    if let Some(model) = &c.model {
        args.push("--model".into());
        args.push(model.clone());
    }
    if let Some(target) = &c.target {
        args.push("--target".into());
        args.push(target.clone());
    }
    if c.no_normalize.unwrap_or(false) {
        args.push("--no-normalize".into());
    }
}

#[derive(Default, Deserialize)]
struct RenderFields {
    dpi: Option<u32>,
    device: Option<String>,
    first: Option<u32>,
    last: Option<u32>,
}

#[derive(Default, Deserialize)]
struct OcrFields {
    level: Option<String>,
    languages: Option<String>,
    min_confidence: Option<f32>,
    no_language_correction: Option<bool>,
}

#[derive(Default, Deserialize)]
struct CorrectFields {
    model: Option<String>,
    target: Option<String>,
    no_normalize: Option<bool>,
}

#[derive(Deserialize)]
struct PdfToImagesRequest {
    input: Option<String>,
    output: Option<String>,
    #[serde(flatten)]
    render: RenderFields,
}

async fn pdf_to_images(
    State(st): State<Arc<AppState>>,
    Json(req): Json<PdfToImagesRequest>,
) -> ApiResult {
    let mut args = vec!["pdf-to-images".to_string()];
    push_positionals(&mut args, req.input, req.output)?;
    push_render_flags(&mut args, &req.render);
    run_updf(&st.updf_bin, args).await
}

#[derive(Deserialize)]
struct OcrRequest {
    /// Directory of page images (required).
    images_dir: String,
    output: Option<String>,
    #[serde(flatten)]
    ocr: OcrFields,
}

async fn ocr(State(st): State<Arc<AppState>>, Json(req): Json<OcrRequest>) -> ApiResult {
    let mut args = vec!["ocr".to_string(), req.images_dir];
    if let Some(output) = req.output {
        args.push(output);
    }
    push_ocr_flags(&mut args, &req.ocr);
    run_updf(&st.updf_bin, args).await
}

#[derive(Deserialize)]
struct CorrectRequest {
    /// Input markdown file (required).
    input: String,
    output: Option<String>,
    #[serde(flatten)]
    correct: CorrectFields,
}

async fn correct(State(st): State<Arc<AppState>>, Json(req): Json<CorrectRequest>) -> ApiResult {
    let mut args = vec!["correct".to_string(), req.input];
    if let Some(output) = req.output {
        args.push(output);
    }
    push_correct_flags(&mut args, &req.correct);
    run_updf(&st.updf_bin, args).await
}

#[derive(Deserialize)]
struct PipelineRequest {
    input: Option<String>,
    output: Option<String>,
    /// Run the (paid) Claude correction stage.
    correct: Option<bool>,
    #[serde(flatten)]
    render: RenderFields,
    #[serde(flatten)]
    ocr: OcrFields,
    #[serde(flatten)]
    correct_opts: CorrectFields,
}

async fn pipeline(State(st): State<Arc<AppState>>, Json(req): Json<PipelineRequest>) -> ApiResult {
    let mut args = vec!["pipeline".to_string()];
    push_positionals(&mut args, req.input, req.output)?;
    push_render_flags(&mut args, &req.render);
    if req.correct.unwrap_or(false) {
        args.push("--correct".into());
    }
    push_ocr_flags(&mut args, &req.ocr);
    push_correct_flags(&mut args, &req.correct_opts);
    run_updf(&st.updf_bin, args).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positionals_require_input_for_output() {
        let mut args = vec!["pipeline".to_string()];
        let err = push_positionals(&mut args, None, Some("out".into()));
        assert!(err.is_err());

        let mut args = vec!["pipeline".to_string()];
        push_positionals(&mut args, Some("in.pdf".into()), Some("out".into())).unwrap();
        assert_eq!(args, vec!["pipeline", "in.pdf", "out"]);
    }

    #[test]
    fn pipeline_request_builds_expected_args() {
        let req = PipelineRequest {
            input: Some("book.pdf".into()),
            output: Some("out".into()),
            correct: Some(true),
            render: RenderFields {
                dpi: Some(200),
                device: Some("pnggray".into()),
                first: Some(1),
                last: Some(3),
            },
            ocr: OcrFields {
                level: Some("fast".into()),
                languages: None,
                min_confidence: None,
                no_language_correction: Some(true),
            },
            correct_opts: CorrectFields {
                model: Some("claude-opus-4-8".into()),
                target: None,
                no_normalize: None,
            },
        };

        let mut args = vec!["pipeline".to_string()];
        push_positionals(&mut args, req.input, req.output).unwrap();
        push_render_flags(&mut args, &req.render);
        if req.correct.unwrap_or(false) {
            args.push("--correct".into());
        }
        push_ocr_flags(&mut args, &req.ocr);
        push_correct_flags(&mut args, &req.correct_opts);

        assert_eq!(
            args,
            vec![
                "pipeline",
                "book.pdf",
                "out",
                "--dpi",
                "200",
                "--device",
                "pnggray",
                "--first",
                "1",
                "--last",
                "3",
                "--correct",
                "--level",
                "fast",
                "--no-language-correction",
                "--model",
                "claude-opus-4-8",
            ]
        );
    }
}
