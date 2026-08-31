//! # forge_ipc — BevyForge wire protocol
//!
//! BevyForge is split into two cooperating processes:
//!
//! * **`bevyforge`** — the editor. An [egui](https://github.com/emilk/egui) desktop
//!   application that owns all user interaction: panels, hierarchy, inspector,
//!   timeline, script editor, compiler output.
//! * **`bevyforge-runtime`** — the engine. A [Bevy](https://bevy.org) application
//!   that owns the ECS [`World`](bevy::prelude::World), renders the scene offscreen
//!   and executes every command the editor sends.
//!
//! The two processes speak this protocol over a length-prefixed TCP stream,
//! serialised with [postcard](https://crates.io/crates/postcard) (compact, self-describing
//! enough, and fast enough for 30 Hz RGB frame streaming over loopback).
//!
//! ```text
//! editor ──EditorToRuntime──▶ runtime  (commands)
//! editor ◀──RuntimeToEditor── runtime  (state, frames, logs, stats)
//! ```

#![allow(clippy::module_inception)]

pub mod math;
pub mod scene_doc;
pub mod transport;

pub use scene_doc::{
    ForgeScene, SceneAnimation, SceneAnimEntry, SceneCamera, SceneEntity, SceneEntityKind,
    SceneLight, SceneMaterial, SceneScript,
};
pub use transport::{connect, listen, send_on_stream, Connection};

/// BevyForge protocol version. Bumped whenever a message type changes meaning.
pub const PROTOCOL_VERSION: u32 = 3;

/// Default TCP port the runtime listens on when none is requested.
pub const DEFAULT_PORT: u16 = 48470;

/// Entity handle as seen across the process boundary.
///
/// The editor treats these as opaque tokens; only the runtime interprets them.
pub type EntityId = u64;

/// Sentinel for "no entity" that keeps types simpler than `Option<Option<_>>`.
pub const NO_ENTITY: EntityId = u64::MAX;

// ---------------------------------------------------------------------------
// Top-level envelope
// ---------------------------------------------------------------------------

/// Every framed message on the wire is one of these.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Message {
    /// Editor -> runtime command.
    ToRuntime(EditorToRuntime),
    /// Runtime -> editor state or event.
    ToEditor(RuntimeToEditor),
}

// ---------------------------------------------------------------------------
// Shared data model
// ---------------------------------------------------------------------------

/// Primitive mesh shapes the runtime can spawn directly.
/// These map 1:1 onto `bevy::prelude::mesh` constructors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MeshPrimitive {
    Cube,
    Sphere,
    Icosphere,
    Capsule,
    Cylinder,
    Cone,
    Plane,
    Torus,
}

impl MeshPrimitive {
    /// Human readable name shown in menus.
    pub fn label(self) -> &'static str {
        match self {
            MeshPrimitive::Cube => "Cube",
            MeshPrimitive::Sphere => "Sphere",
            MeshPrimitive::Icosphere => "Icosphere",
            MeshPrimitive::Capsule => "Capsule",
            MeshPrimitive::Cylinder => "Cylinder",
            MeshPrimitive::Cone => "Cone",
            MeshPrimitive::Plane => "Plane",
            MeshPrimitive::Torus => "Torus",
        }
    }
    /// Every primitive, in menu order.
    pub const ALL: &'static [MeshPrimitive] = &[
        MeshPrimitive::Cube,
        MeshPrimitive::Sphere,
        MeshPrimitive::Icosphere,
        MeshPrimitive::Capsule,
        MeshPrimitive::Cylinder,
        MeshPrimitive::Cone,
        MeshPrimitive::Plane,
        MeshPrimitive::Torus,
    ];
}

/// What the `GameObject` menu can spawn.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum EntityKind {
    /// Bare entity with only a `Name`.
    Empty,
    /// Mesh primitive with a default `StandardMaterial`.
    Mesh(MeshPrimitive),
    /// Perspective 3D camera.
    Camera,
    /// Directional "sun" light.
    DirectionalLight,
    /// Omni point light.
    PointLight,
    /// Cone spot light.
    SpotLight,
    /// Gameplay prefab: capsule mesh + `Player`, `CharacterController`,
    /// `Health` and `Inventory` script components.
    PlayerPrefab,
}

