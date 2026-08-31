//! Lightweight append-only file logger.
//!
//! Everything startup-related, engine-spawn-related and fatal goes here so a
//! machine where the UI cannot even open still leaves a trace. The file lives
//! next to the executable when writable (portable zips), else in the OS log
//! dir (`%LOCALAPPDATA%\BevyForge` on Windows, `$XDG_DATA_HOME/bevyforge` or
//! `~/.local/share/bevyforge` on Linux).

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

static LOG_FILE: OnceLock<Option<PathBuf>> = OnceLock::new();
static SINK: OnceLock<Mutex<()>> = OnceLock::new();
static SEQ: AtomicU64 = AtomicU64::new(0);

const MAX_BYTES: u64 = 2 * 1024 * 1024;

fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join("bevyforge.log"));
        }
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        out.push(PathBuf::from(local).join("BevyForge").join("bevyforge.log"));
    }
    if let Ok(data) = std::env::var("XDG_DATA_HOME") {
        out.push(PathBuf::from(data).join("bevyforge").join("bevyforge.log"));
    } else if let Ok(home) = std::env::var("HOME") {
        out.push(PathBuf::from(home).join(".local/share/bevyforge/bevyforge.log"));
    } else if let Ok(tmp) = std::env::var("TEMP") {
        out.push(PathBuf::from(tmp).join("bevyforge.log"));
    }
    out
}

fn file() -> Option<&'static PathBuf> {
    LOG_FILE.get_or_init(|| {
        for path in candidates() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            // Rotating check: if the current file grew beyond the cap, rotate.
            if path.exists() {
                if let Ok(meta) = std::fs::metadata(&path) {
                    if meta.len() > MAX_BYTES {
                        let rotated = path.with_extension("log.1");
                        let _ = std::fs::rename(&path, &rotated);
                    }
                }
            }
            match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                Ok(_) => return Some(path),
                Err(_) => continue,
            }
        }
        None
    })
    .as_ref()
}

fn stamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let ms = now.subsec_millis();
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    format!("{h:02}:{m:02}:{s:02}.{ms:03}")
}

/// Force the log file open (called once at startup).
pub fn init() {
    let _ = file();
}

pub fn log(level: &str, message: &str) {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let line = format!("[{} #{seq:05}] {level}: {message}", stamp());
    // stderr for terminal runs; file for everything (best-effort).
    eprintln!("{line}");
    if let Some(path) = file() {
        let lock = SINK.get_or_init(|| Mutex::new(()));
        if let Ok(_guard) = lock.lock() {
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(f, "{line}");
            }
        }
    }
}

pub fn info(message: &str) {
    log("INFO", message);
}

pub fn warn(message: &str) {
    log("WARN", message);
}

pub fn error(message: &str) {
    log("ERROR", message);
}

/// The log file actually in use (for the doctor report).
pub fn active_path() -> Option<&'static PathBuf> {
    file()
}
