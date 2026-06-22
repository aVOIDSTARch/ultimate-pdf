//! A comprehensive interface for driving the `agent-text-cleanup` CLI.
//!
//! `agent-text-cleanup` is installed as its own binary on this system. Rather than
//! linking its library, this crate treats it as an external command and provides a
//! typed, ergonomic Rust surface over its subcommands so the rest of the project can
//! call it easily (mirroring how `updf-api` shells out to `updf`):
//!
//! ```no_run
//! use updf_agent_cleanup::{AgentTextCleanup, Correct, Normalize};
//!
//! let cli = AgentTextCleanup::new();
//! // Offline regex repair of one file -> stdout (captured).
//! let cleaned = cli.normalize(&Normalize::new("page.ocr.md"))?.into_stdout()?;
//! // Agent correction of several files into a directory.
//! let out = cli.correct(
//!     &Correct::new(["a.md", "b.md"]).output_dir("corrected"),
//! )?;
//! assert!(out.success);
//! # Ok::<(), updf_agent_cleanup::CleanupError>(())
//! ```
//!
//! Every method spawns the binary and captures its output; nothing is reimplemented
//! here, keeping the CLI the single source of truth.

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

/// Default binary name, looked up on `PATH` when nothing more specific is found.
pub const DEFAULT_BIN: &str = "agent-text-cleanup";
/// Environment variable that overrides the binary location.
pub const BIN_ENV: &str = "AGENT_TEXT_CLEANUP_BIN";

/// Locate the `agent-text-cleanup` binary.
///
/// Resolution order: `AGENT_TEXT_CLEANUP_BIN`, then a sibling of the current
/// executable, then `agent-text-cleanup` on `PATH`.
pub fn resolve_binary() -> PathBuf {
    if let Ok(p) = std::env::var(BIN_ENV) {
        return PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(DEFAULT_BIN);
            if candidate.exists() {
                return candidate;
            }
        }
    }
    PathBuf::from(DEFAULT_BIN)
}

/// A handle to the `agent-text-cleanup` CLI.
#[derive(Debug, Clone)]
pub struct AgentTextCleanup {
    bin: PathBuf,
}

impl Default for AgentTextCleanup {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentTextCleanup {
    /// Build a handle, resolving the binary via [`resolve_binary`].
    pub fn new() -> Self {
        Self { bin: resolve_binary() }
    }

    /// Build a handle pointing at a specific binary path.
    pub fn with_binary(bin: impl Into<PathBuf>) -> Self {
        Self { bin: bin.into() }
    }

    /// The binary this handle invokes.
    pub fn binary(&self) -> &Path {
        &self.bin
    }

    /// Return the binary's version string (`agent-text-cleanup --version`).
    pub fn version(&self) -> Result<String, CleanupError> {
        self.run(["--version"])?.into_stdout()
    }

    /// Whether the binary can be run (used for health checks).
    pub fn is_available(&self) -> bool {
        self.version().is_ok()
    }

    /// `normalize` — offline regex repair (no API calls, no cost).
    pub fn normalize(&self, opts: &Normalize) -> Result<CommandOutput, CleanupError> {
        self.capture(&opts.to_args(), None)
    }

    /// `correct` — agent correction of one or more files.
    pub fn correct(&self, opts: &Correct) -> Result<CommandOutput, CleanupError> {
        self.capture(&opts.to_args(), None)
    }

    /// `report` — render a token-usage report from a template.
    pub fn report(&self, opts: &Report) -> Result<CommandOutput, CleanupError> {
        self.capture(&opts.to_args(), None)
    }

    /// `usage` — print token-usage totals.
    pub fn usage(&self, opts: &Usage) -> Result<CommandOutput, CleanupError> {
        self.capture(&opts.to_args(), None)
    }

    /// Run with arbitrary arguments, capturing output.
    pub fn run<I, S>(&self, args: I) -> Result<CommandOutput, CleanupError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let argv: Vec<OsString> = args.into_iter().map(|s| s.as_ref().to_os_string()).collect();
        self.capture(&argv, None)
    }