impl EntityKind {
    pub fn label(self) -> &'static str {
        match self {
            EntityKind::Empty => "Empty Entity",
            EntityKind::Mesh(p) => p.label(),
            EntityKind::Camera => "Camera",
            EntityKind::DirectionalLight => "Directional Light",
            EntityKind::PointLight => "Point Light",
            EntityKind::SpotLight => "Spot Light",
            EntityKind::PlayerPrefab => "Player Prefab",
        }
    }
}

/// Components the editor knows how to create and edit. The runtime registers
/// exactly this set in its `TypeRegistry`, keeping both sides in lockstep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ComponentKind {
    Transform,
    Visibility,
    Mesh,
    Material,
    Camera,
    DirectionalLight,
    PointLight,
    SpotLight,
    // --- forge_scripts (user-editable gameplay components) ---
    Rotator,
    Orbiter,
    LinearMover,
    PingPongMover,
    Player,
    CharacterController,
    Health,
    Inventory,
}

impl ComponentKind {
    pub fn label(self) -> &'static str {
        match self {
            ComponentKind::Transform => "Transform",
            ComponentKind::Visibility => "Visibility",
            ComponentKind::Mesh => "Mesh",
            ComponentKind::Material => "Material",
            ComponentKind::Camera => "Camera",
            ComponentKind::DirectionalLight => "Directional Light",
            ComponentKind::PointLight => "Point Light",
            ComponentKind::SpotLight => "Spot Light",
            ComponentKind::Rotator => "Rotator",
            ComponentKind::Orbiter => "Orbiter",
            ComponentKind::LinearMover => "LinearMover",
            ComponentKind::PingPongMover => "PingPongMover",
            ComponentKind::Player => "Player",
            ComponentKind::CharacterController => "CharacterController",
            ComponentKind::Health => "Health",
            ComponentKind::Inventory => "Inventory",
        }
    }

    /// Components offered in the `+ Add Component` menu (built-ins that every
    /// entity does not already carry, plus all script components).
    pub const ADDABLE: &'static [ComponentKind] = &[
        ComponentKind::Rotator,
        ComponentKind::Orbiter,
        ComponentKind::LinearMover,
        ComponentKind::PingPongMover,
        ComponentKind::Player,
        ComponentKind::CharacterController,
        ComponentKind::Health,
        ComponentKind::Inventory,
    ];
}

/// A single editable field path inside a component.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ComponentField {
    // Transform
    Translation,
    RotationEulerDeg,
    Scale,
    // Visibility
    EntityVisible,
    InheritedVisible,
    // Mesh
    MeshPrimitiveKind,
    // Material
    BaseColor,
    Metallic,
    Roughness,
    Emissive,
    // Camera
    FovDeg,
    CameraHdr,
    // DirectionalLight
    SunColor,
    SunIlluminance,
    SunShadows,
    // PointLight / SpotLight
    LightColor,
    LightIntensity,
    LightRadius,
    LightRange,
    LightOuterAngleDeg,
    LightShadows,
    // Rotator
    RotatorSpeed,
    // Orbiter
    OrbiterCenter,
    OrbiterRadius,
    OrbiterSpeed,
    // LinearMover
    MoverVelocity,
    MoverPingPong,
    // PingPongMover
    PingPongOffset,
    PingPongPeriod,
    // Player
    PlayerSpeed,
    PlayerJumpForce,
    PlayerSprintMultiplier,
    // CharacterController
    CcHeight,
    CcRadius,
    CcStepOffset,
    CcSlopeLimit,
    // Health
    HealthCurrent,
    HealthMax,
    // Inventory
    InventorySlots,
}

/// Typed value payload for [`ComponentField`] writes.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum FieldValue {
    F32(f32),
    U32(u32),
    Bool(bool),
    Vec3([f32; 3]),
    Rgba([f32; 4]),
    Mesh(MeshPrimitive),
    Str(String),
}

/// Hierarchy icon hint computed runtime-side from component presence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NodeIcon {
    Camera,
    Light,
    Mesh,
    Player,
    Script,
    Group,
    Environment,
}

