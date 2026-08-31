//! Supervisor for the `bevyforge-runtime` child process.
//!
//! The editor locates the runtime binary next to itself (or in a given search
//! dir), spawns it with the project path and a requested port, and watches
//! stdout for the `FORGE_PORT=<n>` line the runtime prints once its IPC
//! listener is bound. stderr is drained into a ring for crash reporting.
//!
//! Windows note: the binary on disk is `bevyforge-runtime.exe`, so every
//! lookup must go through `std::env::consts::EXE_SUFFIX` — `Path::is_file`
//! performs no extension aliasing and a bare `bevyforge-runtime` check always
//! fails on Windows (the "all buttons are dead" bug).
//!
//! GPU note: on machines whose drivers ship broken Vulkan/DX12/GL stacks the
//! runtime dies inside wgpu adapter creation. The supervisor therefore walks a
//! backend fallback chain (default → dx12 → vulkan → gl on Windows) and only
//! fails once every backend has been tried, carrying the engine's own error
//! output back to the editor UI.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};

/// Base name of the runtime binary (without platform suffix).
pub const RUNTIME_BASE: &str = "bevyforge-runtime";

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
    /// GPU backend fallback chain, tried in order. `""` = let wgpu decide.
    /// Each entry is passed to the runtime as `--backend <value>`.
    pub backends: Vec<String>,
}

impl Default for RuntimeSpawner {
    fn default() -> Self {
        Self {
            binary: None,
            port: forge_ipc::DEFAULT_PORT,
            backends: default_backend_chain(),
        }
    }
}

/// Backend fallback chain per platform: the empty spec lets wgpu apply its own
/// selection; the remaining entries force individual backends so a machine
/// with exactly one working driver still boots.
pub fn default_backend_chain() -> Vec<String> {
    if cfg!(target_os = "windows") {
        vec!["".into(), "dx12".into(), "vulkan".into(), "gl".into()]
    } else if cfg!(target_os = "macos") {
        vec!["".into(), "metal".into()]
    } else {
        vec!["".into(), "vulkan".into(), "gl".into()]
    }
}

/// Candidate paths for the runtime binary given the editor's directory and a
/// platform suffix (`""` on Unix, `".exe"` on Windows).
pub fn candidate_paths_with(exe_dir: &Path, suffix: &str) -> Vec<PathBuf> {
    let base = format!("{RUNTIME_BASE}{suffix}");
    vec![
        exe_dir.join(&base),
        exe_dir.join(format!("../{base}")),
        exe_dir.join(format!("../../{base}")),
        PathBuf::from(&base),
    ]
}

impl RuntimeSpawner {
    /// Resolve the runtime binary path.
    ///
    /// Search order:
    /// 1. explicit override
    /// 2. same directory as the current executable (release layout)
    /// 3. one/two levels up (cargo `target/debug` and `target/release/<profile>` layouts)
    /// 4. `bevyforge-runtime<EXE_SUFFIX>` on `$PATH`
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
        for c in candidate_paths_with(&exe_dir, std::env::consts::EXE_SUFFIX) {
            if c.is_file() {
                return Ok(c);
            }
        }
        bail!(
            "{RUNTIME_BASE}{} not found next to the editor or on PATH.\n\
             searched: {}\n\
             fix: extract the FULL BevyForge archive so both executables sit \
             in the same folder (Windows Defender may also have quarantined it).",
            std::env::consts::EXE_SUFFIX,
            candidate_paths_with(&exe_dir, std::env::consts::EXE_SUFFIX)
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }

    /// Spawn the runtime bound to `project_dir` and wait (bounded) for the
    /// `FORGE_PORT` handshake line. Walks the backend fallback chain.
    pub fn spawn(&self, project_dir: &Path, handshake_timeout: Duration) -> Result<RuntimeHandle> {
        let binary = self.find_binary()?;
        let mut failures: Vec<String> = Vec::new();
        for backend in &self.backends {
            match self.try_spawn(&binary, project_dir, backend, handshake_timeout) {
                Ok(handle) => return Ok(handle),
                Err(e) => failures.push(format!(
                    "--backend {}: {e:#}",
                    if backend.is_empty() { "auto" } else { backend }
                )),
            }
        }
        bail!(
            "the render engine failed to start on every GPU backend tried.\n{}",
            failures.join("\n")
        )
    }

    fn try_spawn(
        &self,
        binary: &Path,
        project_dir: &Path,
        backend: &str,
        handshake_timeout: Duration,
    ) -> Result<RuntimeHandle> {
        let mut cmd = Command::new(binary);
        cmd.arg("--project")
            .arg(project_dir)
            .arg("--port")
            .arg(self.port.to_string());
        if !backend.is_empty() {
            cmd.arg("--backend").arg(backend);
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        // The runtime is a console-subsystem binary; without CREATE_NO_WINDOW
        // Windows pops up a black console window for every engine spawn (and
        // closing that console would kill the engine).
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd
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

        // Wait for the port handshake, collecting output for crash reporting.
        let deadline = std::time::Instant::now() + handshake_timeout;
        let mut port = None;
        let mut tail: Vec<String> = Vec::new();
        let mut early_exit: Option<Option<i32>> = None;
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(RuntimeSignal::Port(p)) => {
                    port = Some(p);
                    break;
                }
                Ok(RuntimeSignal::Stdout(l)) => {
                    if tail.len() >= 14 {
                        tail.remove(0);
                    }
                    tail.push(l);
                }
                Ok(RuntimeSignal::Exited(code)) => {
                    early_exit = Some(code);
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        if let Some(port) = port {
            return Ok(RuntimeHandle { child: shared_child, signals: rx, port });
        }

        // Failure paths: kill whatever is left and report with engine output.
        if let Ok(mut c) = shared_child.lock() {
            let _ = c.kill();
        }
        let tail_txt = if tail.is_empty() {
            String::new()
        } else {
            format!("\nengine output:\n  {}", tail.join("\n  "))
        };
        match early_exit {
            Some(code) => bail!(
                "engine crashed during startup (exit code {code:?}){}",
                tail_txt
            ),
            None => bail!(
                "engine did not signal startup within {}s{}",
                handshake_timeout.as_secs(),
                tail_txt
            ),
        }
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

    #[test]
    fn windows_candidates_include_exe_suffix() {
        let dir = PathBuf::from("/tmp/fake-exe-dir");
        let paths = candidate_paths_with(&dir, ".exe");
        assert!(paths.contains(&dir.join("bevyforge-runtime.exe")));
        assert!(paths.contains(&dir.join("../bevyforge-runtime.exe")));
        // bare name must NOT be relied upon when a suffix exists
        assert!(!paths.contains(&dir.join("bevyforge-runtime")));
    }

    #[test]
    fn unix_candidates_have_no_suffix() {
        let dir = PathBuf::from("/tmp/fake-exe-dir");
        let paths = candidate_paths_with(&dir, "");
        assert!(paths.contains(&dir.join("bevyforge-runtime")));
    }

    #[test]
    fn default_chain_covers_platform_backends() {
        let chain = default_backend_chain();
        assert_eq!(chain.first().map(String::as_str), Some(""));
        if cfg!(target_os = "windows") {
            assert!(chain.contains(&"dx12".to_string()));
            assert!(chain.contains(&"vulkan".to_string()));
            assert!(chain.contains(&"gl".to_string()));
        }
    }
}