    /// Run with arbitrary arguments, feeding `input` to the command's stdin.
    ///
    /// Use this for the subcommands' `-` (stdin) inputs, e.g.
    /// `run_with_stdin(Normalize::stdin().to_args(), text)`.
    pub fn run_with_stdin<I, S>(&self, args: I, input: &str) -> Result<CommandOutput, CleanupError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let argv: Vec<OsString> = args.into_iter().map(|s| s.as_ref().to_os_string()).collect();
        self.capture(&argv, Some(input))
    }

    /// Forward arguments to the binary with inherited stdio, returning its exit status.
    ///
    /// This is the streaming path used by the `updf-agent-cleanup` proxy binary; output
    /// goes straight to the terminal rather than being captured.
    pub fn forward<I, S>(&self, args: I) -> Result<ExitStatus, CleanupError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Command::new(&self.bin)
            .args(args)
            .status()
            .map_err(|source| CleanupError::Spawn {
                binary: self.bin.clone(),
                source,
            })
    }

    /// Spawn the binary, optionally piping `stdin`, and capture stdout/stderr/status.
    fn capture(&self, args: &[OsString], stdin: Option<&str>) -> Result<CommandOutput, CleanupError> {
        let spawn_err = |source| CleanupError::Spawn {
            binary: self.bin.clone(),
            source,
        };

        let mut cmd = Command::new(&self.bin);
        cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd.stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });

        let mut child = cmd.spawn().map_err(spawn_err)?;
        if let Some(text) = stdin {
            if let Some(mut handle) = child.stdin.take() {
                handle.write_all(text.as_bytes()).map_err(spawn_err)?;
                // `handle` drops here, closing stdin so the child can finish.
            }
        }

        let output = child.wait_with_output().map_err(spawn_err)?;
        Ok(CommandOutput {
            command: args.iter().map(|a| a.to_string_lossy().into_owned()).collect(),
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Captured result of running an `agent-text-cleanup` subcommand.
#[derive(Debug, Clone)]
pub struct CommandOutput {
    /// The argument vector passed to the binary (for traceability).
    pub command: Vec<String>,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    /// `true` when the command exited zero.
    pub fn ok(&self) -> bool {
        self.success
    }

    /// Return captured stdout on success, or a [`CleanupError::Failed`] on a nonzero exit.
    pub fn into_stdout(self) -> Result<String, CleanupError> {
        if self.success {
            Ok(self.stdout)
        } else {
            Err(CleanupError::Failed {
                command: self.command,
                exit_code: self.exit_code,
                stderr: self.stderr,
            })
        }
    }
}

/// Errors from driving the CLI.
#[derive(Debug)]
pub enum CleanupError {
    /// The binary could not be spawned (missing, not executable, I/O error).
    Spawn {
        binary: PathBuf,
        source: std::io::Error,
    },
    /// The command ran but exited nonzero.
    Failed {
        command: Vec<String>,
        exit_code: Option<i32>,
        stderr: String,
    },
}

impl std::fmt::Display for CleanupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CleanupError::Spawn { binary, source } => write!(
                f,
                "failed to spawn agent-text-cleanup ({}): {source}\n\
                 hint: install the binary or set {BIN_ENV}",
                binary.display()
            ),
            CleanupError::Failed {
                command,
                exit_code,
                stderr,
            } => write!(
                f,
                "agent-text-cleanup {} failed (exit: {exit_code:?}): {}",
                command.join(" "),
                stderr.trim()
            ),
        }
    }
}

impl std::error::Error for CleanupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CleanupError::Spawn { source, .. } => Some(source),
            CleanupError::Failed { .. } => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Typed options for each subcommand
// ---------------------------------------------------------------------------

/// Options for `normalize` (`normalize <input> [--output FILE]`).
#[derive(Debug, Clone, Default)]
pub struct Normalize {
    /// Input file, or `-` for stdin (pair with [`AgentTextCleanup::run_with_stdin`]).
    pub input: PathBuf,
    /// Write the result here instead of stdout.
    pub output: Option<PathBuf>,
}

impl Normalize {
    pub fn new(input: impl Into<PathBuf>) -> Self {
        Self {
            input: input.into(),
            output: None,
        }
    }

    /// Read from stdin (`-`).
    pub fn stdin() -> Self {
        Self::new("-")
    }

    pub fn output(mut self, path: impl Into<PathBuf>) -> Self {
        self.output = Some(path.into());
        self
    }

    /// Build the argument vector for this command.
    pub fn to_args(&self) -> Vec<OsString> {
        let mut args = vec![OsString::from("normalize"), self.input.clone().into_os_string()];
        if let Some(out) = &self.output {
            args.push("--output".into());
            args.push(out.clone().into_os_string());
        }
        args
    }
}

/// Options for `correct`
/// (`correct <inputs...> [--target FILE] [--output-dir DIR] [--stdout] [--model MODEL]`).
#[derive(Debug, Clone, Default)]
pub struct Correct {
    /// Files to correct. A single `-` reads stdin.
    pub inputs: Vec<PathBuf>,
    /// JSON `FormatTarget` file describing the desired output shape.
    pub target: Option<PathBuf>,
    /// Write corrected files into this directory (keeping their base names).
    pub output_dir: Option<PathBuf>,
    /// Print to stdout instead of writing files.
    pub stdout: bool,
    /// Override the model (default: `claude-opus-4-8`).
    pub model: Option<String>,
}

impl Correct {
    pub fn new(inputs: impl IntoIterator<Item = impl Into<PathBuf>>) -> Self {
        Self {
            inputs: inputs.into_iter().map(Into::into).collect(),
            ..Default::default()
        }
    }

    /// Correct a single file.
    pub fn file(input: impl Into<PathBuf>) -> Self {
        Self::new([input.into()])
    }

