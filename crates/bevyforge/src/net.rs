//! Editor↔runtime networking: spawns or attaches to the runtime process,
//! pumps commands and events on background threads.

use crossbeam_channel::{Receiver, Sender};
use forge_ipc::{EditorToRuntime, Message, RuntimeToEditor};

use forge_editor_core::{RuntimeHandle, RuntimeSpawner};

/// Handle to the network layer owned by the app.
#[allow(dead_code)]
pub struct Net {
    pub cmd_tx: Sender<EditorToRuntime>,
    pub events: Receiver<NetEvent>,
    pub runtime: Option<std::sync::Arc<std::sync::Mutex<std::process::Child>>>,
    pub attached: bool,
}

/// Events surfaced to the UI thread.
#[derive(Debug)]
pub enum NetEvent {
    Message(RuntimeToEditor),
    Connected,
    Disconnected(String),
    RuntimeExited(Option<i32>),
    RuntimeStdout(String),
}

impl Net {
    /// UI-only mode: no runtime at all (panels show the offline state).
    pub fn offline() -> Net {
        let (cmd_tx, _cmd_rx) = crossbeam_channel::unbounded();
        let (_evt_tx, events) = crossbeam_channel::unbounded();
        Net { cmd_tx, events, runtime: None, attached: false }
    }

    /// Spawn the runtime child process and connect to its IPC port.
    pub fn spawn_runtime(project_dir: &std::path::Path, port: u16) -> anyhow::Result<Net> {
        let spawner = RuntimeSpawner { binary: None, port };
        let handle: RuntimeHandle = spawner.spawn(project_dir, std::time::Duration::from_secs(20))?;
        let child = handle.child;
        let port = handle.port;
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<EditorToRuntime>();
        let (evt_tx, evt_rx) = crossbeam_channel::unbounded::<NetEvent>();

        // Runtime process signal pump (stdout/exit) — owns the receiver.
        {
            let signals = handle.signals;
            let evt_tx = evt_tx.clone();
            std::thread::spawn(move || {
                while let Ok(sig) = signals.recv() {
                    let evt = match sig {
                        forge_editor_core::RuntimeSignal::Port(_) => continue,
                        forge_editor_core::RuntimeSignal::Stdout(line) => {
                            NetEvent::RuntimeStdout(line)
                        }
                        forge_editor_core::RuntimeSignal::Exited(code) => {
                            NetEvent::RuntimeExited(code)
                        }
                    };
                    if evt_tx.send(evt).is_err() {
                        return;
                    }
                }
            });
        }

        // Connect with retries while the runtime finishes booting.
        std::thread::spawn(move || {
            let mut conn = None;
            for _ in 0..60 {
                match forge_ipc::connect(port, std::time::Duration::from_millis(500)) {
                    Ok(c) => {
                        conn = Some(c);
                        break;
                    }
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(200)),
                }
            }
            let Some(mut conn) = conn else {
                let _ = evt_tx.send(NetEvent::Disconnected(format!(
                    "could not reach runtime on port {port}"
                )));
                return;
            };
            let _ = evt_tx.send(NetEvent::Connected);

            // Outbound pump — needs its own socket handle.
            let write_stream = match conn.clone_stream() {
                Ok(s) => s,
                Err(e) => {
                    let _ = evt_tx.send(NetEvent::Disconnected(format!("socket split failed: {e}")));
                    return;
                }
            };
            {
                let evt_tx = evt_tx.clone();
                std::thread::spawn(move || {
                    let mut write_stream = write_stream;
                    for cmd in cmd_rx {
                        if forge_ipc::send_on_stream(&mut write_stream, &Message::ToRuntime(cmd))
                            .is_err()
                        {
                            let _ = evt_tx.send(NetEvent::Disconnected("socket write failed".into()));
                            return;
                        }
                    }
                });
            }

            // Inbound pump.
            loop {
                match conn.recv() {
                    Ok(Message::ToEditor(evt)) => {
                        if evt_tx.send(NetEvent::Message(evt)).is_err() {
                            return;
                        }
                    }
                    Ok(Message::ToRuntime(_)) => continue,
                    Err(e) => {
                        let _ = evt_tx.send(NetEvent::Disconnected(format!("runtime link lost: {e}")));
                        return;
                    }
                }
            }
        });

        Ok(Net { cmd_tx, events: evt_rx, runtime: Some(child), attached: false })
    }

    /// Attach to an already-running runtime (editor restart scenario).
    pub fn attach(port: u16) -> anyhow::Result<Net> {
        let mut conn = forge_ipc::connect(port, std::time::Duration::from_secs(3))?;
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<EditorToRuntime>();
        let (evt_tx, evt_rx) = crossbeam_channel::unbounded::<NetEvent>();
        let _ = evt_tx.send(NetEvent::Connected);

        let write_stream = conn.clone_stream()?;
        std::thread::spawn(move || {
            let mut write_stream = write_stream;
            for cmd in cmd_rx {
                if forge_ipc::send_on_stream(&mut write_stream, &Message::ToRuntime(cmd)).is_err() {
                    return;
                }
            }
        });
        std::thread::spawn(move || {
            loop {
                match conn.recv() {
                    Ok(Message::ToEditor(evt)) => {
                        if evt_tx.send(NetEvent::Message(evt)).is_err() {
                            return;
                        }
                    }
                    Ok(Message::ToRuntime(_)) => continue,
                    Err(e) => {
                        let _ = evt_tx.send(NetEvent::Disconnected(format!("link lost: {e}")));
                        return;
                    }
                }
            }
        });
        Ok(Net { cmd_tx, events: evt_rx, runtime: None, attached: true })
    }

    /// Non-blocking drain of everything that arrived since last frame.
    pub fn drain(&self, sink: &mut Vec<NetEvent>, max: usize) {
        let mut n = 0;
        while n < max {
            match self.events.try_recv() {
                Ok(evt) => {
                    sink.push(evt);
                    n += 1;
                }
                Err(_) => break,
            }
        }
    }

    pub fn send(&self, cmd: EditorToRuntime) {
        let _ = self.cmd_tx.send(cmd);
    }

    /// Kill the runtime child (editor exit).
    pub fn shutdown(&self) {
        if let Some(child) = &self.runtime {
            if let Ok(mut c) = child.lock() {
                let _ = c.kill();
            }
        } else {
            self.send(EditorToRuntime::Shutdown);
        }
    }
}