/// One node of the flattened-then-rebuilt scene tree.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HierNode {
    pub id: EntityId,
    pub name: String,
    pub icon: NodeIcon,
    pub visible: bool,
    pub locked: bool,
    pub has_children: bool,
    pub depth: u32,
    /// True when this entity is currently selected in the runtime.
    pub selected: bool,
}

/// A component's full editable state, serialised for the inspector.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ComponentData {
    pub kind: ComponentKind,
    /// Generic key/value rows the inspector renders in order.
    pub rows: Vec<(ComponentField, FieldRow)>,
}

/// One row in the inspector: a label plus a typed editable value.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FieldRow {
    pub label: String,
    pub value: FieldValue,
    /// Optional display-only annotation, e.g. unit suffix ("lux", "deg").
    pub unit: Option<String>,
}

/// An entity's transform, captured runtime-side with glam for exact undo.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TransformAbs {
    pub entity: EntityId,
    pub translation: [f32; 3],
    /// Degrees, glam `EulerRot::XYZ` convention (matches the inspector).
    pub euler_deg: [f32; 3],
    pub scale: [f32; 3],
}

/// Log record captured by the runtime's tracing layer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LogEntry {
    pub level: LogLevel,
    /// "12:45:10" style timestamp.
    pub time: String,
    pub target: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
    Debug,
    Trace,
}

impl LogLevel {
    /// Tag as rendered in the console panel, e.g. `[INFO]`.
    pub fn tag(self) -> &'static str {
        match self {
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
            LogLevel::Debug => "DEBUG",
            LogLevel::Trace => "TRACE",
        }
    }
}

/// Per-frame statistics for the status bar.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Stats {
    pub fps: f32,
    pub frame_ms: f32,
    pub entity_count: u32,
    pub system_count: u32,
    /// Resident memory in MiB (0 when the platform cannot report it).
    pub mem_mib: f32,
    /// wgpu backend name, e.g. "Vulkan" / "GL".
    pub backend: String,
}

/// Editor-authored transform keyframe track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum AnimTrack {
    Translation,
    Rotation,
    Scale,
}

impl AnimTrack {
    pub fn label(self) -> &'static str {
        match self {
            AnimTrack::Translation => "Position",
            AnimTrack::Rotation => "Rotation",
            AnimTrack::Scale => "Scale",
        }
    }
}

/// All tracks for one animated entity, mirrored to the timeline UI.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AnimEntityTracks {
    pub entity: EntityId,
    pub name: String,
    /// (track, sorted keyframe times)
    pub tracks: Vec<(AnimTrack, Vec<(f32, [f32; 3])>)>,
}

/// Global animation playback state.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct AnimState {
    pub time: f32,
    pub duration: f32,
    pub playing: bool,
    pub looped: bool,
}

impl Default for AnimState {
    fn default() -> Self {
        Self { time: 0.0, duration: 30.0, playing: false, looped: true }
    }
}

/// Viewport/environment render settings controlled from the lighting panel.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EnvironmentSettings {
    pub ambient_color: [f32; 4],
    pub ambient_brightness: f32,
    pub sun_illuminance: f32,
    pub sun_elevation_deg: f32,
    pub sun_azimuth_deg: f32,
    pub sun_shadows: bool,
    pub tonemapping: TonemappingKind,
    /// Camera exposure in EV100 units (bevy 0.19 `Exposure { ev100 }`).
    pub exposure_ev100: f32,
    pub fog_enabled: bool,
    pub fog_color: [f32; 4],
    pub fog_start: f32,
    pub fog_end: f32,
    pub clear_color: [f32; 4],
    pub show_grid: bool,
    pub show_selection_outline: bool,
}

impl Default for EnvironmentSettings {
    fn default() -> Self {
        Self {
            ambient_color: [1.0, 1.0, 1.0, 0.0],
            ambient_brightness: 0.15,
            sun_illuminance: 12_000.0,
            sun_elevation_deg: 45.0,
            sun_azimuth_deg: 150.0,
            sun_shadows: true,
            tonemapping: TonemappingKind::AcesFitted,
            exposure_ev100: 9.7,
            fog_enabled: false,
            fog_color: [0.06, 0.08, 0.12, 1.0],
            fog_start: 20.0,
            fog_end: 120.0,
            clear_color: [0.035, 0.045, 0.07, 1.0],
            show_grid: true,
            show_selection_outline: true,
        }
    }
}