    pub fn target(mut self, path: impl Into<PathBuf>) -> Self {
        self.target = Some(path.into());
        self
    }

    pub fn output_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.output_dir = Some(dir.into());
        self
    }

    pub fn stdout(mut self, yes: bool) -> Self {
        self.stdout = yes;
        self
    }

    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn to_args(&self) -> Vec<OsString> {
        let mut args = vec![OsString::from("correct")];
        for input in &self.inputs {
            args.push(input.clone().into_os_string());
        }
        if let Some(target) = &self.target {
            args.push("--target".into());
            args.push(target.clone().into_os_string());
        }
        if let Some(dir) = &self.output_dir {
            args.push("--output-dir".into());
            args.push(dir.clone().into_os_string());
        }
        if self.stdout {
            args.push("--stdout".into());
        }
        if let Some(model) = &self.model {
            args.push("--model".into());
            args.push(model.as_str().into());
        }
        args
    }
}

/// Options for `report` (`report [--template FILE] [--log FILE] [--output FILE]`).
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub template: Option<PathBuf>,
    pub log: Option<PathBuf>,
    pub output: Option<PathBuf>,
}

impl Report {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn template(mut self, path: impl Into<PathBuf>) -> Self {
        self.template = Some(path.into());
        self
    }

    pub fn log(mut self, path: impl Into<PathBuf>) -> Self {
        self.log = Some(path.into());
        self
    }

    pub fn output(mut self, path: impl Into<PathBuf>) -> Self {
        self.output = Some(path.into());
        self
    }

    pub fn to_args(&self) -> Vec<OsString> {
        let mut args = vec![OsString::from("report")];
        if let Some(t) = &self.template {
            args.push("--template".into());
            args.push(t.clone().into_os_string());
        }
        if let Some(l) = &self.log {
            args.push("--log".into());
            args.push(l.clone().into_os_string());
        }
        if let Some(o) = &self.output {
            args.push("--output".into());
            args.push(o.clone().into_os_string());
        }
        args
    }
}

/// Options for `usage` (`usage [--log FILE]`).
#[derive(Debug, Clone, Default)]
pub struct Usage {
    pub log: Option<PathBuf>,
}

impl Usage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn log(mut self, path: impl Into<PathBuf>) -> Self {
        self.log = Some(path.into());
        self
    }

    pub fn to_args(&self) -> Vec<OsString> {
        let mut args = vec![OsString::from("usage")];
        if let Some(l) = &self.log {
            args.push("--log".into());
            args.push(l.clone().into_os_string());
        }
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn as_strings(args: &[OsString]) -> Vec<String> {
        args.iter().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn normalize_args_with_output() {
        let args = Normalize::new("page.ocr.md").output("out.md").to_args();
        assert_eq!(as_strings(&args), vec!["normalize", "page.ocr.md", "--output", "out.md"]);
    }

    #[test]
    fn normalize_stdin_args() {
        let args = Normalize::stdin().to_args();
        assert_eq!(as_strings(&args), vec!["normalize", "-"]);
    }

    #[test]
    fn correct_args_full() {
        let args = Correct::new(["a.md", "b.md"])
            .target("t.json")
            .output_dir("corrected")
            .model("claude-sonnet-4-6")
            .to_args();
        assert_eq!(
            as_strings(&args),
            vec![
                "correct",
                "a.md",
                "b.md",
                "--target",
                "t.json",
                "--output-dir",
                "corrected",
                "--model",
                "claude-sonnet-4-6",
            ]
        );
    }

    #[test]
    fn correct_stdout_flag() {
        let args = Correct::file("a.md").stdout(true).to_args();
        assert_eq!(as_strings(&args), vec!["correct", "a.md", "--stdout"]);
    }

    #[test]
    fn report_args() {
        let args = Report::new().template("tpl.md").log("log.json").output("r.md").to_args();
        assert_eq!(
            as_strings(&args),
            vec!["report", "--template", "tpl.md", "--log", "log.json", "--output", "r.md"]
        );
    }

    #[test]
    fn usage_args() {
        assert_eq!(as_strings(&Usage::new().to_args()), vec!["usage"]);
        assert_eq!(
            as_strings(&Usage::new().log("l.json").to_args()),
            vec!["usage", "--log", "l.json"]
        );
    }

    #[test]
    fn with_binary_sets_path() {
        let cli = AgentTextCleanup::with_binary("/opt/bin/agent-text-cleanup");
        assert_eq!(cli.binary(), Path::new("/opt/bin/agent-text-cleanup"));
    }

    #[test]
    fn spawn_failure_is_reported() {
        let cli = AgentTextCleanup::with_binary("/no/such/agent-text-cleanup-binary");
        let err = cli.usage(&Usage::new()).unwrap_err();
        assert!(matches!(err, CleanupError::Spawn { .. }), "{err}");
    }
}
