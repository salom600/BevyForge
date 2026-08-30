//! Viewport overlays: reference grid and the selection outline/axes.

use bevy::color::Color;
use bevy::math::{Isometry3d, Vec3};
use bevy::prelude::*;

use crate::factory::SourceMesh;
use forge_ipc::MeshPrimitive;
use crate::state::{EnvironmentSettingsHolder, Selection};

/// Approximate half-extents per primitive for outline sizing.
fn half_extents(prim: MeshPrimitive) -> Vec3 {
    match prim {
        MeshPrimitive::Cube => Vec3::new(0.5, 0.5, 0.5),
        MeshPrimitive::Sphere => Vec3::new(0.5, 0.5, 0.5),
        MeshPrimitive::Icosphere => Vec3::new(0.5, 0.5, 0.5),
        MeshPrimitive::Capsule => Vec3::new(0.5, 1.0, 0.5),
        MeshPrimitive::Cylinder => Vec3::new(0.5, 1.0, 0.5),
        MeshPrimitive::Cone => Vec3::new(0.5, 1.0, 0.5),
        MeshPrimitive::Plane => Vec3::new(1.0, 0.01, 1.0),
        MeshPrimitive::Torus => Vec3::new(0.5, 0.15, 0.5),
    }
}

fn grid_color(alpha: f32) -> Color {
    Color::srgba(0.28, 0.34, 0.46, alpha)
}

const SELECTION_COLOR: Color = Color::srgb(0.95, 0.55, 0.20);

/// Draws the ground grid + selection gizmos every frame.
pub fn draw_editor_overlays(
    mut gizmos: Gizmos,
    env: Res<EnvironmentSettingsHolder>,
    selection: Res<Selection>,
    transforms: Query<(&Transform, Option<&SourceMesh>)>,
) {
    if env.0.show_grid {
        // 40m x 40m grid, 1m cells, lifted 1mm to avoid z-fighting with geometry.
        gizmos.grid_3d(
            Isometry3d::from_translation(Vec3::new(0.0, 0.001, 0.0)),
            bevy::math::UVec3::new(40, 1, 40),
            Vec3::new(1.0, 1.0, 1.0),
            grid_color(0.55),
        );
        // Axis lines for orientation (X red, Z blue).
        gizmos.line(
            Vec3::new(-20.0, 0.002, 0.0),
            Vec3::new(20.0, 0.002, 0.0),
            Color::srgba(0.86, 0.24, 0.28, 0.8),
        );
        gizmos.line(
            Vec3::new(0.0, 0.002, -20.0),
            Vec3::new(0.0, 0.002, 20.0),
            Color::srgba(0.24, 0.42, 0.86, 0.8),
        );
    }

    if env.0.show_selection_outline {
        let Some(entity) = selection.0 else { return };
        let Ok((transform, mesh)) = transforms.get(entity) else { return };
        let extents = mesh.map(|m| half_extents(m.0)).unwrap_or(Vec3::new(0.35, 0.35, 0.35));
        let scaled = extents * transform.scale.max(Vec3::splat(0.001));
        draw_wire_box(&mut gizmos, transform, scaled, SELECTION_COLOR);

        // Transform axes at the selection origin.
        let origin = transform.translation;
        let right = transform.rotation * Vec3::X;
        let up = transform.rotation * Vec3::Y;
        let forward = transform.rotation * Vec3::Z;
        let len = 1.2;
        gizmos.line(origin, origin + right * len, Color::srgb(0.86, 0.24, 0.28));
        gizmos.line(origin, origin + up * len, Color::srgb(0.32, 0.78, 0.36));
        gizmos.line(origin, origin + forward * len, Color::srgb(0.24, 0.42, 0.86));
    }
}

/// Wireframe box around `transform` with `half_extents` (scaled), 12 edges.
fn draw_wire_box(gizmos: &mut Gizmos, t: &Transform, half: Vec3, color: Color) {
    let rot = t.rotation;
    let c = t.translation;
    let corners = [
        Vec3::new(-1.0, -1.0, -1.0),
        Vec3::new(1.0, -1.0, -1.0),
        Vec3::new(1.0, 1.0, -1.0),
        Vec3::new(-1.0, 1.0, -1.0),
        Vec3::new(-1.0, -1.0, 1.0),
        Vec3::new(1.0, -1.0, 1.0),
        Vec3::new(1.0, 1.0, 1.0),
        Vec3::new(-1.0, 1.0, 1.0),
    ]
    .map(|sign| c + rot * (sign * half));
    let edges: [(usize, usize); 12] = [
        (0, 1), (1, 2), (2, 3), (3, 0),
        (4, 5), (5, 6), (6, 7), (7, 4),
        (0, 4), (1, 5), (2, 6), (3, 7),
    ];
    for (a, b) in edges {
        gizmos.line(corners[a], corners[b], color);
    }
}
