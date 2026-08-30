//! BevyForge editor application: owns the UI state, the network layer, the
//! project, the undo stack and the compiler runner; paints the full layout.

use std::time::{Duration, Instant};

use egui::{Context, ViewportBuilder, ViewportCommand};
use forge_editor_core::{Project, UndoEntry, UndoStack};
use forge_ipc::{
    ComponentKind, EditorToRuntime, FieldValue, LogLevel, RuntimeToEditor,
};

use crate::net::{Net, NetEvent};
use crate::state::{DockTab, EditorState, ScriptDoc, ViewportTab};
use crate::panels;

pub struct BevyForgeApp {
    pub state: EditorState,
    pub net: Option<Net>,
    pub project: Option<Project>,
    pub undo: UndoStack,
    pub compiler: Option<CompilerRun>,
    pub dialog: Option<DialogPurpose>,
    pub file_dialog: egui_file_dialog::FileDialog,
    pub hierarchy_drag: Option<u64>,
    pub keyframe_drag: Option<KeyframeDrag>,
    pub exit_after: Option<f64>,
    pub launched: Instant,
    pub viewport_size_sent: (u32, u32),
    pub image_previews: std::collections::HashMap<String, egui::TextureHandle>,
    pub pending_spawn_label: Option<String>,
    pub check_requested: bool,
    pub shortcuts_enabled: bool,
    pub hierarchy_search: String,
    pub asset_search: String,
    /// Clone of the current egui context (set each `ui` call) for async use.
    pub ui_ctx: Option<egui::Context>,
}

/// An in-flight `cargo check` for the scripts crate.
pub struct CompilerRun {
    pub child: std::process::Child,
    pub stdout: std::process::ChildStdout,
    pub parser: forge_editor_core::DiagnosticsParser,
    pub started: Instant,
}

/// What the file dialog is currently choosing.
#[derive(Debug, Clone, PartialEq)]
pub enum DialogPurpose {
    OpenProject,
    OpenScene,
    SaveSceneAs,
    NewProject,
}

/// Active keyframe drag on the timeline.
#[derive(Debug, Clone)]
pub struct KeyframeDrag {
    pub entity: u64,
    pub entity_name: String,
    pub track: forge_ipc::AnimTrack,
    pub index: usize,
}

impl BevyForgeApp {
    pub fn new(cc: &eframe::CreationContext<'_>, project: Option<Project>, net: Option<Net>, exit_after: Option<f64>) -> Self {
        crate::theme::install(&cc.egui_ctx);
        let mut state = EditorState::new();
        state.status_message = if net.is_some() { "Connecting…".into() } else { "No runtime".into() };
        state.logs.push(forge_ipc::LogEntry {
            level: forge_ipc::LogLevel::Info,
            time: crate::panels::clock_hms(),
            target: "bevyforge".into(),
            message: format!("BevyForge editor {} started", env!("CARGO_PKG_VERSION")),
        });
        if let Some(project) = &project {
            state.compile_raw.push(format!(
                "[{}] Project: {} ({})",
                crate::panels::clock_hms(),
                project.manifest.name,
                project.root.display()
            ));
        }
        Self {
            state,
            net,
            project,
            undo: UndoStack::new(200),
            compiler: None,
            dialog: None,
            file_dialog: {
                let mut cfg = egui_file_dialog::FileDialogConfig::default();
                cfg.title = Some("Choose a folder or file".to_string());
                let quick = |s: &mut egui_file_dialog::QuickAccess| {
                    s.add_path("Project Root", ".");
                };
                cfg.quick_accesses.push(egui_file_dialog::FileDialogConfig::default()
                    .add_quick_access("Project", quick)
                    .quick_accesses
                    .pop()
                    .expect("just pushed"));
                egui_file_dialog::FileDialog::with_config(cfg)
            },
            hierarchy_drag: None,
            keyframe_drag: None,
            exit_after,
            launched: Instant::now(),
            viewport_size_sent: (0, 0),
            image_previews: Default::default(),
            pending_spawn_label: None,
            check_requested: false,
            shortcuts_enabled: true,
            hierarchy_search: String::new(),
            asset_search: String::new(),
            ui_ctx: None,
        }
    }

