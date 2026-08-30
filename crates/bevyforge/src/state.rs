//! Editor-side mirrored state of the runtime world + UI session state.

use std::collections::HashSet;

use forge_ipc::{
    AnimEntityTracks, AnimState, ComponentData, EnvironmentSettings, HierNode, LogEntry, Stats,
};

/// Viewport tab (mirrors the design's Scene/Game tabs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewportTab {
    #[default]
    Scene,
    Game,
}

/// Which bottom dock tab is visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DockTab {
    Timeline,
    #[default]
    Console,
    Output,
}

/// A script file open in the script editor tab.
#[derive(Debug, Clone)]
pub struct ScriptDoc {
    pub path: String,
    pub name: String,
    pub text: String,
    pub dirty: bool,
    pub error_line: Option<(u32, String)>,
}

/// Everything the UI renders from.
#[allow(clippy::struct_excessive_bools)]
pub struct EditorState {
    // --- connection ---
    pub connected: bool,
    pub runtime_pid: Option<u32>,
    pub bevy_version: String,
    pub forge_version: String,
    pub status_message: String,

    // --- scene mirror ---
    pub hierarchy: Vec<HierNode>,
    pub expanded: HashSet<u64>,
    pub selected: Option<u64>,
    pub selected_name: String,
    pub components: Vec<ComponentData>,
    pub scene_path: Option<String>,
    pub scene_dirty: bool,

    // --- animation mirror ---
    pub anim: AnimState,
    pub anim_tracks: Vec<AnimEntityTracks>,

    // --- environment mirror ---
    pub env: EnvironmentSettings,

    // --- stats ---
    pub stats: Stats,

    // --- console ---
    pub logs: Vec<LogEntry>,
    pub log_filter: String,
    pub log_level_min: forge_ipc::LogLevel,

    // --- compiler ---
    pub diagnostics: Vec<forge_editor_core::Diagnostic>,
    pub compile_errors: u32,
    pub compile_warnings: u32,
    pub compile_running: bool,
    pub compile_raw: Vec<String>,
    pub selected_diagnostic: Option<usize>,

    // --- viewport ---
    pub viewport_tab: ViewportTab,
    pub frame: Option<(u32, u32, egui::ColorImage)>,
    pub texture: Option<egui::TextureHandle>,
    pub show_grid: bool,
    pub show_outline: bool,
    /// (target xyz, distance, yaw deg, pitch deg)
    pub camera_rig: ([f32; 3], f32, f32, f32),
    /// Runtime camera view-projection (column-major) for gizmo projection.
    pub camera_vp: Option<crate::gizmo::Mat4>,
    /// Runtime camera eye position (world space).
    pub camera_eye: [f32; 3],
    /// Active manipulator (toolbar / W-E-R).
    pub gizmo_mode: crate::gizmo::GizmoMode,
    /// In-progress viewport gizmo drag.
    pub gizmo_drag: Option<crate::gizmo::DragState>,

    // --- panels visibility ---
    pub show_hierarchy: bool,
    pub show_assets: bool,
    pub show_inspector: bool,
    pub show_environment: bool,
    pub show_timeline: bool,
    pub show_console: bool,
    pub show_compiler: bool,

    // --- docks ---
    pub dock_tab: DockTab,

    // --- script editor ---
    pub scripts: Vec<ScriptDoc>,
    pub active_script: Option<usize>,

    // --- toasts ---
    pub toasts: Vec<(std::time::Instant, forge_ipc::LogLevel, String)>,

    // --- image preview popup ---
    pub preview_popup: Option<String>,

    // --- play mode ---
    pub playing: bool,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            connected: false,
            runtime_pid: None,
            bevy_version: String::new(),
            forge_version: String::new(),
            status_message: String::new(),
            hierarchy: Vec::new(),
            expanded: HashSet::new(),
            selected: None,
            selected_name: String::new(),
            components: Vec::new(),
            scene_path: None,
            scene_dirty: false,
            anim: AnimState::default(),
            anim_tracks: Vec::new(),
            env: EnvironmentSettings::default(),
            stats: Stats::default(),
            logs: Vec::new(),
            log_filter: String::new(),
            log_level_min: forge_ipc::LogLevel::Debug,
            diagnostics: Vec::new(),
            compile_errors: 0,
            compile_warnings: 0,
            compile_running: false,
            compile_raw: Vec::new(),
            selected_diagnostic: None,
            viewport_tab: ViewportTab::Scene,
            frame: None,
            texture: None,
            show_grid: true,
            show_outline: true,
            camera_rig: ([0.0, 0.5, 0.0], 12.0, -35.0, 28.0),
            camera_vp: None,
            camera_eye: [0.0, 0.0, 12.0],
            gizmo_mode: crate::gizmo::GizmoMode::Translate,
            gizmo_drag: None,
            show_hierarchy: true,
            show_assets: true,
            show_inspector: true,
            show_environment: false,
            show_timeline: true,
            show_console: true,
            show_compiler: true,
            dock_tab: DockTab::Console,
            scripts: Vec::new(),
            active_script: None,
            toasts: Vec::new(),
            preview_popup: None,
            playing: false,
        }
    }
}

impl EditorState {
    pub fn new() -> Self {
        let mut state = Self::default();
        state.status_message = "Starting…".into();
        state
    }

    /// Toast helper.
    pub fn push_toast(&mut self, level: forge_ipc::LogLevel, message: impl Into<String>) {
        self.toasts.push((std::time::Instant::now(), level, message.into()));
        if self.toasts.len() > 8 {
            self.toasts.remove(0);
        }
    }
}
