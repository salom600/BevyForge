//! Supervisor for the `bevyforge-runtime` child process.
//!
//! The editor locates the runtime binary next to itself (or in a given search
//! dir), spawns it with the project path and a requested port, and watches
//! stdout for the `FORGE_PORT=<n>` line the runtime prints once its IPC
//! listener is bound. stderr is drained into a ring for crash reporting.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};

/// A spawned runtime child plus its stdout signal channel.
pub struct RuntimeHandle {
    pub child: Arc<Mutex<Child>>,
    pub signals: mpsc::Receiver<RuntimeSignal>,
    pub port: u16,
}

/// Events observed from the runtime's stdout/stderr pumps.
#[derive(Debug, Clone)]
pub enum RuntimeSignal {
    /// Runtime announced its IPC port (always the first signal).
    Port(u16),
    /// A raw stdout line (diagnostics).
    Stdout(String),
    /// The runtime process exited with this status.
    Exited(Option<i32>),
}

/// Finds and launches `bevyforge-runtime`.
pub struct RuntimeSpawner {
    /// Explicit path to the runtime binary; when `None`, search heuristics run.
    pub binary: Option<PathBuf>,
    pub port: u16,
}

impl Default for RuntimeSpawner {
    fn default() -> Self {
        Self { binary: None, port: forge_ipc::DEFAULT_PORT }
    }
}

impl RuntimeSpawner {
    /// Resolve the runtime binary path.
    ///
    /// Search order:
    /// 1. explicit override
    /// 2. same directory as the current executable
    /// 3. `../bevyforge-runtime` relative to the executable (cargo target dir)
    /// 4. `bevyforge-runtime` on `$PATH`
    pub fn find_binary(&self) -> Result<PathBuf> {
        if let Some(p) = &self.binary {
            if p.is_file() {
                return Ok(p.clone());
            }
            bail!("configured runtime binary not found: {}", p.display());
        }
        let exe_dir = std::env::current_exe()?
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let candidates = [
            exe_dir.join("bevyforge-runtime"),
            exe_dir.join("../bevyforge-runtime"),
            PathBuf::from("bevyforge-runtime"),
        ];
        for c in candidates {
            if c.is_file() {
                return Ok(c);
            }
        }
        bail!("bevyforge-runtime binary not found next to editor or on PATH")
    }

    /// Spawn the runtime bound to `project_dir` and wait (bounded) for the
    /// `FORGE_PORT` handshake line.
    pub fn spawn(&self, project_dir: &Path, handshake_timeout: Duration) -> Result<RuntimeHandle> {
        let binary = self.find_binary()?;
        let mut child = Command::new(&binary)
            .arg("--project")
            .arg(project_dir)
            .arg("--port")
            .arg(self.port.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("spawning {}", binary.display()))?;

        let stdout = child.stdout.take().context("runtime stdout missing")?;
        let (tx, rx) = mpsc::channel();

        // stdout pump — watches for FORGE_PORT and forwards lines.
        let tx_out = tx.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        if let Some(port) = l.strip_prefix("FORGE_PORT=") {
                            if let Ok(p) = port.trim().parse::<u16>() {
                                let _ = tx_out.send(RuntimeSignal::Port(p));
                            }
                        }
                        let _ = tx_out.send(RuntimeSignal::Stdout(l));
                    }
                    Err(_) => break,
                }
            }
        });

        // stderr pump.
        if let Some(stderr) = child.stderr.take() {
            let tx_err = tx.clone();
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    let _ = tx_err.send(RuntimeSignal::Stdout(format!("stderr: {line}")));
                }
            });
        }

        // Exit watcher: poll try_wait on a shared handle.
        let tx_exit = tx.clone();
        let shared_child = Arc::new(Mutex::new(child));
        let watcher_child = shared_child.clone();
        std::thread::spawn(move || loop {
            let status = {
                let Ok(mut c) = watcher_child.lock() else { return };
                match c.try_wait() {
                    Ok(Some(status)) => status.code(),
                    Ok(None) => {
                        drop(c);
                        std::thread::sleep(Duration::from_millis(250));
                        continue;
                    }
                    Err(_) => None,
                }
            };
            let _ = tx_exit.send(RuntimeSignal::Exited(status));
            return;
        });

        // Wait for the port handshake.
        let deadline = std::time::Instant::now() + handshake_timeout;
        let mut port = None;
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(RuntimeSignal::Port(p)) => {
                    port = Some(p);
                    break;
                }
                Ok(_) => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        let Some(port) = port else {
            if let Ok(mut c) = shared_child.lock() {
                let _ = c.kill();
            }
            bail!("runtime did not announce FORGE_PORT within the handshake window");
        };

        Ok(RuntimeHandle { child: shared_child, signals: rx, port })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_binary_is_reported() {
        let spawner = RuntimeSpawner {
            binary: Some(PathBuf::from("/nonexistent/bevyforge-runtime")),
            ..Default::default()
        };
        assert!(spawner.find_binary().is_err());
    }
}