    // ------------------------------------------------------------------
    // Command helpers
    // ------------------------------------------------------------------

    pub fn cmd(&mut self, cmd: EditorToRuntime) {
        if let Some(net) = &self.net {
            net.send(cmd);
        }
    }

    /// Record an undo entry and apply its forward (redo) commands.
    pub fn apply_with_undo(&mut self, label: &str, undo: Vec<EditorToRuntime>, redo: Vec<EditorToRuntime>) {
        for c in &redo {
            self.cmd(c.clone());
        }
        self.undo.push(UndoEntry {
            label: label.to_string(),
            undo,
            redo,
            spawned_entity: None,
            trash_id: None,
        });
    }

    pub fn selected_entity(&self) -> Option<u64> {
        self.state.selected
    }

    // ------------------------------------------------------------------
    // Net event processing
    // ------------------------------------------------------------------

    pub fn pump_network(&mut self) {
        let Some(net) = &self.net else { return };
        let mut events = Vec::new();
        net.drain(&mut events, 4096);
        for evt in events {
            match evt {
                NetEvent::Connected => {
                    self.state.connected = true;
                    self.state.status_message = "Ready".into();
                    self.cmd(EditorToRuntime::Hello);
                    self.cmd(EditorToRuntime::RequestFullState);
                    self.send_camera();
                }
                NetEvent::Disconnected(reason) => {
                    self.state.connected = false;
                    self.state.playing = false;
                    self.state.status_message = format!("Runtime offline — {reason}");
                }
                NetEvent::RuntimeExited(code) => {
                    self.state.connected = false;
                    self.state.playing = false;
                    self.state.status_message = format!("Runtime exited ({code:?})");
                }
                NetEvent::RuntimeStdout(line) => {
                    // Runtime console output lands in the Output tab.
                    self.state.compile_raw.push(line);
                    let excess = self.state.compile_raw.len().saturating_sub(2000);
                    if excess > 0 {
                        self.state.compile_raw.drain(..excess);
                    }
                }
                NetEvent::Message(msg) => self.handle_runtime_message(msg),
            }
        }
    }

    fn handle_runtime_message(&mut self, msg: RuntimeToEditor) {
        let s = &mut self.state;
        match msg {
            RuntimeToEditor::Welcome { protocol, forge_version, bevy_version, pid } => {
                if protocol != forge_ipc::PROTOCOL_VERSION {
                    s.push_toast(LogLevel::Error, format!("Protocol mismatch: editor {}, runtime {protocol}", forge_ipc::PROTOCOL_VERSION));
                }
                s.forge_version = forge_version;
                s.bevy_version = bevy_version.clone();
                s.runtime_pid = Some(pid);
                s.push_toast(LogLevel::Info, format!("Runtime connected (Bevy {bevy_version}, pid {pid})"));
            }
            RuntimeToEditor::Pong(_) => {}
            RuntimeToEditor::Frame { width, height, rgb } => {
                let expected = (width as usize * height as usize * 3, rgb.len());
                if expected.0 == expected.1 {
                    let image = egui::ColorImage::from_rgb(
                        [width as usize, height as usize],
                        &rgb,
                    );
                    s.frame = Some((width, height, image));
                }
            }
            RuntimeToEditor::Hierarchy { nodes } => {
                // Preserve expansion state; auto-expand depth<2 rows.
                for node in &nodes {
                    if node.depth < 1 {
                        s.expanded.insert(node.id);
                    }
                }
                s.hierarchy = nodes;
            }
            RuntimeToEditor::EntityComponents { entity, name, components } => {
                if s.selected == Some(entity) {
                    s.selected_name = name;
                    s.components = components;
                }
            }
            RuntimeToEditor::Logs(entries) => {
                s.logs.extend(entries);
                let excess = s.logs.len().saturating_sub(5000);
                if excess > 0 {
                    s.logs.drain(..excess);
                }
            }
            RuntimeToEditor::Stats(stats) => s.stats = stats,
            RuntimeToEditor::PickResult { x, y, entity } => {
                if x >= 0.0 {
                    if let Some(bits) = entity {
                        s.selected = Some(bits);
                        s.expanded.insert(bits);
                        self.cmd(EditorToRuntime::Select { entity: Some(bits) });
                    }
                }
            }
            RuntimeToEditor::AnimState { state, tracks } => {
                s.anim = state;
                s.anim_tracks = tracks;
            }
            RuntimeToEditor::EnvState(settings) => s.env = settings,
            RuntimeToEditor::SceneInfo { path, dirty } => {
                s.scene_path = path;
                s.scene_dirty = dirty;
            }
            RuntimeToEditor::Notice { level, message } => {
                s.push_toast(level, message);
            }
            RuntimeToEditor::ScreenshotDone { path, success, message } => {
                let _ = path;
                s.push_toast(if success { LogLevel::Info } else { LogLevel::Error }, message);
            }
            RuntimeToEditor::Goodbye { reason } => {
                s.connected = false;
                s.status_message = format!("Runtime closed — {reason}");
            }
        }
    }