/// Tonemapping operators exposed by the engine renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TonemappingKind {
    None,
    Reinhard,
    AcesFitted,
    AgX,
    TonyMcMapface,
    BlenderFilmic,
}

impl TonemappingKind {
    pub const ALL: &'static [TonemappingKind] = &[
        TonemappingKind::None,
        TonemappingKind::Reinhard,
        TonemappingKind::AcesFitted,
        TonemappingKind::AgX,
        TonemappingKind::TonyMcMapface,
        TonemappingKind::BlenderFilmic,
    ];
    pub fn label(self) -> &'static str {
        match self {
            TonemappingKind::None => "None (Linear)",
            TonemappingKind::Reinhard => "Reinhard",
            TonemappingKind::AcesFitted => "ACES Fitted",
            TonemappingKind::AgX => "AgX",
            TonemappingKind::TonyMcMapface => "Tony McMapface",
            TonemappingKind::BlenderFilmic => "Blender Filmic",
        }
    }
}

// ---------------------------------------------------------------------------
// Editor -> Runtime
// ---------------------------------------------------------------------------

/// Commands the editor issues. Every variant maps to an ECS mutation the
/// runtime performs during its `ApplyEditorCommands` schedule.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum EditorToRuntime {
    /// Handshake; runtime answers with [`RuntimeToEditor::Welcome`].
    Hello,
    Ping(u64),
    /// Resize the offscreen render target backing the viewport panel.
    SetViewportSize { width: u32, height: u32 },
    /// Change the currently highlighted entity (gizmo outline).
    Select { entity: Option<EntityId> },
    SpawnEntity { name: String, parent: Option<EntityId>, kind: EntityKind },
    DeleteEntity { entity: EntityId },
    DuplicateEntity { entity: EntityId },
    Reparent { entity: EntityId, new_parent: Option<EntityId> },
    RenameEntity { entity: EntityId, name: String },
    SetField { entity: EntityId, component: ComponentKind, field: ComponentField, value: FieldValue },
    AddComponent { entity: EntityId, component: ComponentKind },
    RemoveComponent { entity: EntityId, component: ComponentKind },
    /// Discard the scene and start empty (keeps camera + sun).
    NewScene,
    OpenScene { path: String },
    SaveScene { path: String },
    /// Replace the runtime world with this scene document. Used by the editor
    /// to push edits it made while the engine was offline; entity ids are
    /// re-assigned by the runtime, so a full state refresh follows.
    LoadSceneDoc { scene: ForgeScene },
    /// Toggle play mode; runtime snapshots the world and enables game systems.
    SetPlayMode { playing: bool },
    /// Request a ray cast through the viewport; answers `PickResult`.
    Pick { x: f32, y: f32 },
    // --- animation ---
    AddKeyframe { entity: EntityId, track: AnimTrack, time: f32, value: [f32; 3] },
    RemoveKeyframe { entity: EntityId, track: AnimTrack, index: usize },
    MoveKeyframe { entity: EntityId, track: AnimTrack, index: usize, new_time: f32 },
    ClearTracks { entity: EntityId },
    SetAnimTime(f32),
    SetAnimPlaying(bool),
    SetAnimDuration(f32),
    SetAnimLooped(bool),
    // --- environment / lighting ---
    SetEnvironment(EnvironmentSettings),
    // --- camera rig ---
    /// Position the free editor camera (orbit rig: target + spherical offset).
    SetEditorCamera { target: [f32; 3], distance: f32, yaw_deg: f32, pitch_deg: f32 },
    /// Render through a scene camera entity (Game tab) or the editor rig (None).
    SetViewportCamera { entity: Option<EntityId> },
    /// Hierarchy lock toggle (prevents delete/move on locked entities).
    SetLocked { entity: EntityId, locked: bool },
    // --- gizmo manipulation (relative, applied to the live transform) ---
    /// Nudge translation by a world-space delta (viewport gizmo drag).
    MoveEntity { entity: EntityId, delta: [f32; 3] },
    /// Rotate around a world-space axis through the entity origin (degrees).
    RotateEntityWorld { entity: EntityId, axis: [f32; 3], angle_deg: f32 },
    /// Multiply the entity scale by per-axis factors (clamped > 0.001).
    ScaleEntityBy { entity: EntityId, factor: [f32; 3] },
    /// Start a gizmo drag: runtime snapshots the entity transform for undo.
    BeginGizmoGesture { entity: EntityId },
    /// Finish a gizmo drag: runtime reports exact pre/post transforms so the
    /// editor can push a precise undo/redo pair (no float drift).
    EndGizmoGesture { entity: EntityId, label: String },
    // --- misc ---
    /// Ask for an immediate full state sync (hierarchy, components, anim, env).
    RequestFullState,
    /// Runtime renders one 1920x1080 frame and writes a PNG to `path`.
    RequestScreenshot { path: String },
    /// Graceful exit.
    Shutdown,
}

