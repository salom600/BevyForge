//! BevyForge editor application: owns the UI state, the network layer, the
//! project, the undo stack and the compiler runner; paints the full layout.

use std::time::{Duration, Instant};

use egui::{Context, ViewportCommand};
use forge_editor_core::{Project, UndoEntry, UndoStack};
use forge_ipc::{
    EditorToRuntime, LogLevel, RuntimeToEditor,
};

use crate::net::{Net, NetEvent};
use crate::offline::OfflineScene;
use crate::state::{DockTab, EditorState, ScriptDoc};
use crate::panels;

/// Engine version this editor pairs with (mirrors forge_runtime::ENGINE_VERSION).
pub const BEVY_VERSION: &str = "0.19";

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
    pub image_previews: std::collections::HashMap<String, egui::TextureHandle>,
    pub check_requested: bool,
    pub shortcuts_enabled: bool,
    pub hierarchy_search: String,
    pub asset_search: String,
    /// Clone of the current egui context (set each `ui` call) for async use.
    pub ui_ctx: Option<egui::Context>,
    /// TCP port the runtime binds (kept so respawns reuse it).
    pub runtime_port: u16,
    /// Why the runtime is not running (spawn/attach failure), shown in the
    /// offline banner. `None` while connected.
    pub spawn_error: Option<String>,
    /// When to attempt the next runtime respawn while offline.
    pub next_retry: Option<Instant>,
    /// Exponential backoff between respawn attempts (2 s … 10 s).
    pub retry_delay: Duration,
    /// In-flight respawn attempt (spawn runs on a worker thread so a slow or
    /// broken machine never freezes the editor UI).
    pub respawn_rx:
        Option<std::sync::mpsc::Receiver<anyhow::Result<crate::net::Net>>>,
    /// Throttle for the "action skipped while offline" toast.
    pub last_offline_warn: Option<Instant>,
    /// Set when the app is shutting down: suppress auto-respawn.
    pub closing: bool,
    /// The editor's own scene document, authoritative while the engine is
    /// offline. Makes every editing button genuinely work without the
    /// runtime; shipped to the engine via `LoadSceneDoc` on reconnect.
    pub offline: OfflineScene,
    /// Port to attach to on the first frame (`--connect`), run on a worker.
    pub initial_attach: Option<u16>,
    /// Sandbox UI smoke-test hook (`FORGE_UI_TEST` env): dialog purpose to
    /// auto-arm ~2 s after launch so an automated harness can screenshot and
    /// complete the real menu flow. Never set in normal use.
    pub ui_test_arm: Option<DialogPurpose>,
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
    pub track: forge_ipc::AnimTrack,
    pub index: usize,
}

