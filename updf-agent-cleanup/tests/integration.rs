// End-to-end tests against the real `agent-text-cleanup` binary.
//
// These exercise the library's capturing + stdin paths (not just arg-building).
// They skip gracefully when the binary isn't installed, so they're safe in CI.
// Only the offline `normalize` / `usage` commands are used — no API calls, no cost.

use updf_agent_cleanup::{AgentTextCleanup, Normalize, Usage};

fn cli() -> Option<AgentTextCleanup> {
    let cli = AgentTextCleanup::new();
    cli.is_available().then_some(cli)
}

#[test]
fn normalize_via_stdin_dehyphenates() {
    let Some(cli) = cli() else {
        eprintln!("SKIP: agent-text-cleanup not installed");
        return;
    };

    let out = cli
        .run_with_stdin(Normalize::stdin().to_args(), "atten-\ntion please\n")
        .expect("normalize should run");
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains("attention"),
        "expected dehyphenation, got: {:?}",
        out.stdout
    );
}

#[test]
fn usage_command_succeeds() {
    let Some(cli) = cli() else {
        eprintln!("SKIP: agent-text-cleanup not installed");
        return;
    };

    // Point at a throwaway log path so we don't depend on machine state; a missing
    // log reads as empty rather than erroring.
    let log = std::env::temp_dir().join(format!("uac-usage-{}.json", std::process::id()));
    let out = cli.usage(&Usage::new().log(&log)).expect("usage should run");
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(out.stdout.contains("all time"), "got: {:?}", out.stdout);
}