// ---------------------------------------------------------------------------
// Runtime -> Editor
// ---------------------------------------------------------------------------

/// State and events pushed from the runtime to the editor.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum RuntimeToEditor {
    Welcome { protocol: u32, forge_version: String, bevy_version: String, pid: u32 },
    Pong(u64),
    /// RGB8 offscreen render of the scene camera, viewport-sized.
    Frame { width: u32, height: u32, rgb: Vec<u8> },
    /// Active viewport camera matrices for gizmo projection. Sent whenever
    /// the camera moves (column-major view-projection + eye position).
    CameraInfo { view_proj: [f32; 16], eye: [f32; 3] },
    /// Exact pre/post transform pair after a gizmo drag (undo bookkeeping).
    GestureDone { entity: EntityId, label: String, pre: TransformAbs, post: TransformAbs },
    /// Full hierarchy refresh (cheap; sent on change and at 4 Hz).
    Hierarchy { nodes: Vec<HierNode> },
    /// Inspector payload for the selected entity.
    EntityComponents { entity: EntityId, name: String, components: Vec<ComponentData> },
    /// Batched log lines from the runtime's tracing layer.
    Logs(Vec<LogEntry>),
    Stats(Stats),
    PickResult { x: f32, y: f32, entity: Option<EntityId> },
    AnimState { state: AnimState, tracks: Vec<AnimEntityTracks> },
    EnvState(EnvironmentSettings),
    /// Current scene path + dirty flag.
    SceneInfo { path: Option<String>, dirty: bool },
    /// One-shot user-facing feedback ("Scene saved", errors, ...).
    Notice { level: LogLevel, message: String },
    ScreenshotDone { path: String, success: bool, message: String },
    /// Runtime is exiting (crash or `Shutdown`); carries the reason.
    Goodbye { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_hello() {
        let msg = Message::ToRuntime(EditorToRuntime::Hello);
        let bytes = postcard::to_allocvec(&msg).unwrap();
        let back: Message = postcard::from_bytes(&bytes).unwrap();
        assert!(matches!(back, Message::ToRuntime(EditorToRuntime::Hello)));
    }

    #[test]
    fn roundtrip_frame() {
        let msg = Message::ToEditor(RuntimeToEditor::Frame {
            width: 4,
            height: 4,
            rgb: vec![128u8; 48],
        });
        let bytes = postcard::to_allocvec(&msg).unwrap();
        let back: Message = postcard::from_bytes(&bytes).unwrap();
        match back {
            Message::ToEditor(RuntimeToEditor::Frame { width, height, rgb }) => {
                assert_eq!((width, height), (4, 4));
                assert_eq!(rgb.len(), 48);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_set_field() {
        let msg = Message::ToRuntime(EditorToRuntime::SetField {
            entity: 42,
            component: ComponentKind::Transform,
            field: ComponentField::Translation,
            value: FieldValue::Vec3([1.5, 2.0, -3.25]),
        });
        let bytes = postcard::to_allocvec(&msg).unwrap();
        let back: Message = postcard::from_bytes(&bytes).unwrap();
        assert!(matches!(
            back,
            Message::ToRuntime(EditorToRuntime::SetField { .. })
        ));
    }
}
