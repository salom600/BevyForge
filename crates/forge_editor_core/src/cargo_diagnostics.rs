//! Parser for `cargo check --message-format=json` output.
//!
//! The editor runs cargo as a child process and streams its stdout through
//! [`DiagnosticsParser`], which keeps only the human-relevant facts the Rust
//! Compiler panel renders: level, message, file, line — plus the pretty
//! `rendered` block for the code snippet view.

use std::collections::VecDeque;

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Error,
    Warning,
    Note,
    Help,
}

impl DiagnosticLevel {
    pub fn label(self) -> &'static str {
        match self {
            DiagnosticLevel::Error => "error",
            DiagnosticLevel::Warning => "warning",
            DiagnosticLevel::Note => "note",
            DiagnosticLevel::Help => "help",
        }
    }
}

/// One flattened diagnostic the UI can render.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub level: DiagnosticLevel,
    pub message: String,
    pub file: String,
    pub line: u32,
    /// Pretty rustc-rendered snippet (multi-line).
    pub rendered: String,
}

#[derive(Deserialize)]
struct CargoMessage {
    reason: Option<String>,
    message: Option<CargoDiag>,
}

#[derive(Deserialize)]
struct CargoDiag {
    level: Option<String>,
    message: Option<String>,
    rendered: Option<String>,
    spans: Option<Vec<CargoSpan>>,
}

#[derive(Deserialize)]
struct CargoSpan {
    file_name: Option<String>,
    line_start: Option<u32>,
    is_primary: Option<bool>,
}

/// Streaming parser: feed raw stdout lines, receive finished diagnostics.
#[derive(Default)]
pub struct DiagnosticsParser {
    finished: VecDeque<Diagnostic>,
    error_count: u32,
    warning_count: u32,
}

impl DiagnosticsParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one stdout line. JSON compiler messages are parsed; everything
    /// else (cargo progress lines) is ignored.
    pub fn feed_line(&mut self, line: &str) {
        let Ok(msg) = serde_json::from_str::<CargoMessage>(line) else {
            return;
        };
        match msg.reason.as_deref() {
            Some("compiler-message") => {
                if let Some(diag) = msg.message {
                    self.absorb(diag);
                }
            }
            Some("build-finished") => { /* terminal event; counts finalised */ }
            _ => {}
        }
    }

    fn absorb(&mut self, diag: CargoDiag) {
        let level = match diag.level.as_deref().unwrap_or("") {
            "error" | "error: internal compiler error" | "fatal error" => DiagnosticLevel::Error,
            "warning" => DiagnosticLevel::Warning,
            "note" => DiagnosticLevel::Note,
            "help" => DiagnosticLevel::Help,
            _ => return,
        };
        // Prefer the primary span; fall back to the first span with data.
        let (file, line) = diag
            .spans
            .unwrap_or_default()
            .into_iter()
            .find(|s| s.is_primary.unwrap_or(false))
            .map(|s| (s.file_name.unwrap_or_default(), s.line_start.unwrap_or(0)))
            .unwrap_or_else(|| (String::new(), 0));

        if level == DiagnosticLevel::Error {
            self.error_count += 1;
        } else if level == DiagnosticLevel::Warning {
            self.warning_count += 1;
        }

        self.finished.push_back(Diagnostic {
            level,
            message: diag.message.unwrap_or_default(),
            file,
            line,
            rendered: diag.rendered.unwrap_or_default(),
        });
    }

    /// Drain everything parsed so far.
    pub fn take(&mut self) -> Vec<Diagnostic> {
        self.finished.drain(..).collect()
    }

    pub fn error_count(&self) -> u32 {
        self.error_count
    }

    pub fn warning_count(&self) -> u32 {
        self.warning_count
    }

    pub fn reset(&mut self) {
        self.finished.clear();
        self.error_count = 0;
        self.warning_count = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_error_with_span() {
        let line = r#"{"reason":"compiler-message","message":{"level":"error","message":"unused variable: `old_position`","rendered":"error[E0425]: ...","spans":[{"file_name":"crates/forge_scripts/src/lib.rs","line_start":42,"is_primary":true}]}}"#;
        let mut p = DiagnosticsParser::new();
        p.feed_line(line);
        let diags = p.take();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].level, DiagnosticLevel::Error);
        assert!(diags[0].file.ends_with("lib.rs"));
        assert_eq!(diags[0].line, 42);
        assert_eq!(p.error_count(), 1);
    }

    #[test]
    fn ignores_progress_lines() {
        let mut p = DiagnosticsParser::new();
        p.feed_line("Compiling forge_scripts v0.1.0 (/workspace)");
        p.feed_line("not json at all");
        assert!(p.take().is_empty());
    }
}
