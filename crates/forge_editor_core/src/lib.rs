//! # forge_editor_core
//!
//! Editor-side brain of BevyForge: everything the UI layer should not have to
//! care about — the on-disk project model, the undo/redo command history, the
//! `cargo check` diagnostics parser that feeds the Rust Compiler panel, and the
//! runtime-process supervisor that spawns/reaps `bevyforge-runtime`.

pub mod cargo_diagnostics;
pub mod project;
pub mod runtime_process;
pub mod undo;

pub use cargo_diagnostics::{Diagnostic, DiagnosticLevel, DiagnosticsParser};
pub use project::Project;
pub use runtime_process::{RuntimeHandle, RuntimeSignal, RuntimeSpawner};
pub use undo::{UndoEntry, UndoStack};
