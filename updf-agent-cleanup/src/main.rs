// Thin proxy binary: forward every argument to the external `agent-text-cleanup`
// CLI and propagate its exit status. The reusable surface is the library in
// `lib.rs`; this just lets the project run the tool by name through one resolver
// (AGENT_TEXT_CLEANUP_BIN / sibling exe / PATH).

use std::process::ExitCode;

use updf_agent_cleanup::{AgentTextCleanup, BIN_ENV};

fn main() -> ExitCode {
    let cli = AgentTextCleanup::new();
    match cli.forward(std::env::args_os().skip(1)) {
        Ok(status) => match status.code() {
            Some(0) => ExitCode::SUCCESS,
            Some(code) => ExitCode::from(code as u8),
            None => ExitCode::FAILURE, // terminated by a signal
        },
        Err(e) => {
            eprintln!("updf-agent-cleanup: {e}");
            eprintln!("hint: install `agent-text-cleanup`, or set {BIN_ENV} to its path");
            ExitCode::FAILURE
        }
    }
}