impl BevyForgeApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        project: Option<Project>,
        net: Option<Net>,
        spawn_error: Option<String>,
        exit_after: Option<f64>,
        initial_attach: Option<u16>,
    ) -> Self {
        crate::theme::install(&cc.egui_ctx);
        let mut state = EditorState::new();
        // The offline scene document is seeded from the project's main scene
        // file BEFORE the engine exists, so the hierarchy/inspector are alive
        // from the very first frame.
        let offline = OfflineScene::from_project(project.as_ref());
        let (seed_hierarchy, seed_env) = (offline.hierarchy(None), offline.doc.environment.clone());
        state.hierarchy = seed_hierarchy;
        state.env = seed_env;
        let net_offline = net.as_ref().map(|n| n.is_offline()).unwrap_or(true);
        state.status_message = if net_offline {
            "Engine starting… (editing works meanwhile)".into()
        } else {
            "Connecting…".into()
        };
        state.logs.push(forge_ipc::LogEntry {
            level: forge_ipc::LogLevel::Info,
            time: crate::panels::clock_hms(),
            target: "bevyforge".into(),
            message: format!("BevyForge editor {} started", env!("CARGO_PKG_VERSION")),
        });
        if let Some(reason) = &spawn_error {
            state.logs.push(forge_ipc::LogEntry {
                level: forge_ipc::LogLevel::Error,
                time: crate::panels::clock_hms(),
                target: "runtime".into(),
                message: reason.clone(),
            });
            state
                .compile_raw
                .push(format!("[{}] runtime spawn failed: {reason}", crate::panels::clock_hms()));
        }
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
            image_previews: Default::default(),
            check_requested: false,
            shortcuts_enabled: true,
            hierarchy_search: String::new(),
            asset_search: String::new(),
            ui_ctx: None,
            runtime_port: forge_ipc::DEFAULT_PORT,
            next_retry: if net_offline { Some(Instant::now() + Duration::from_millis(200)) } else { None },
            retry_delay: Duration::from_secs(2),
            respawn_rx: None,
            spawn_error,
            last_offline_warn: None,
            closing: false,
            offline,
            initial_attach,
            ui_test_arm: std::env::var("FORGE_UI_TEST").ok().and_then(|m| match m.as_str() {
                "open_project" => Some(DialogPurpose::OpenProject),
                "new_project" => Some(DialogPurpose::NewProject),
                "open_scene" => Some(DialogPurpose::OpenScene),
                "save_scene_as" => Some(DialogPurpose::SaveSceneAs),
                _ => None,
            }),
        }
    }

    // ------------------------------------------------------------------
    // Command helpers
    // ------------------------------------------------------------------

    pub fn cmd(&mut self, cmd: EditorToRuntime) {
        if self.net.as_ref().map(|n| n.is_offline()).unwrap_or(true) {
            // OFFLINE: apply editing commands to the local scene document so
            // the editor stays genuinely usable; only engine-bound commands
            // (play, picking, screenshots) are refused with a toast.
            let feedback = self.offline.apply(&cmd);
            if let Some(id) = feedback.spawned {
                self.state.selected = Some(id);
                self.state.expanded.insert(id);
            }
            if let Some((level, message)) = feedback.notice {
                self.state.push_toast(level, message);
            }
            if let Some((label, undo, redo)) = feedback.gesture_done {
                self.push_undo_only(&label, undo, redo);
            }
            if feedback.needs_engine {
                let now = Instant::now();
                if self
                    .last_offline_warn
                    .map(|t| now.duration_since(t) > Duration::from_secs(3))
                    .unwrap_or(true)
                {
                    self.last_offline_warn = Some(now);
                    self.state.push_toast(
                        LogLevel::Warn,
                        "This action needs the render engine (auto-retry active, see banner)",
                    );
                }
            }
            return;
        }
        if let Some(net) = &self.net {
            net.send(cmd);
        }
    }

    // ------------------------------------------------------------------
    // Runtime lifecycle: respawn while offline, restart on demand
    // ------------------------------------------------------------------

    /// Attempt a respawn of the runtime when offline (exponential backoff,
    /// spawn executed on a worker thread so the UI stays responsive).
    pub fn pump_runtime_lifecycle(&mut self) {
        if self.closing {
            return;
        }
        let offline = self.net.as_ref().map(|n| n.is_offline()).unwrap_or(true);
        if !offline {
            self.next_retry = None;
            return;
        }

        // Poll an in-flight respawn attempt.
        if let Some(rx) = &self.respawn_rx {
            match rx.try_recv() {
                Ok(Ok(n)) => {
                    self.respawn_rx = None;
                    self.net = Some(n);
                    self.retry_delay = Duration::from_secs(2);
                    self.spawn_error = None;
                    self.state.status_message = "Connecting…".into();
                    crate::logging::info("engine process spawned and handshake OK");
                    self.state
                        .push_toast(LogLevel::Info, "Engine started — connecting…");
                    return;
                }
                Ok(Err(e)) => {
                    self.respawn_rx = None;
                    let msg = format!("{e:#}");
                    crate::logging::warn(&format!("engine spawn failed: {msg}"));
                    if self.spawn_error.as_deref() != Some(msg.as_str()) {
                        self.state.logs.push(forge_ipc::LogEntry {
                            level: forge_ipc::LogLevel::Error,
                            time: crate::panels::clock_hms(),
                            target: "runtime".into(),
                            message: msg.clone(),
                        });
                    }
                    self.spawn_error = Some(msg);
                    self.state.status_message = "Engine offline — retrying…".into();
                    return; // wait for the backoff timer before the next attempt
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.respawn_rx = None;
                }
            }
        }

        let Some(at) = self.next_retry else { return };
        if Instant::now() < at {
            return;
        }

        // One-shot --connect attach, also on a worker thread.
        if let Some(port) = self.initial_attach.take() {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(crate::net::Net::attach(port));
            });
            self.respawn_rx = Some(rx);
            self.state.status_message = "Attaching to engine…".into();
            return;
        }

        self.next_retry = Some(Instant::now() + self.retry_delay);
        self.retry_delay = (self.retry_delay * 2).min(Duration::from_secs(10));

        let root = self
            .project
            .as_ref()
            .map(|p| p.root.clone())
            .unwrap_or_default();
        let port = self.runtime_port;
        crate::logging::info(&format!("spawning bevyforge-runtime (project {}, port {port})", root.display()));
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(crate::net::Net::spawn_runtime(&root, port));
        });
        self.respawn_rx = Some(rx);
        self.state.status_message = "Engine starting… (editing works meanwhile)".into();
    }

    /// Kill and respawn the runtime (status-bar button / menu action).
    pub fn restart_runtime(&mut self) {
        if let Some(net) = &self.net {
            net.terminate();
        }
        self.net = Some(crate::net::Net::offline());
        self.state.connected = false;
        self.state.playing = false;
        self.state.status_message = "Restarting engine…".into();
        self.spawn_error = None;
        self.retry_delay = Duration::from_secs(1);
        self.next_retry = Some(Instant::now() + Duration::from_millis(300));
        self.state
            .push_toast(LogLevel::Info, "Restarting the render engine…");
    }

    // ------------------------------------------------------------------
    // File dialogs (project & scene open/save) — armed by the Project menu,
    // rendered and consumed by `pump_file_dialog` every frame.
    // ------------------------------------------------------------------

    /// Arm the shared file dialog for `purpose`, configuring title, start
    /// directory and default file name for that purpose. The dialog is drawn
    /// by `pump_file_dialog` on the following frames; a completed pick is
    /// routed to `handle_dialog_pick`, a cancel simply leaves it closed.
    pub fn arm_dialog(&mut self, purpose: DialogPurpose) {
        let (start_dir, scenes_dir) = match self.project.as_ref() {
            Some(p) => (
                Some(
                    p.root
                        .parent()
                        .map(|d| d.to_path_buf())
                        .unwrap_or_else(|| p.root.clone()),
                ),
                Some(p.scenes_dir()),
            ),
            None => (None, None),
        };
        let cfg = self.file_dialog.config_mut();
        match &purpose {
            DialogPurpose::OpenProject => {
                cfg.title = Some("Open project — choose the project folder".into());
                if let Some(dir) = start_dir {
                    cfg.initial_directory = dir;
                }
            }
            DialogPurpose::NewProject => {
                cfg.title = Some("New project — choose a folder for it".into());
                if let Some(dir) = start_dir {
                    cfg.initial_directory = dir;
                }
            }
            DialogPurpose::OpenScene => {
                cfg.title = Some("Open scene (.scn.ron)".into());
                if scenes_dir.as_ref().is_some_and(|d| d.is_dir()) {
                    cfg.initial_directory = scenes_dir.unwrap_or_default();
                }
            }
            DialogPurpose::SaveSceneAs => {
                cfg.title = Some("Save scene as".into());
                if scenes_dir.as_ref().is_some_and(|d| d.is_dir()) {
                    cfg.initial_directory = scenes_dir.unwrap_or_default();
                }
                cfg.default_file_name = "scene.scn.ron".into();
            }
        }
        match purpose {
            DialogPurpose::OpenProject | DialogPurpose::NewProject => self.file_dialog.pick_directory(),
            DialogPurpose::OpenScene => self.file_dialog.pick_file(),
            DialogPurpose::SaveSceneAs => self.file_dialog.save_file(),
        }
        self.console_log(
            LogLevel::Info,
            "dialog",
            format!("{purpose:?} dialog armed"),
        );
        self.dialog = Some(purpose);
    }

    /// Render the armed file dialog (a cheap no-op while none is armed) and
    /// route a completed pick to `handle_dialog_pick`. A cancelled dialog is
    /// left armed-but-closed: it draws nothing and the next `arm_dialog`
    /// reconfigures and reuses it.
    pub fn pump_file_dialog(&mut self, ctx: &egui::Context) {
        if self.dialog.is_none() {
            return;
        }
        self.file_dialog.update(ctx);
        if let Some(path) = self.file_dialog.take_picked() {
            let purpose = self.dialog.take().expect("dialog purpose is Some");
            self.console_log(
                LogLevel::Info,
                "dialog",
                format!("picked: {}", path.display()),
            );
            self.handle_dialog_pick(purpose, path);
        }
    }

    fn handle_dialog_pick(&mut self, purpose: DialogPurpose, path: std::path::PathBuf) {
        match purpose {
            DialogPurpose::OpenProject => self.open_project_at(&path),
            DialogPurpose::NewProject => self.create_project_at(&path),
            DialogPurpose::OpenScene => self.open_scene_at(path),
            DialogPurpose::SaveSceneAs => self.save_scene_at(path),
        }
    }

    /// Make the folder picked in the "Open Project…" dialog the active
    /// project. Any existing folder works — a missing manifest gets a
    /// default one (same behavior as the CLI).
    pub fn open_project_at(&mut self, dir: &std::path::Path) {
        match Project::open(dir) {
            Ok(project) => self.switch_project(project),
            Err(e) => {
                let msg = format!("Opening project failed: {e:#}");
                self.state.push_toast(LogLevel::Error, msg.clone());
                self.console_log(LogLevel::Error, "project", msg);
            }
        }
    }

    /// Create a fresh project in the folder picked in the "New Project…"
    /// dialog and make it active. A folder that already holds a project is
    /// opened as-is instead (idempotent, never destroys content).
    pub fn create_project_at(&mut self, dir: &std::path::Path) {
        if dir.join("BevyForge.toml").exists() {
            self.open_project_at(dir);
            return;
        }
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "NewProject".into());
        match Project::create(dir, &name) {
            Ok(project) => {
                self.console_log(
                    LogLevel::Info,
                    "project",
                    format!("created '{name}' in {}", dir.display()),
                );
                self.switch_project(project);
            }
            Err(e) => {
                let msg = format!("Creating project failed: {e:#}");
                self.state.push_toast(LogLevel::Error, msg.clone());
                self.console_log(LogLevel::Error, "project", msg);
            }
        }
    }

    /// Make `project` the active one: reseed the offline scene from its main
    /// scene file, point save/load defaults at it and restart the engine so
    /// the fresh runtime process runs with the new project root.
    fn switch_project(&mut self, project: Project) {
        let main_scene = project.resolve_scene("");
        self.console_log(
            LogLevel::Info,
            "project",
            format!(
                "active: {} ({})",
                project.manifest.name,
                project.root.display()
            ),
        );
        self.offline = OfflineScene::from_project(Some(&project));
        self.state.hierarchy = self.offline.hierarchy(None);
        self.state.env = self.offline.doc.environment.clone();
        self.state.scene_path = Some(main_scene.to_string_lossy().to_string());
        self.state.selected = None;
        self.state.components.clear();
        self.undo.clear();
        self.project = Some(project);
        self.restart_runtime();
        let name = self
            .project
            .as_ref()
            .map(|p| p.manifest.name.clone())
            .unwrap_or_default();
        self.state
            .push_toast(LogLevel::Info, format!("Project '{name}' active — engine restarting"));
    }

    /// Route the "Open Scene…" pick through the normal scene pipeline
    /// (engine while online, offline document otherwise).
    pub fn open_scene_at(&mut self, path: std::path::PathBuf) {
        if !path.is_file() {
            let msg = format!("Scene not found: {}", path.display());
            self.state.push_toast(LogLevel::Error, msg.clone());
            self.console_log(LogLevel::Error, "scene", msg);
            return;
        }
        let path = path.to_string_lossy().to_string();
        self.state.scene_path = Some(path.clone());
        self.console_log(LogLevel::Info, "scene", format!("opening {path}"));
        self.open_scene(path);
    }

    /// Route the "Save Scene As…" pick; the chosen path becomes the new
    /// default save target for Ctrl+S as well.
    pub fn save_scene_at(&mut self, path: std::path::PathBuf) {
        let path = path.to_string_lossy().to_string();
        self.state.scene_path = Some(path.clone());
        self.console_log(LogLevel::Info, "scene", format!("saving {path}"));
        self.save_scene_to(path);
    }

    /// Push a line into the editor Console panel so every dialog action is
    /// visibly logged (no silent failures anywhere in the project flow).
    pub fn console_log(&mut self, level: LogLevel, target: &str, message: String) {
        self.state.logs.push(forge_ipc::LogEntry {
            level,
            time: crate::panels::clock_hms(),
            target: target.into(),
            message,
        });
    }

    /// Sandbox UI smoke-test hook: `FORGE_UI_TEST=<dialog>` arms that dialog
    /// ~2 s after launch so an automated harness can screenshot and complete
    /// the real menu flow end to end.
    fn pump_ui_test(&mut self) {
        if self.ui_test_arm.is_none()
            || self.dialog.is_some()
            || self.launched.elapsed() < Duration::from_secs(2)
        {
            return;
        }
        if let Some(purpose) = self.ui_test_arm.take() {
            crate::logging::info(&format!("FORGE_UI_TEST: arming dialog {purpose:?}"));
            self.arm_dialog(purpose);
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

    /// Record an undo entry WITHOUT re-applying its forward commands (used by
    /// gizmo gestures: the effects are already live in the runtime).
    pub fn push_undo_only(&mut self, label: &str, undo: Vec<EditorToRuntime>, redo: Vec<EditorToRuntime>) {
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
                    // If the scene document diverged from disk while the
                    // engine was down (offline edits or a crash losing the
                    // world), push it now; the runtime rebuilds its world and
                    // re-assigns entity ids.
                    if self.offline.has_unsynced_edits() {
                        let scene = self.offline.to_scene();
                        let count = scene.entities.len();
                        if let Some(net) = &self.net {
                            net.send(EditorToRuntime::LoadSceneDoc { scene });
                        }
                        self.undo.clear();
                        self.state.selected = None;
                        self.state.selected_name.clear();
                        self.state.components.clear();
                        self.state.scene_dirty = true;
                        self.state.push_toast(
                            LogLevel::Info,
                            format!(
                                "Synced {count} offline edit(s) to the engine — undo history reset"
                            ),
                        );
                    }
                    self.cmd(EditorToRuntime::Hello);
                    self.cmd(EditorToRuntime::RequestFullState);
                    self.send_camera();
                }
                NetEvent::Disconnected(reason) => {
                    self.state.connected = false;
                    self.state.playing = false;
                    self.state.status_message = format!("Runtime offline — {reason}");
                    crate::logging::warn(&format!("runtime link lost: {reason}"));
                }
                NetEvent::RuntimeExited(code) => {
                    self.state.connected = false;
                    self.state.playing = false;
                    self.state.status_message = format!("Runtime exited ({code:?})");
                    // Crash recovery: swap to offline mode and let the
                    // lifecycle pump respawn the engine automatically.
                    if self.closing {
                        return;
                    }
                    if let Some(net) = &self.net {
                        net.terminate();
                    }
                    self.net = Some(crate::net::Net::offline());
                    self.spawn_error = Some(format!("the render engine exited unexpectedly (code {code:?})"));
                    crate::logging::error(&format!("runtime exited unexpectedly (code {code:?})"));
                    self.state.logs.push(forge_ipc::LogEntry {
                        level: forge_ipc::LogLevel::Error,
                        time: crate::panels::clock_hms(),
                        target: "runtime".into(),
                        message: format!("engine exited (code {code:?}) — respawning"),
                    });
                    self.retry_delay = Duration::from_secs(2);
                    self.next_retry = Some(Instant::now() + Duration::from_secs(2));
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
            RuntimeToEditor::CameraInfo { view_proj, eye } => {
                s.camera_vp = Some(crate::gizmo::Mat4::from_cols_array(view_proj));
                s.camera_eye = eye;
            }
            RuntimeToEditor::GestureDone { entity, label, pre, post } => {
                if s.selected == Some(entity) && pre != post {
                    let fields = |t: forge_ipc::TransformAbs| vec![
                        EditorToRuntime::SetField {
                            entity,
                            component: forge_ipc::ComponentKind::Transform,
                            field: forge_ipc::ComponentField::Translation,
                            value: forge_ipc::FieldValue::Vec3(t.translation),
                        },
                        EditorToRuntime::SetField {
                            entity,
                            component: forge_ipc::ComponentKind::Transform,
                            field: forge_ipc::ComponentField::RotationEulerDeg,
                            value: forge_ipc::FieldValue::Vec3(t.euler_deg),
                        },
                        EditorToRuntime::SetField {
                            entity,
                            component: forge_ipc::ComponentKind::Transform,
                            field: forge_ipc::ComponentField::Scale,
                            value: forge_ipc::FieldValue::Vec3(t.scale),
                        },
                    ];
                    self.push_undo_only(&label, fields(pre), fields(post));
                }
            }
            RuntimeToEditor::PickResult { x, y: _, entity } => {
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
                // Treat a graceful goodbye as a crash for recovery purposes.
                if !self.closing {
                    if let Some(net) = &self.net {
                        net.terminate();
                    }
                    self.net = Some(crate::net::Net::offline());
                    self.spawn_error = Some(format!("the engine closed the connection ({reason})"));
                    self.retry_delay = Duration::from_secs(2);
                    self.next_retry = Some(Instant::now() + Duration::from_secs(2));
                }
            }
        }
    }

    /// Refresh the UI mirror (hierarchy/inspector/env/scene info) from the
    /// offline document whenever it changed. Cheap; runs at most once per
    /// frame and only when a command actually touched the doc.
    fn sync_offline_mirror(&mut self) {
        if !self.offline.take_mirror_dirty() {
            // Still refresh the offline camera each frame so gizmos track the
            // orbit rig without the engine.
            if self.net.as_ref().map(|n| n.is_offline()).unwrap_or(true) {
                self.update_offline_camera();
            }
            return;
        }
        let selected = self.state.selected;
        let nodes = self.offline.hierarchy(selected);
        for node in &nodes {
            if node.depth < 1 {
                self.state.expanded.insert(node.id);
            }
        }
        self.state.hierarchy = nodes;
        let found = selected.and_then(|id| self.offline.components_for(id));
        match found {
            Some((name, components)) => {
                self.state.selected_name = name;
                self.state.components = components;
            }
            None => {
                if self.state.selected.is_some() {
                    self.state.selected = None;
                    self.state.selected_name.clear();
                    self.state.components.clear();
                }
            }
        }
        self.state.scene_path = self.offline.scene_path();
        self.state.scene_dirty = self.offline.has_unsynced_edits();
        self.state.env = self.offline.doc.environment.clone();
        self.update_offline_camera();
    }

    /// While offline the runtime cannot report `CameraInfo`; derive the
    /// view-projection from the editor rig so the gizmo stays usable.
    fn update_offline_camera(&mut self) {
        if self.net.as_ref().map(|n| !n.is_offline()).unwrap_or(false) {
            return; // engine online → CameraInfo is authoritative
        }
        let (target, distance, yaw, pitch) = self.state.camera_rig;
        let (yr, pr) = (yaw.to_radians(), pitch.clamp(-89.0, 89.0).to_radians());
        let eye = [
            distance * pr.cos() * yr.cos() + target[0],
            distance * pr.sin() + target[1],
            distance * pr.cos() * yr.sin() + target[2],
        ];
        let view = crate::gizmo::Mat4::look_at_rh(eye, target, [0.0, 1.0, 0.0]);
        let proj = crate::gizmo::Mat4::perspective_rh(45.0_f32.to_radians(), 16.0 / 9.0, 0.1, 2000.0);
        self.state.camera_eye = eye;
        self.state.camera_vp = Some(proj.mul(&view));
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
        if self.net.as_ref().map(|n| n.is_offline()).unwrap_or(true) {
            // Honest feedback without flipping the UI into a broken state.
            self.cmd(EditorToRuntime::SetPlayMode { playing: true });
            return;
        }
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
                self.closing = true;
                if let Some(net) = &self.net {
                    net.shutdown();
                }
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        }

        self.pump_network();
        self.pump_runtime_lifecycle();
        self.sync_offline_mirror();
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
        panels::offline_banner(self, ui);
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
        self.pump_ui_test();
        self.pump_file_dialog(&ctx);
        panels::draw_toasts(self, &ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(net) = &self.net {
            net.shutdown();
        }
    }
}
