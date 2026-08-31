//! The shared on-disk / on-the-wire scene document (`*.scn.ron`).
//!
//! Both processes depend on these types:
//!
//! * the **runtime** serialises its world into a [`ForgeScene`] when saving
//!   and rebuilds the world from one when loading;
//! * the **editor** seeds its offline document from the project's scene file,
//!   applies edit commands to it while the engine is down, writes it back on
//!   offline saves and ships it to the runtime with
//!   [`crate::EditorToRuntime::LoadSceneDoc`] when the engine reconnects.
//!
//! Types here must stay dependency-free (serde only) so the editor binary
//! never links Bevy.

/// Root document of a `*.scn.ron` file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ForgeScene {
    pub engine: String,
    pub environment: crate::EnvironmentSettings,
    #[serde(default)]
    pub animation: SceneAnimation,
    pub entities: Vec<SceneEntity>,
}

impl Default for ForgeScene {
    fn default() -> Self {
        Self {
            engine: format!("bevyforge {}", env!("CARGO_PKG_VERSION")),
            environment: crate::EnvironmentSettings::default(),
            animation: SceneAnimation::default(),
            entities: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SceneAnimation {
    #[serde(default)]
    pub duration: f32,
    #[serde(default)]
    pub entries: Vec<SceneAnimEntry>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SceneAnimEntry {
    pub name: String,
    pub tracks: Vec<(crate::AnimTrack, Vec<(f32, [f32; 3])>)>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SceneEntity {
    pub name: String,
    #[serde(default)]
    pub parent: Option<String>,
    pub kind: SceneEntityKind,
    /// (translation, rotation euler XYZ deg, scale)
    pub transform: ([f32; 3], [f32; 3], [f32; 3]),
    #[serde(default = "default_true")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub material: Option<SceneMaterial>,
    #[serde(default)]
    pub camera: Option<SceneCamera>,
    #[serde(default)]
    pub light: Option<SceneLight>,
    #[serde(default)]
    pub scripts: Vec<SceneScript>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SceneEntityKind {
    Empty,
    Mesh(crate::MeshPrimitive),
    Camera,
    DirectionalLight,
    PointLight,
    SpotLight,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SceneMaterial {
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: [f32; 4],
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SceneCamera {
    pub fov_deg: f32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SceneLight {
    Directional { color: [f32; 4], illuminance: f32, shadows: bool },
    Point { color: [f32; 4], intensity: f32, radius: f32, shadows: bool },
    Spot { color: [f32; 4], intensity: f32, range: f32, outer_angle_deg: f32, shadows: bool },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SceneScript {
    Rotator { speed: [f32; 3] },
    Orbiter { center: [f32; 3], radius: f32, speed: f32 },
    LinearMover { velocity: [f32; 3], ping_pong: bool },
    PingPongMover { offset: [f32; 3], period: f32 },
    Player { speed: f32, jump_force: f32, sprint_multiplier: f32 },
    CharacterController { height: f32, radius: f32, step_offset: f32, slope_limit: f32 },
    Health { current: f32, max: f32 },
    Inventory { slots: u32 },
}

impl SceneEntity {
    /// Identity transform helper for fresh entities.
    pub fn identity_transform() -> ([f32; 3], [f32; 3], [f32; 3]) {
        ([0.0, 0.0, 0.0], [0.0, 0.0, 0.0], [1.0, 1.0, 1.0])
    }
}