    pub fn send_camera(&mut self) {
        let (target, distance, yaw, pitch) = self.state.camera_rig;
        self.cmd(EditorToRuntime::SetEditorCamera { target, distance, yaw_deg: yaw, pitch_deg: pitch });
    }

    // ------------------------------------------------------------------
    // Compiler runner
    // ------------------------------------------------------------------

    pub fn start_cargo_check(&mut self) {
        if self.state.compile_running || self.project.is_none() {
            return;
        }
        let Some(project) = self.project.clone() else { return };
        // cargo check runs against the WORKSPACE (scripts crate lives there);
        // locate it relative to the editor binary first, fall back to cwd.
        let manifest = std::env::current_exe()
            .ok()
            .and_then(|p| p.ancestors().nth(3).map(|d| d.join("Cargo.toml")))
            .filter(|p| p.exists())
            .unwrap_or_else(|| std::path::PathBuf::from("Cargo.toml"));
        let Ok(mut child) = std::process::Command::new("cargo")
            .arg("check")
            .arg("--message-format=json")
            .arg(format!("--manifest-path={}", manifest.display()))
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            self.state.push_toast(LogLevel::Error, "failed to launch cargo (is Rust installed?)");
            return;
        };
        let stdout = child.stdout.take().expect("piped stdout");
        self.compiler = Some(CompilerRun {
            child,
            stdout,
            parser: forge_editor_core::DiagnosticsParser::new(),
            started: Instant::now(),
        });
        self.state.compile_running = true;
        self.state.compile_raw.push(format!(
            "[{:}] Compiling workspace (cargo check)…",
            crate::panels::clock_hms()
        ));
        let _ = project;
    }

    pub fn pump_compiler(&mut self) {
        use std::io::{BufRead, BufReader};
        let Some(run) = &mut self.compiler else { return };
        let mut reader = BufReader::new(&mut run.stdout);
        let mut consumed = 0;
        loop {
            if consumed > 200 {
                break;
            }
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim_end().to_string();
                    if trimmed.starts_with('{') {
                        run.parser.feed_line(&trimmed);
                    } else if !trimmed.is_empty() {
                        self.state.compile_raw.push(trimmed);
                    }
                    consumed += 1;
                }
                Err(_) => break,
            }
        }
        // Non-blocking finish check.
        let finished = {
            let Some(run) = self.compiler.as_mut() else { return };
            matches!(run.child.try_wait(), Ok(Some(_)))
        };
        if consumed > 0 {
            // drain parser output
            let run = self.compiler.as_mut().unwrap();
            let diags = run.parser.take();
            for d in diags {
                self.state.diagnostics.push(d);
            }
            self.state.compile_errors = self
                .state
                .diagnostics
                .iter()
                .filter(|d| d.level == forge_editor_core::DiagnosticLevel::Error)
                .count() as u32;
            self.state.compile_warnings = self
                .state
                .diagnostics
                .iter()
                .filter(|d| d.level == forge_editor_core::DiagnosticLevel::Warning)
                .count() as u32;
        }
        if finished {
            let run = self.compiler.take().unwrap();
            let secs = run.started.elapsed().as_secs_f32();
            self.state.compile_running = false;
            let (e, w) = (self.state.compile_errors, self.state.compile_warnings);
            let summary = if e > 0 {
                format!("Finished check in {secs:.1}s — {e} error(s), {w} warning(s)")
            } else if w > 0 {
                format!("Finished check in {secs:.1}s — clean ({w} warning(s))")
            } else {
                format!("Finished check in {secs:.1}s — 0 errors")
            };
            self.state.compile_raw.push(summary.clone());
            self.state.push_toast(
                if e > 0 { LogLevel::Error } else { LogLevel::Info },
                summary,
            );
        }
    }

    // ------------------------------------------------------------------
    // Scripts
    // ------------------------------------------------------------------

    pub fn open_script(&mut self, path: &str) {
        if let Some(existing) = self.state.scripts.iter().position(|d| d.path == path) {
            self.state.active_script = Some(existing);
            return;
        }
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let name = std::path::Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.to_string());
                self.state.scripts.push(ScriptDoc {
                    path: path.to_string(),
                    name,
                    text,
                    dirty: false,
                    error_line: None,
                });
                self.state.active_script = Some(self.state.scripts.len() - 1);
                self.state.dock_tab = DockTab::Output;
            }
            Err(e) => self.state.push_toast(LogLevel::Error, format!("open script failed: {e}")),
        }
    }

    pub fn save_script(&mut self, index: usize) {
        let Some(doc) = self.state.scripts.get(index) else { return };
        let path = doc.path.clone();
        let text = doc.text.clone();
        match std::fs::write(&path, &text) {
            Ok(()) => {
                if let Some(doc) = self.state.scripts.get_mut(index) {
                    doc.dirty = false;
                }
                self.state.push_toast(LogLevel::Info, format!("saved {path}"));
                self.check_requested = true;
            }
            Err(e) => self.state.push_toast(LogLevel::Error, format!("save failed: {e}")),
        }
    }

    // ------------------------------------------------------------------
    // Scene ops
    // ------------------------------------------------------------------

    pub fn save_scene_to(&mut self, path: String) {
        self.cmd(EditorToRuntime::SaveScene { path });
    }

    pub fn open_scene(&mut self, path: String) {
        self.undo.clear();
        self.cmd(EditorToRuntime::OpenScene { path });
    }

    pub fn delete_selected(&mut self) {
        if let Some(entity) = self.selected_entity() {
            self.cmd(EditorToRuntime::DeleteEntity { entity });
            self.state.selected = None;
            self.state.components.clear();
        }
    }

    pub fn duplicate_selected(&mut self) {
        if let Some(entity) = self.selected_entity() {
            self.cmd(EditorToRuntime::DuplicateEntity { entity });
        }
    }

    pub fn toggle_play(&mut self) {
        let target = !self.state.playing;
        self.state.playing = target;
        self.cmd(EditorToRuntime::SetPlayMode { playing: target });
    }
}

