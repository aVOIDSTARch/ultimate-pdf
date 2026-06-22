// Thin front-end that exposes the `agent-text-cleanup` crate as a standalone binary.
//
// All behavior lives in the library; this just loads `.env`, parses arguments, and
// dispatches. The same crate is used as the correction stage of the `updf` pipeline.

use agent_text_cleanup::cli::{self, Cli};
use clap::Parser;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    // Load ANTHROPIC_API_KEY from a local .env if present (ignored if absent).
    let _ = dotenvy::dotenv();
    cli::run(Cli::parse()).await
}
