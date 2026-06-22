//! Ultimate PDF — the umbrella crate.
//!
//! This is the project's "main" crate. It owns the cross-stage [`pipeline`]
//! orchestration (PDF → page images → OCR markdown → corrected markdown) and
//! re-exports each stage crate so callers can reach everything through one
//! dependency. The `ultimate-pdf` binary (see `main.rs`) is the supervisor /
//! control plane that launches the system and reports its health; the `updf`
//! CLI drives this `pipeline` module for the actual work.

pub mod pipeline;

// Re-export the stage crates so this umbrella crate is a single entry point.
pub use agent_text_cleanup;
pub use apple_vision_image_text_extractor;
pub use updf_pdf_to_image_set;

// Subprocess interface to the installed `agent-text-cleanup` CLI binary, so the
// project can drive it through the umbrella crate (`ultimate_pdf::updf_agent_cleanup`).
pub use updf_agent_cleanup;
