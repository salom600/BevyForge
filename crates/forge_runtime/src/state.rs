//! Shared runtime state: IPC channels, selection, viewport camera rig,
//! play-mode flags, environment holder, pending screenshots.

use bevy::prelude::*;

use forge_ipc::EnvironmentSettings;

use crate::scene_io::ForgeScene;

/// Cross-thread plumbing to the IPC relay threads.
#[derive(Resource)]
pub struct IpcChannels {
    pub cmd_rx: crossbeam_channel::Receiver<forge_ipc::EditorToRuntime>,
    pub evt_tx: crossbeam_channel::Sender<forge_ipc::RuntimeToEditor>,
}

/// Which entity renders the viewport: the editor rig or a scene camera.
#[derive(Debug, Clone, Copy, Resource)]
pub enum CameraSource {
    Editor,
    Scene(Entity),
}

/// Currently selected entity (hierarchy click / picking).
#[derive(Debug, Default, Resource)]
pub struct Selection(pub Option<Entity>);

/// Offscreen render target size (mirrors the editor viewport panel).
#[derive(Debug, Clone, Copy, Resource)]
pub struct ViewportSize {
    pub width: u32,
    pub height: u32,
}

/// Free camera orbit rig state; the editor owns the numbers, we apply them.
#[derive(Debug, Clone, Copy, Resource)]
pub struct ViewportRig {
    pub target: Vec3,
    pub distance: f32,
    pub yaw_deg: f32,
    pub pitch_deg: f32,
}

impl Default for ViewportRig {
    fn default() -> Self {
        Self { target: Vec3::ZERO, distance: 12.0, yaw_deg: -35.0, pitch_deg: 28.0 }
    }
}

impl ViewportRig {
    /// Camera position for the current spherical orbit parameters.
    pub fn eye(&self) -> Vec3 {
        let yaw = self.yaw_deg.to_radians();
        let pitch = self.pitch_deg.to_radians().clamp(-1.5, 1.5);
        Vec3::new(
            self.distance * pitch.cos() * yaw.cos(),
            self.distance * pitch.sin(),
            self.distance * pitch.cos() * yaw.sin(),
        ) + self.target
    }
}

/// Marks entities the hierarchy must hide and scene saves must skip.
#[derive(Component, Default)]
pub struct EditorOnlyTag;

/// Marks entities that belong to the user's scene (shown in hierarchy,
/// saved to scenes). Everything else (gizmo internals, editor rig) is hidden.
#[derive(Component, Default)]
pub struct UserScene;

/// Hierarchy lock state (prevents accidental delete/move).
#[derive(Component, Default)]
pub struct EditorLocked;

/// Edit-mode vs Play-mode gate for gameplay systems.
#[derive(Debug, Resource)]
pub struct PlayState {
    pub playing: bool,
}

/// System run condition for gameplay systems.
pub fn playing(play: Res<PlayState>) -> bool {
    play.playing
}

/// Which frames must be re-pushed to the editor.
#[derive(Debug, Clone, Default, Resource)]
pub struct RuntimeFlags {
    pub hierarchy_dirty: bool,
    pub components_dirty: bool,
    pub anim_dirty: bool,
    pub env_dirty: bool,
    pub scene_dirty: bool,
}

impl RuntimeFlags {
    pub fn all_dirty(&mut self) {
        *self = Self {
            hierarchy_dirty: true,
            components_dirty: true,
            anim_dirty: true,
            env_dirty: true,
            scene_dirty: true,
        };
    }
}

/// In-memory scene snapshot used by Play Mode rollback.
#[derive(Debug, Default, Resource)]
pub struct PlaySnapshot(pub Option<ForgeScene>);

/// Live environment render settings (mirrors the editor lighting panel).
#[derive(Debug, Clone, Resource)]
pub struct EnvironmentSettingsHolder(pub EnvironmentSettings);

/// A queued high-resolution screenshot request.
#[derive(Debug, Default, Resource)]
pub struct PendingScreenshot {
    /// (path, width, height)
    pub request: Option<(String, u32, u32)>,
}

/// CLI one-shot screenshot configuration (--screenshot out.png).
#[derive(Debug, Clone, Default, Resource)]
pub struct StartupShot {
    pub path: Option<String>,
    pub width: u32,
    pub height: u32,
}

/// Path of the currently open scene file (None = untitled).
#[derive(Debug, Default, Resource)]
pub struct ScenePath(pub Option<String>);

/// --init-demo: author the design-mirroring starter scene, save and exit.
#[derive(Debug, Default, Resource)]
pub struct InitDemo(pub bool);