impl eframe::App for BevyForgeApp {
    /// Non-UI work: networking, compiler, shortcuts, lifecycle timers.
    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // Auto-exit mode (used by validation harnesses).
        if let Some(secs) = self.exit_after {
            if self.launched.elapsed() > Duration::from_secs_f64(secs) {
                if let Some(net) = &self.net {
                    net.shutdown();
                }
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        }

        self.pump_network();
        if self.check_requested {
            self.check_requested = false;
            self.start_cargo_check();
        }
        self.pump_compiler();

        // Expire toasts.
        self.state
            .toasts
            .retain(|(t, _, _)| t.elapsed() < Duration::from_secs(5));

        crate::panels::handle_shortcuts(self, ctx);
        ctx.request_repaint_after(Duration::from_millis(33));
    }

    /// The whole editor layout; panels are added root-ui-first, CentralPanel last.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.ui_ctx = Some(ctx.clone());

        panels::top_menu_bar(self, ui);
        if self.state.show_hierarchy {
            panels::hierarchy_panel(self, ui);
        }
        if self.state.show_assets {
            panels::assets_panel(self, ui);
        }
        if self.state.show_inspector {
            panels::inspector_panel(self, ui);
        }
        if self.state.show_environment {
            panels::environment_panel(self, ui);
        }
        panels::status_bar(self, ui);
        panels::central(self, ui);
        panels::draw_toasts(self, &ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(net) = &self.net {
            net.shutdown();
        }
    }
}
