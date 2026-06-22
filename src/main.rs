//! `ultimate-pdf` — the system supervisor and control plane.
//!
//! This binary initializes and controls the whole project. `serve` launches the
//! `updf-api` HTTP server (which is itself a surface over the `updf` CLI) and
//! supervises it, restarting it if it crashes. `health` polls a running system's
//! `/health` endpoint over HTTP so the system's health can be checked remotely.
//!
//! The functional crates are each their own binary; the supervisor locates them
//! next to itself in the build output and wires them together via environment
//! variables (`UPDF_API_ADDR`, `UPDF_BIN`).

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::{Args, Parser, Subcommand};
use tokio::process::Command;

#[derive(Debug, Parser)]
#[command(
    name = "ultimate-pdf",
    version,
    about = "Supervisor & control plane for the Ultimate PDF system",
    long_about = "Initializes and controls the Ultimate PDF system.\n\n\
        `serve` launches and supervises the updf-api HTTP server (a surface over the \
        `updf` CLI); `health` polls a running system's /health endpoint so the system \
        can be monitored remotely.",
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command_>,
}

#[derive(Debug, Subcommand)]
enum Command_ {
    /// Start the system: launch and supervise the updf-api server (default).
    Serve(ServeArgs),
    /// Poll a running system's /health endpoint and print the report.
    Health(HealthArgs),
}

#[derive(Debug, Args)]
struct ServeArgs {
    /// Address the API server should bind to.
    #[arg(long, default_value = "127.0.0.1:8787")]
    addr: String,

    /// Give up after this many rapid (<5s) restarts of the API process.
    #[arg(long, default_value_t = 5)]
    max_restarts: u32,
}

#[derive(Debug, Args)]
struct HealthArgs {
    /// Base URL of a running system (the /health path is appended).
    #[arg(long, default_value = "http://127.0.0.1:8787")]
    url: String,

    /// Request timeout in seconds.
    #[arg(long, default_value_t = 5)]
    timeout: u64,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command_::Serve(ServeArgs {
        addr: "127.0.0.1:8787".to_string(),
        max_restarts: 5,
    })) {
        Command_::Serve(args) => serve(args).await,
        Command_::Health(args) => health(args).await,
    }
}

/// Resolve a sibling binary in the same directory as this executable.
fn sibling_bin(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}

// ---------------------------------------------------------------------------
// serve — launch and supervise the API process
// ---------------------------------------------------------------------------

async fn serve(args: ServeArgs) -> ExitCode {
    let api_bin = sibling_bin("updf-api");
    let updf_bin = sibling_bin("updf");

    if !api_bin.exists() {
        eprintln!(
            "error: updf-api binary not found at {} — run `cargo build` first",
            api_bin.display()
        );
        return ExitCode::FAILURE;
    }

    println!("ultimate-pdf supervisor");
    println!("  api binary : {}", api_bin.display());
    println!("  updf binary: {}", updf_bin.display());
    println!("  bind addr  : {}", args.addr);
    println!("  health     : http://{}/health", args.addr);
    println!("  (press Ctrl-C to shut down)\n");

    let mut rapid_restarts = 0u32;

    loop {
        let started = Instant::now();

        let mut child = match Command::new(&api_bin)
            .env("UPDF_API_ADDR", &args.addr)
            .env("UPDF_BIN", &updf_bin)
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                eprintln!("error: could not start updf-api ({}): {e}", api_bin.display());
                return ExitCode::FAILURE;
            }
        };

        let exited = tokio::select! {
            status = child.wait() => Some(status),
            _ = tokio::signal::ctrl_c() => None,
        };

        match exited {
            // Ctrl-C: shut the child down and stop supervising.
            None => {
                println!("\nshutting down updf-api ...");
                let _ = child.kill().await;
                let _ = child.wait().await;
                return ExitCode::SUCCESS;
            }
            Some(status) => {
                let ran = started.elapsed();
                eprintln!("updf-api exited ({status:?}) after {ran:.1?}");

                if ran < Duration::from_secs(5) {
                    rapid_restarts += 1;
                } else {
                    rapid_restarts = 0;
                }
                if rapid_restarts > args.max_restarts {
                    eprintln!(
                        "giving up after {rapid_restarts} rapid restarts; \
                         check the errors above"
                    );
                    return ExitCode::FAILURE;
                }

                eprintln!(
                    "restarting in 2s (rapid restart {rapid_restarts}/{}) ...",
                    args.max_restarts
                );
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// health — poll a running system remotely
// ---------------------------------------------------------------------------

async fn health(args: HealthArgs) -> ExitCode {
    let url = format!("{}/health", args.url.trim_end_matches('/'));

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(args.timeout))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: could not build HTTP client: {e}");
            return ExitCode::FAILURE;
        }
    };

    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: could not reach {url}: {e}");
            eprintln!("hint: is the system running? start it with `ultimate-pdf serve`");
            return ExitCode::FAILURE;
        }
    };

    let http_ok = resp.status().is_success();
    let body: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: unexpected response from {url}: {e}");
            return ExitCode::FAILURE;
        }
    };

    match serde_json::to_string_pretty(&body) {
        Ok(pretty) => println!("{pretty}"),
        Err(_) => println!("{body}"),
    }

    let status_ok = body.get("status").and_then(|s| s.as_str()) == Some("ok");
    if http_ok && status_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
