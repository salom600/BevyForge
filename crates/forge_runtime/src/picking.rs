//! Viewport click-to-select: manual ray cast through the active camera using
//! bevy's `ray_mesh_intersection` helper (no pointer input exists headless).

use bevy::camera::Projection;
use bevy::math::{Affine3A, Vec2};
use bevy::mesh::{Indices, Mesh, VertexAttributeValues};
use bevy::picking::mesh_picking::ray_cast::{ray_mesh_intersection, Backfaces};
use bevy::prelude::*;

use crate::state::{CameraSource, IpcChannels};

/// Answer a `Pick` command: ray from camera through viewport coordinates.
pub fn pick_at(world: &mut World, x: f32, y: f32) -> Option<Entity> {
    use crate::state::EditorOnlyTag;

    // Resolve the camera currently rendering the viewport.
    let camera = match *world.resource::<CameraSource>() {
        CameraSource::Editor => world
            .query_filtered::<Entity, (With<Camera3d>, With<EditorOnlyTag>)>()
            .iter(world)
            .next(),
        CameraSource::Scene(e) => Some(e),
    }?;
    if world.get::<Camera3d>(camera).is_none() {
        return None;
    }

    let (cam_transform, proj_clone) = {
        let gt = *world.get::<GlobalTransform>(camera)?;
        let proj = world.get::<Projection>(camera)?.clone();
        (gt, proj)
    };

    // Viewport pixel size: from the camera's viewport or the image target.
    let viewport_size = viewport_pixel_size(world, camera)?;

    // Editor sends normalised (0..1, y down from top-left like screen space).
    let px = Vec2::new(x * viewport_size.x, (1.0 - y) * viewport_size.y);

    let cam = world.get::<Camera>(camera).cloned()?;
    let ray = cam
        .viewport_to_world(&cam_transform, px)
        .ok()?;

    // Iterate meshes, keep the closest hit (query state first, then the
    // immutable asset resource so borrows coexist).
    let mut q = world.query::<(Entity, &Mesh3d, &GlobalTransform)>();
    let meshes = world.resource::<Assets<Mesh>>();
    let mut best: Option<(Entity, f32)> = None;
    for (entity, mesh3d, gt) in q.iter(world) {
        let Some(mesh) = meshes.get(&mesh3d.0) else { continue };
        let positions = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(VertexAttributeValues::Float32x3(v)) => v.as_slice(),
            _ => continue,
        };
        let normals = match mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
            Some(VertexAttributeValues::Float32x3(v)) => Some(v.as_slice()),
            _ => None,
        };
        let indices: Option<Vec<u32>> = mesh.indices().map(|i| match i {
            Indices::U16(v) => v.iter().map(|x| *x as u32).collect(),
            Indices::U32(v) => v.to_vec(),
        });
        let hit = match indices {
            Some(indices) => ray_mesh_intersection(
                ray,
                &Affine3A::from(gt.affine()),
                positions,
                normals,
                Some(&indices[..]),
                None,
                Backfaces::Cull,
            ),
            None => None,
        };
        if let Some(hit) = hit {
            if best.map(|(_, d)| hit.distance < d).unwrap_or(true) {
                best = Some((entity, hit.distance));
            }
        }
    }
    let _ = proj_clone;
    best.map(|(e, _)| e)
}

fn viewport_pixel_size(world: &World, camera: Entity) -> Option<Vec2> {
    let cam = world.get::<Camera>(camera)?;
    if let Some(vp) = cam.viewport.clone() {
        return Some(Vec2::new(vp.physical_size.x as f32, vp.physical_size.y as f32));
    }
    // 0.19: the render target lives on its own component.
    match world.get::<bevy::camera::RenderTarget>(camera)? {
        bevy::camera::RenderTarget::Image(target) => {
            let images = world.get_resource::<Assets<Image>>()?;
            let image = images.get(&target.handle)?;
            Some(Vec2::new(image.width() as f32, image.height() as f32))
        }
        _ => None,
    }
}

/// System wrapper pushing pick results back to the editor; the actual cast is
/// performed inside the command executor (needs `&mut World`).
pub fn _placeholder() {
    let _ = std::any::TypeId::of::<IpcChannels>();
}
