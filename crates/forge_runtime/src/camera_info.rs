//! Streams the active viewport camera's view-projection matrix and eye
//! position to the editor (`RuntimeToEditor::CameraInfo`).
//!
//! The editor paints the transform gizmo as a screen-space overlay, so it
//! needs the exact matrices the renderer uses. We broadcast only on change
//! (camera orbit, rig move, viewport resize, camera switch) — a 76-byte
//! message, negligible even at 60 Hz.

use bevy::camera::Projection;
use bevy::prelude::*;

use crate::state::{CameraSource, EditorOnlyTag, IpcChannels};

/// Remembers the last broadcast matrix so we only send on change.
#[derive(Default, Resource)]
pub struct LastCameraInfo(Option<[u8; 64]>);

/// Runs in `PostUpdate`; picks the viewport camera, computes
/// `view_proj = clip_from_view * view` (column-major flat) and sends
/// `CameraInfo` whenever the matrix bytes change.
pub fn push_camera_info(
    source: Res<CameraSource>,
    editor_cam: Query<(&GlobalTransform, &Camera, &Projection), (With<Camera3d>, With<EditorOnlyTag>)>,
    scene_cams: Query<(&GlobalTransform, &Camera, &Projection), (With<Camera3d>, Without<EditorOnlyTag>)>,
    mut last: ResMut<LastCameraInfo>,
    channels: Res<IpcChannels>,
) {
    let picked: Option<(&GlobalTransform, &Camera, &Projection)> = match &*source {
        CameraSource::Editor => editor_cam.iter().next(),
        CameraSource::Scene(e) => {
            // Fall back to the editor rig when the scene camera vanished.
            scene_cams.get(*e).ok().or_else(|| editor_cam.iter().next())
        }
    };
    let Some((world_from_cam, cam, proj)) = picked else {
        return;
    };
    if !cam.is_active {
        return;
    }

    let view = world_from_cam.to_matrix().inverse();
    let view_proj: Mat4 = proj.get_clip_from_view() * view;
    let eye = world_from_cam.translation();

    let flat: [f32; 16] = view_proj.to_cols_array();
    // Cheap change test: exact byte compare of the matrix.
    let bytes: [u8; 64] = flat
        .iter()
        .flat_map(|f| f.to_ne_bytes())
        .collect::<Vec<u8>>()
        .try_into()
        .unwrap_or([0u8; 64]);
    if last.0 == Some(bytes) {
        return;
    }
    last.0 = Some(bytes);

    let _ = channels.evt_tx.send(forge_ipc::RuntimeToEditor::CameraInfo {
        view_proj: flat,
        eye: eye.to_array(),
    });
}
