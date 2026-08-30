//! Entity spawning, component inspection and typed field application.
//!
//! Every component the editor can see flows through here as the *typed*
//! protocol model ([`forge_ipc::ComponentData`]) — no reflection parsing, no
//! RON in the UI, everything compile-checked on both sides.

use bevy::camera::Projection;
use bevy::color::Color;
use bevy::light::{DirectionalLight, PointLight, SpotLight};
use bevy::math::{EulerRot, Quat, Vec3};
use bevy::pbr::StandardMaterial;
use bevy::prelude::*;

use forge_ipc::{
    ComponentData, ComponentField, ComponentKind, EntityKind, FieldRow, FieldValue,
    HierNode, MeshPrimitive, NodeIcon,
};

use forge_scripts as scripts;
use crate::state::{EditorLocked, UserScene};

// ---------------------------------------------------------------------------
// Name helpers
// ---------------------------------------------------------------------------

/// Blender-style unique name ("Cube" → "Cube.001" while taken).
pub fn unique_name(world: &mut World, base: &str) -> String {
    let mut taken = std::collections::HashSet::new();
    let mut q = world.query::<Option<&Name>>();
    for name in q.iter(world) {
        if let Some(n) = name {
            taken.insert(n.as_str().to_string());
        }
    }
    if !taken.contains(base) {
        return base.to_string();
    }
    for i in 1..10_000u32 {
        let candidate = format!("{base}.{i:03}");
        if !taken.contains(&candidate) {
            return candidate;
        }
    }
    format!("{base}.dup")
}

pub fn entity_name(world: &World, entity: Entity) -> String {
    world
        .get::<Name>(entity)
        .map(|n| n.as_str().to_string())
        .unwrap_or_else(|| format!("{entity:?}"))
}

// ---------------------------------------------------------------------------
// Spawning
// ---------------------------------------------------------------------------

/// Approximate half-height per primitive, used to rest new meshes on the grid.
fn rest_height(prim: MeshPrimitive) -> f32 {
    match prim {
        MeshPrimitive::Cube => 0.5,
        MeshPrimitive::Sphere => 0.5,
        MeshPrimitive::Icosphere => 0.5,
        MeshPrimitive::Capsule => 1.0,
        MeshPrimitive::Cylinder => 1.0,
        MeshPrimitive::Cone => 1.0,
        MeshPrimitive::Plane => 0.0,
        MeshPrimitive::Torus => 0.25,
    }
}

/// Build the Bevy mesh for a protocol primitive kind.
pub fn build_mesh(prim: MeshPrimitive) -> Mesh {
    match prim {
        MeshPrimitive::Cube => Mesh::from(Cuboid::new(1.0, 1.0, 1.0)),
        MeshPrimitive::Sphere => Mesh::from(Sphere::new(0.5)),
        MeshPrimitive::Icosphere => Mesh::from(
            Sphere::new(0.5)
                .mesh()
                .kind(bevy::mesh::primitives::SphereKind::Ico { subdivisions: 3 }),
        ),
        MeshPrimitive::Capsule => Mesh::from(Capsule3d::new(0.5, 1.0)),
        MeshPrimitive::Cylinder => Mesh::from(Cylinder::new(0.5, 2.0)),
        MeshPrimitive::Cone => Mesh::from(
            bevy::math::primitives::Cone { radius: 0.5, height: 2.0 }.mesh(),
        ),
        MeshPrimitive::Plane => Mesh::from(Plane3d::default().mesh().size(2.0, 2.0)),
        MeshPrimitive::Torus => Mesh::from(Torus::new(0.35, 0.15)),
    }
}

/// Runtime-only marker recording which primitive generated a mesh (used by
/// the inspector and deterministic scene saves).
#[derive(Component, Clone, Copy, Debug)]
pub struct SourceMesh(pub MeshPrimitive);

/// Spawns an entity of `kind` with a unique name; returns the entity.
pub fn spawn_entity(world: &mut World, base_name: &str, parent: Option<Entity>, kind: EntityKind) -> Entity {
    let name = unique_name(world, base_name);

    // Pre-build asset handles so the spawn borrow stays short.
    let meshed: Option<(MeshPrimitive, Handle<Mesh>, Handle<StandardMaterial>)> = match kind {
        EntityKind::Mesh(prim) => {
            let mesh = {
                let mut meshes = world.resource_mut::<Assets<Mesh>>();
                meshes.add(build_mesh(prim))
            };
            let material = {
                let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
                let material = if kind == EntityKind::PlayerPrefab {
                    StandardMaterial {
                        base_color: Color::srgb(0.95, 0.62, 0.28),
                        metallic: 0.1,
                        perceptual_roughness: 0.45,
                        emissive: bevy::color::LinearRgba::rgb(0.08, 0.03, 0.0),
                        ..default()
                    }
                } else {
                    StandardMaterial {
                        base_color: Color::srgb(0.65, 0.68, 0.72),
                        metallic: 0.05,
                        perceptual_roughness: 0.65,
                        ..default()
                    }
                };
                materials.add(material)
            };
            Some((prim, mesh, material))
        }
        EntityKind::PlayerPrefab => {
            let mesh = {
                let mut meshes = world.resource_mut::<Assets<Mesh>>();
                meshes.add(build_mesh(MeshPrimitive::Capsule))
            };
            let material = {
                let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
                materials.add(StandardMaterial {
                    base_color: Color::srgb(0.95, 0.62, 0.28),
                    metallic: 0.1,
                    perceptual_roughness: 0.45,
                    emissive: bevy::color::LinearRgba::rgb(0.08, 0.03, 0.0),
                    ..default()
                })
            };
            Some((MeshPrimitive::Capsule, mesh, material))
        }
        _ => None,
    };

    let mut entity_cmd = world.spawn((Name::new(name), UserScene));
    if let Some(parent) = parent {
        entity_cmd.insert(bevy::ecs::hierarchy::ChildOf(parent));
    }

    match kind {
        EntityKind::Empty => {}
        EntityKind::Mesh(prim) => {
            if let Some((_, mesh, material)) = meshed {
                entity_cmd.insert((
                    Mesh3d(mesh),
                    MeshMaterial3d(material),
                    SourceMesh(prim),
                    Transform::from_xyz(0.0, rest_height(prim), 0.0),
                ));
            }
        }
        EntityKind::Camera => {
            // Scene cameras stay inactive until the Game tab renders through
            // them (headless build has no window target to bind by default).
            entity_cmd.insert((Camera3d::default(), Transform::from_xyz(0.0, 2.0, 6.0)));
        }
        EntityKind::DirectionalLight => {
            entity_cmd.insert((
                DirectionalLight { shadow_maps_enabled: true, ..default() },
                Transform::from_xyz(0.0, 12.0, 0.0).looking_at(Vec3::ZERO, Vec3::Y),
            ));
        }
        EntityKind::PointLight => {
            entity_cmd.insert((
                PointLight { intensity: 200_000.0, shadow_maps_enabled: true, ..default() },
                Transform::from_xyz(0.0, 2.5, 0.0),
            ));
        }
        EntityKind::SpotLight => {
            entity_cmd.insert((
                SpotLight { intensity: 400_000.0, shadow_maps_enabled: true, ..default() },
                Transform::from_xyz(0.0, 4.0, 0.0).looking_at(Vec3::ZERO, Vec3::Y),
            ));
        }
        EntityKind::PlayerPrefab => {
            if let Some((prim, mesh, material)) = meshed {
                entity_cmd.insert((
                    Mesh3d(mesh),
                    MeshMaterial3d(material),
                    SourceMesh(prim),
                    Transform::from_xyz(0.0, 1.0, 0.0),
                    scripts::Player::default(),
                    scripts::CharacterController::default(),
                    scripts::Health::default(),
                    scripts::Inventory::default(),
                ));
            }
        }
    }
    entity_cmd.id()
}

/// Default rig spawned for fresh scenes: Main Camera + Directional Light.
pub fn spawn_default_rig_contents(world: &mut World) -> (Entity, Entity) {
    let camera = {
        let name = unique_name(world, "Main Camera");
        let mut e = world.spawn((Name::new(name), crate::state::UserScene, Camera3d::default()));
        e.insert(Transform::from_xyz(-4.0, 3.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y));
        e.id()
    };
    let sun = {
        let name = unique_name(world, "Directional Light");
        let mut e = world.spawn((
            Name::new(name),
            crate::state::UserScene,
            DirectionalLight {
                illuminance: 12_000.0,
                shadow_maps_enabled: true,
                ..default()
            },
        ));
        e.insert(Transform::from_xyz(0.0, 12.0, 0.0).looking_at(Vec3::ZERO, Vec3::Y));
        e.id()
    };
    (camera, sun)
}

// ---------------------------------------------------------------------------
// Hierarchy snapshot
// ---------------------------------------------------------------------------

fn node_icon(world: &World, entity: Entity) -> NodeIcon {
    if world.get::<Camera3d>(entity).is_some() {
        NodeIcon::Camera
    } else if world.get::<DirectionalLight>(entity).is_some()
        || world.get::<PointLight>(entity).is_some()
        || world.get::<SpotLight>(entity).is_some()
    {
        NodeIcon::Light
    } else if world.get::<scripts::Player>(entity).is_some() {
        NodeIcon::Player
    } else if world.get::<Mesh3d>(entity).is_some() {
        NodeIcon::Mesh
    } else if world.get::<scripts::Rotator>(entity).is_some()
        || world.get::<scripts::Orbiter>(entity).is_some()
        || world.get::<scripts::LinearMover>(entity).is_some()
        || world.get::<scripts::PingPongMover>(entity).is_some()
    {
        NodeIcon::Script
    } else {
        NodeIcon::Group
    }
}

/// Build the flattened hierarchy for the editor (depth-first, users only).
pub fn build_hierarchy(world: &mut World, selected: Option<Entity>) -> Vec<HierNode> {
    use bevy::ecs::hierarchy::ChildOf;
    use bevy::camera::visibility::Visibility as Vis;

    let mut out = Vec::new();
    let mut to_visit: Vec<(Entity, u32)> = Vec::new();

    // Roots: user-scene entities without ChildOf.
    {
        let mut q = world.query::<(Entity, Option<&ChildOf>, Option<&UserScene>)>();
        let mut roots: Vec<Entity> = q
            .iter(world)
            .filter_map(|(e, parent, user)| {
                if parent.is_none() && user.is_some() {
                    Some(e)
                } else {
                    None
                }
            })
            .collect();
        roots.sort_by_key(|e| world.get::<Name>(*e).map(|n| n.as_str().to_string()).unwrap_or_default());
        to_visit.extend(roots.into_iter().map(|e| (e, 0)));
    }

    // Iterative DFS (depth-capped as a belt-and-braces guard).
    while let Some((entity, depth)) = to_visit.pop() {
        if depth > 64 {
            continue;
        }
        let name = entity_name(world, entity);
        let icon = node_icon(world, entity);
        let visible = matches!(world.get::<Vis>(entity), None | Some(Vis::Visible) | Some(Vis::Inherited));
        let locked = world.get::<EditorLocked>(entity).is_some();
        let children: Vec<Entity> = world
            .get::<bevy::ecs::hierarchy::Children>(entity)
            .map(|c| c.to_vec())
            .unwrap_or_default()
            .into_iter()
            .filter(|c| world.get::<UserScene>(*c).is_some())
            .collect();
        out.push(HierNode {
            id: entity.to_bits(),
            name,
            icon,
            visible,
            locked,
            has_children: !children.is_empty(),
            depth,
            selected: selected == Some(entity),
        });
        // Reverse so the pop order preserves name-sorted children order.
        let mut kids: Vec<(Entity, String)> = children
            .into_iter()
            .map(|c| (c, entity_name(world, c)))
            .collect();
        kids.sort_by(|a, b| a.1.cmp(&b.1));
        to_visit.extend(kids.into_iter().rev().map(|(e, _)| (e, depth + 1)));
    }
    out
}

// ---------------------------------------------------------------------------
// Inspector extraction
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn row(label: &str, value: FieldValue, unit: Option<&str>) -> (ComponentField, FieldRow) {
    // The field kind is derived from the value to keep rows self-describing.
    let field = match value {
        FieldValue::F32(_) => ComponentField::HealthCurrent, // overridden below per component
        _ => ComponentField::HealthCurrent,
    };
    (
        field,
        FieldRow { label: label.to_string(), value, unit: unit.map(|s| s.to_string()) },
    )
}

fn v3(v: Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

fn q_euler_deg(q: &Quat) -> [f32; 3] {
    let (x, y, z) = q.to_euler(EulerRot::XYZ);
    [x.to_degrees(), y.to_degrees(), z.to_degrees()]
}

/// Extract the full inspector payload for one entity.
pub fn extract_components(world: &mut World, entity: Entity) -> (String, Vec<ComponentData>) {
    let name = entity_name(world, entity);
    let mut components: Vec<ComponentData> = Vec::new();

    if let Some(t) = world.get::<Transform>(entity) {
        components.push(ComponentData {
            kind: ComponentKind::Transform,
            rows: vec![
                (ComponentField::Translation, FieldRow {
                    label: "Translation".into(),
                    value: FieldValue::Vec3(v3(t.translation)),
                    unit: None,
                }),
                (ComponentField::RotationEulerDeg, FieldRow {
                    label: "Rotation".into(),
                    value: FieldValue::Vec3(q_euler_deg(&t.rotation)),
                    unit: Some("deg".into()),
                }),
                (ComponentField::Scale, FieldRow {
                    label: "Scale".into(),
                    value: FieldValue::Vec3(v3(t.scale)),
                    unit: None,
                }),
            ],
        });
    }

    if let Some(vis) = world.get::<Visibility>(entity) {
        let visible = !matches!(vis, Visibility::Hidden);
        components.push(ComponentData {
            kind: ComponentKind::Visibility,
            rows: vec![(
                ComponentField::EntityVisible,
                FieldRow { label: "Visible".into(), value: FieldValue::Bool(visible), unit: None },
            )],
        });
    }

    if let Some(src) = world.get::<SourceMesh>(entity) {
        components.push(ComponentData {
            kind: ComponentKind::Mesh,
            rows: vec![(
                ComponentField::MeshPrimitiveKind,
                FieldRow {
                    label: "Primitive".into(),
                    value: FieldValue::Mesh(src.0),
                    unit: None,
                },
            )],
        });
    }

    if let Some(mat_handle) = world.get::<MeshMaterial3d<StandardMaterial>>(entity) {
        if let Some(materials) = world.get_resource::<Assets<StandardMaterial>>() {
            if let Some(m) = materials.get(&mat_handle.0) {
                let c = m.base_color.to_srgba();
                let e = m.emissive;
                components.push(ComponentData {
                    kind: ComponentKind::Material,
                    rows: vec![
                        (ComponentField::BaseColor, FieldRow {
                            label: "Base Color".into(),
                            value: FieldValue::Rgba([c.red, c.green, c.blue, c.alpha]),
                            unit: None,
                        }),
                        (ComponentField::Metallic, FieldRow {
                            label: "Metallic".into(),
                            value: FieldValue::F32(m.metallic),
                            unit: None,
                        }),
                        (ComponentField::Roughness, FieldRow {
                            label: "Roughness".into(),
                            value: FieldValue::F32(m.perceptual_roughness),
                            unit: None,
                        }),
                        (ComponentField::Emissive, FieldRow {
                            label: "Emissive".into(),
                            value: FieldValue::Rgba([e.red, e.green, e.blue, e.alpha]),
                            unit: None,
                        }),
                    ],
                });
            }
        }
    }

    if world.get::<Camera>(entity).is_some() {
        let fov = match world.get::<Projection>(entity) {
            Some(Projection::Perspective(p)) => p.fov.to_degrees(),
            _ => 45.0,
        };
        components.push(ComponentData {
            kind: ComponentKind::Camera,
            rows: vec![
                (ComponentField::FovDeg, FieldRow {
                    label: "FOV".into(),
                    value: FieldValue::F32(fov),
                    unit: Some("deg".into()),
                }),
                (ComponentField::CameraHdr, FieldRow {
                    label: "HDR".into(),
                    value: FieldValue::Bool(world.get::<bevy::camera::Hdr>(entity).is_some()),
                    unit: None,
                }),
            ],
        });
    }

    if let Some(light) = world.get::<DirectionalLight>(entity) {
        let c = light.color.to_srgba();
        components.push(ComponentData {
            kind: ComponentKind::DirectionalLight,
            rows: vec![
                (ComponentField::SunColor, FieldRow {
                    label: "Color".into(),
                    value: FieldValue::Rgba([c.red, c.green, c.blue, c.alpha]),
                    unit: None,
                }),
                (ComponentField::SunIlluminance, FieldRow {
                    label: "Illuminance".into(),
                    value: FieldValue::F32(light.illuminance),
                    unit: Some("lux".into()),
                }),
                (ComponentField::SunShadows, FieldRow {
                    label: "Shadow Maps".into(),
                    value: FieldValue::Bool(light.shadow_maps_enabled),
                    unit: None,
                }),
            ],
        });
    }

    if let Some(light) = world.get::<PointLight>(entity) {
        let c = light.color.to_srgba();
        components.push(ComponentData {
            kind: ComponentKind::PointLight,
            rows: vec![
                (ComponentField::LightColor, FieldRow {
                    label: "Color".into(),
                    value: FieldValue::Rgba([c.red, c.green, c.blue, c.alpha]),
                    unit: None,
                }),
                (ComponentField::LightIntensity, FieldRow {
                    label: "Intensity".into(),
                    value: FieldValue::F32(light.intensity),
                    unit: Some("lm".into()),
                }),
                (ComponentField::LightRadius, FieldRow {
                    label: "Radius".into(),
                    value: FieldValue::F32(light.radius),
                    unit: None,
                }),
                (ComponentField::LightShadows, FieldRow {
                    label: "Shadow Maps".into(),
                    value: FieldValue::Bool(light.shadow_maps_enabled),
                    unit: None,
                }),
            ],
        });
    }

    if let Some(light) = world.get::<SpotLight>(entity) {
        let c = light.color.to_srgba();
        components.push(ComponentData {
            kind: ComponentKind::SpotLight,
            rows: vec![
                (ComponentField::LightColor, FieldRow {
                    label: "Color".into(),
                    value: FieldValue::Rgba([c.red, c.green, c.blue, c.alpha]),
                    unit: None,
                }),
                (ComponentField::LightIntensity, FieldRow {
                    label: "Intensity".into(),
                    value: FieldValue::F32(light.intensity),
                    unit: Some("lm".into()),
                }),
                (ComponentField::LightRange, FieldRow {
                    label: "Range".into(),
                    value: FieldValue::F32(light.range),
                    unit: None,
                }),
                (ComponentField::LightOuterAngleDeg, FieldRow {
                    label: "Outer Angle".into(),
                    value: FieldValue::F32(light.outer_angle.to_degrees()),
                    unit: Some("deg".into()),
                }),
                (ComponentField::LightShadows, FieldRow {
                    label: "Shadow Maps".into(),
                    value: FieldValue::Bool(light.shadow_maps_enabled),
                    unit: None,
                }),
            ],
        });
    }

    if let Some(r) = world.get::<scripts::Rotator>(entity) {
        components.push(ComponentData {
            kind: ComponentKind::Rotator,
            rows: vec![(
                ComponentField::RotatorSpeed,
                FieldRow { label: "Speed (rad/s)".into(), value: FieldValue::Vec3(v3(r.speed)), unit: None },
            )],
        });
    }
    if let Some(o) = world.get::<scripts::Orbiter>(entity) {
        components.push(ComponentData {
            kind: ComponentKind::Orbiter,
            rows: vec![
                (ComponentField::OrbiterCenter, FieldRow {
                    label: "Center".into(), value: FieldValue::Vec3(v3(o.center)), unit: None,
                }),
                (ComponentField::OrbiterRadius, FieldRow {
                    label: "Radius".into(), value: FieldValue::F32(o.radius), unit: None,
                }),
                (ComponentField::OrbiterSpeed, FieldRow {
                    label: "Speed".into(), value: FieldValue::F32(o.speed), unit: Some("rad/s".into()),
                }),
            ],
        });
    }
    if let Some(m) = world.get::<scripts::LinearMover>(entity) {
        components.push(ComponentData {
            kind: ComponentKind::LinearMover,
            rows: vec![
                (ComponentField::MoverVelocity, FieldRow {
                    label: "Velocity".into(), value: FieldValue::Vec3(v3(m.velocity)), unit: None,
                }),
                (ComponentField::MoverPingPong, FieldRow {
                    label: "Ping Pong".into(), value: FieldValue::Bool(m.ping_pong), unit: None,
                }),
            ],
        });
    }
    if let Some(m) = world.get::<scripts::PingPongMover>(entity) {
        components.push(ComponentData {
            kind: ComponentKind::PingPongMover,
            rows: vec![
                (ComponentField::PingPongOffset, FieldRow {
                    label: "Offset".into(), value: FieldValue::Vec3(v3(m.offset)), unit: None,
                }),
                (ComponentField::PingPongPeriod, FieldRow {
                    label: "Period".into(), value: FieldValue::F32(m.period), unit: Some("s".into()),
                }),
            ],
        });
    }
    if let Some(p) = world.get::<scripts::Player>(entity) {
        components.push(ComponentData {
            kind: ComponentKind::Player,
            rows: vec![
                (ComponentField::PlayerSpeed, FieldRow {
                    label: "speed".into(), value: FieldValue::F32(p.speed), unit: None,
                }),
                (ComponentField::PlayerJumpForce, FieldRow {
                    label: "jump_force".into(), value: FieldValue::F32(p.jump_force), unit: None,
                }),
                (ComponentField::PlayerSprintMultiplier, FieldRow {
                    label: "sprint_multiplier".into(), value: FieldValue::F32(p.sprint_multiplier), unit: None,
                }),
            ],
        });
    }
    if let Some(c) = world.get::<scripts::CharacterController>(entity) {
        components.push(ComponentData {
            kind: ComponentKind::CharacterController,
            rows: vec![
                (ComponentField::CcHeight, FieldRow {
                    label: "height".into(), value: FieldValue::F32(c.height), unit: None,
                }),
                (ComponentField::CcRadius, FieldRow {
                    label: "radius".into(), value: FieldValue::F32(c.radius), unit: None,
                }),
                (ComponentField::CcStepOffset, FieldRow {
                    label: "step_offset".into(), value: FieldValue::F32(c.step_offset), unit: None,
                }),
                (ComponentField::CcSlopeLimit, FieldRow {
                    label: "slope_limit".into(), value: FieldValue::F32(c.slope_limit), unit: Some("deg".into()),
                }),
            ],
        });
    }
    if let Some(h) = world.get::<scripts::Health>(entity) {
        components.push(ComponentData {
            kind: ComponentKind::Health,
            rows: vec![
                (ComponentField::HealthCurrent, FieldRow {
                    label: "current".into(), value: FieldValue::F32(h.current), unit: None,
                }),
                (ComponentField::HealthMax, FieldRow {
                    label: "max".into(), value: FieldValue::F32(h.max), unit: None,
                }),
            ],
        });
    }
    if let Some(i) = world.get::<scripts::Inventory>(entity) {
        components.push(ComponentData {
            kind: ComponentKind::Inventory,
            rows: vec![(
                ComponentField::InventorySlots,
                FieldRow { label: "slots".into(), value: FieldValue::U32(i.slots), unit: None },
            )],
        });
    }

    (name, components)
}

/// Post a single component row update (helper keeping the `row` fn honest).
#[allow(dead_code)]
pub fn make_row(field: ComponentField, label: &str, value: FieldValue, unit: Option<&str>) -> (ComponentField, FieldRow) {
    (field, FieldRow { label: label.to_string(), value, unit: unit.map(|s| s.to_string()) })
}

// ---------------------------------------------------------------------------
// Field application
// ---------------------------------------------------------------------------

fn value_v3(v: FieldValue) -> Option<Vec3> {
    match v {
        FieldValue::Vec3(a) => Some(Vec3::new(a[0], a[1], a[2])),
        _ => None,
    }
}

fn color_from(v: [f32; 4]) -> Color {
    Color::srgba(v[0], v[1], v[2], v[3])
}

/// Apply one typed field edit to an entity's component or material asset.
pub fn apply_set_field(
    world: &mut World,
    entity: Entity,
    component: ComponentKind,
    field: ComponentField,
    value: FieldValue,
) -> Result<(), String> {
    let f32_of = |v: FieldValue| match v {
        FieldValue::F32(f) => Some(f),
        _ => None,
    };

    match (component, field) {
        (ComponentKind::Transform, ComponentField::Translation) => {
            if let Some(mut t) = world.get_mut::<Transform>(entity) {
                if let Some(v) = value_v3(value) {
                    t.translation = v;
                    return Ok(());
                }
            }
        }
        (ComponentKind::Transform, ComponentField::RotationEulerDeg) => {
            if let Some(mut t) = world.get_mut::<Transform>(entity) {
                if let Some(v) = value_v3(value) {
                    t.rotation = Quat::from_euler(
                        EulerRot::XYZ,
                        v.x.to_radians(),
                        v.y.to_radians(),
                        v.z.to_radians(),
                    );
                    return Ok(());
                }
            }
        }
        (ComponentKind::Transform, ComponentField::Scale) => {
            if let Some(mut t) = world.get_mut::<Transform>(entity) {
                if let Some(v) = value_v3(value) {
                    t.scale = v;
                    return Ok(());
                }
            }
        }
        (ComponentKind::Visibility, ComponentField::EntityVisible) => {
            if let Some(mut vis) = world.get_mut::<Visibility>(entity) {
                *vis = if matches!(value, FieldValue::Bool(true)) {
                    Visibility::Visible
                } else {
                    Visibility::Hidden
                };
                return Ok(());
            }
        }
        (ComponentKind::Mesh, ComponentField::MeshPrimitiveKind) => {
            if let Some(FieldValue::Mesh(prim)) = Some(value) {
                let mut meshes = world.resource_mut::<Assets<Mesh>>();
                let new_mesh = meshes.add(build_mesh(prim));
                drop(meshes);
                if let Some(mut mesh3d) = world.get_mut::<Mesh3d>(entity) {
                    mesh3d.0 = new_mesh;
                }
                if let Some(mut src) = world.get_mut::<SourceMesh>(entity) {
                    src.0 = prim;
                }
                return Ok(());
            }
        }
        (ComponentKind::Material, _) => {
            let handle = world
                .get::<MeshMaterial3d<StandardMaterial>>(entity)
                .map(|m| m.0.clone());
            if let Some(handle) = handle {
                if let Some(mut materials) = world.get_resource_mut::<Assets<StandardMaterial>>() {
                    if let Some(mut m) = materials.get_mut(&handle) {
                        match field {
                            ComponentField::BaseColor => {
                                if let FieldValue::Rgba(c) = value {
                                    m.base_color = color_from(c);
                                    return Ok(());
                                }
                            }
                            ComponentField::Metallic => {
                                if let Some(f) = f32_of(value) {
                                    m.metallic = f.clamp(0.0, 1.0);
                                    return Ok(());
                                }
                            }
                            ComponentField::Roughness => {
                                if let Some(f) = f32_of(value) {
                                    m.perceptual_roughness = f.clamp(0.0, 1.0);
                                    return Ok(());
                                }
                            }
                            ComponentField::Emissive => {
                                if let FieldValue::Rgba(c) = value {
                                    m.emissive = bevy::color::LinearRgba::rgb(c[0], c[1], c[2]);
                                    return Ok(());
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        (ComponentKind::Camera, ComponentField::FovDeg) => {
            if let Some(mut proj) = world.get_mut::<Projection>(entity) {
                if let Some(f) = f32_of(value) {
                    if let Projection::Perspective(ref mut p) = *proj {
                        p.fov = f.to_radians().clamp(1.0, 150.0);
                        return Ok(());
                    }
                }
            }
        }
        (ComponentKind::Camera, ComponentField::CameraHdr) => {
            if let FieldValue::Bool(b) = value {
                if b {
                    world.entity_mut(entity).insert(bevy::camera::Hdr);
                } else {
                    let _ = world.entity_mut(entity).remove::<bevy::camera::Hdr>();
                }
                return Ok(());
            }
        }
        (ComponentKind::DirectionalLight, _) => {
            if let Some(mut l) = world.get_mut::<DirectionalLight>(entity) {
                match field {
                    ComponentField::SunColor => {
                        if let FieldValue::Rgba(c) = value {
                            l.color = color_from(c);
                            return Ok(());
                        }
                    }
                    ComponentField::SunIlluminance => {
                        if let Some(f) = f32_of(value) {
                            l.illuminance = f.max(0.0);
                            return Ok(());
                        }
                    }
                    ComponentField::SunShadows => {
                        if let FieldValue::Bool(b) = value {
                            l.shadow_maps_enabled = b;
                            return Ok(());
                        }
                    }
                    _ => {}
                }
            }
        }
        (ComponentKind::PointLight, _) => {
            if let Some(mut l) = world.get_mut::<PointLight>(entity) {
                match field {
                    ComponentField::LightColor => {
                        if let FieldValue::Rgba(c) = value {
                            l.color = color_from(c);
                            return Ok(());
                        }
                    }
                    ComponentField::LightIntensity => {
                        if let Some(f) = f32_of(value) {
                            l.intensity = f.max(0.0);
                            return Ok(());
                        }
                    }
                    ComponentField::LightRadius => {
                        if let Some(f) = f32_of(value) {
                            l.radius = f.max(0.0);
                            return Ok(());
                        }
                    }
                    ComponentField::LightShadows => {
                        if let FieldValue::Bool(b) = value {
                            l.shadow_maps_enabled = b;
                            return Ok(());
                        }
                    }
                    _ => {}
                }
            }
        }
        (ComponentKind::SpotLight, _) => {
            if let Some(mut l) = world.get_mut::<SpotLight>(entity) {
                match field {
                    ComponentField::LightColor => {
                        if let FieldValue::Rgba(c) = value {
                            l.color = color_from(c);
                            return Ok(());
                        }
                    }
                    ComponentField::LightIntensity => {
                        if let Some(f) = f32_of(value) {
                            l.intensity = f.max(0.0);
                            return Ok(());
                        }
                    }
                    ComponentField::LightRange => {
                        if let Some(f) = f32_of(value) {
                            l.range = f.max(0.1);
                            return Ok(());
                        }
                    }
                    ComponentField::LightOuterAngleDeg => {
                        if let Some(f) = f32_of(value) {
                            l.outer_angle = f.to_radians().clamp(0.01, std::f32::consts::FRAC_PI_2);
                            return Ok(());
                        }
                    }
                    ComponentField::LightShadows => {
                        if let FieldValue::Bool(b) = value {
                            l.shadow_maps_enabled = b;
                            return Ok(());
                        }
                    }
                    _ => {}
                }
            }
        }
        (ComponentKind::Rotator, ComponentField::RotatorSpeed) => {
            if let Some(mut r) = world.get_mut::<scripts::Rotator>(entity) {
                if let Some(v) = value_v3(value) {
                    r.speed = v;
                    return Ok(());
                }
            }
        }
        (ComponentKind::Orbiter, _) => {
            if let Some(mut o) = world.get_mut::<scripts::Orbiter>(entity) {
                match field {
                    ComponentField::OrbiterCenter => {
                        if let Some(v) = value_v3(value) {
                            o.center = v;
                            return Ok(());
                        }
                    }
                    ComponentField::OrbiterRadius => {
                        if let Some(f) = f32_of(value) {
                            o.radius = f;
                            return Ok(());
                        }
                    }
                    ComponentField::OrbiterSpeed => {
                        if let Some(f) = f32_of(value) {
                            o.speed = f;
                            return Ok(());
                        }
                    }
                    _ => {}
                }
            }
        }
        (ComponentKind::LinearMover, _) => {
            if let Some(mut m) = world.get_mut::<scripts::LinearMover>(entity) {
                match field {
                    ComponentField::MoverVelocity => {
                        if let Some(v) = value_v3(value) {
                            m.velocity = v;
                            return Ok(());
                        }
                    }
                    ComponentField::MoverPingPong => {
                        if let FieldValue::Bool(b) = value {
                            m.ping_pong = b;
                            return Ok(());
                        }
                    }
                    _ => {}
                }
            }
        }
        (ComponentKind::PingPongMover, _) => {
            if let Some(mut m) = world.get_mut::<scripts::PingPongMover>(entity) {
                match field {
                    ComponentField::PingPongOffset => {
                        if let Some(v) = value_v3(value) {
                            m.offset = v;
                            return Ok(());
                        }
                    }
                    ComponentField::PingPongPeriod => {
                        if let Some(f) = f32_of(value) {
                            m.period = f.max(0.01);
                            return Ok(());
                        }
                    }
                    _ => {}
                }
            }
        }
        (ComponentKind::Player, _) => {
            if let Some(mut p) = world.get_mut::<scripts::Player>(entity) {
                match field {
                    ComponentField::PlayerSpeed => {
                        if let Some(f) = f32_of(value) {
                            p.speed = f;
                            return Ok(());
                        }
                    }
                    ComponentField::PlayerJumpForce => {
                        if let Some(f) = f32_of(value) {
                            p.jump_force = f;
                            return Ok(());
                        }
                    }
                    ComponentField::PlayerSprintMultiplier => {
                        if let Some(f) = f32_of(value) {
                            p.sprint_multiplier = f.max(0.1);
                            return Ok(());
                        }
                    }
                    _ => {}
                }
            }
        }
        (ComponentKind::CharacterController, _) => {
            if let Some(mut c) = world.get_mut::<scripts::CharacterController>(entity) {
                match field {
                    ComponentField::CcHeight => {
                        if let Some(f) = f32_of(value) {
                            c.height = f;
                            return Ok(());
                        }
                    }
                    ComponentField::CcRadius => {
                        if let Some(f) = f32_of(value) {
                            c.radius = f;
                            return Ok(());
                        }
                    }
                    ComponentField::CcStepOffset => {
                        if let Some(f) = f32_of(value) {
                            c.step_offset = f;
                            return Ok(());
                        }
                    }
                    ComponentField::CcSlopeLimit => {
                        if let Some(f) = f32_of(value) {
                            c.slope_limit = f.clamp(0.0, 89.0);
                            return Ok(());
                        }
                    }
                    _ => {}
                }
            }
        }
        (ComponentKind::Health, _) => {
            if let Some(mut h) = world.get_mut::<scripts::Health>(entity) {
                match field {
                    ComponentField::HealthCurrent => {
                        if let Some(f) = f32_of(value) {
                            h.current = f;
                            return Ok(());
                        }
                    }
                    ComponentField::HealthMax => {
                        if let Some(f) = f32_of(value) {
                            h.max = f;
                            return Ok(());
                        }
                    }
                    _ => {}
                }
            }
        }
        (ComponentKind::Inventory, ComponentField::InventorySlots) => {
            if let Some(mut i) = world.get_mut::<scripts::Inventory>(entity) {
                if let FieldValue::U32(u) = value {
                    i.slots = u;
                    return Ok(());
                }
            }
        }
        _ => {}
    }
    Err(format!("unsupported edit: {component:?}.{field:?}"))
}

/// Add a script/builtin component with its default value.
pub fn apply_add_component(world: &mut World, entity: Entity, kind: ComponentKind) -> Result<(), String> {
    use scripts::*;
    macro_rules! insert_if_absent {
        ($ty:ty) => {{
            if world.get::<$ty>(entity).is_some() {
                return Err(format!("entity already has {}", kind.label()));
            }
            let mut e = world.entity_mut(entity);
            e.insert(<$ty>::default());
        }};
    }
    match kind {
        ComponentKind::Rotator => insert_if_absent!(Rotator),
        ComponentKind::Orbiter => insert_if_absent!(Orbiter),
        ComponentKind::LinearMover => insert_if_absent!(LinearMover),
        ComponentKind::PingPongMover => insert_if_absent!(PingPongMover),
        ComponentKind::Player => insert_if_absent!(Player),
        ComponentKind::CharacterController => insert_if_absent!(CharacterController),
        ComponentKind::Health => insert_if_absent!(Health),
        ComponentKind::Inventory => insert_if_absent!(Inventory),
        _ => return Err(format!("{} is intrinsic and cannot be added", kind.label())),
    }
    Ok(())
}

pub fn apply_remove_component(world: &mut World, entity: Entity, kind: ComponentKind) -> Result<(), String> {
    use scripts::*;
    macro_rules! remove_if_present {
        ($ty:ty) => {{
            if world.get::<$ty>(entity).is_none() {
                false
            } else {
                world.entity_mut(entity).remove::<$ty>();
                true
            }
        }};
    }
    let removed = match kind {
        ComponentKind::Rotator => remove_if_present!(Rotator),
        ComponentKind::Orbiter => remove_if_present!(Orbiter),
        ComponentKind::LinearMover => remove_if_present!(LinearMover),
        ComponentKind::PingPongMover => remove_if_present!(PingPongMover),
        ComponentKind::Player => remove_if_present!(Player),
        ComponentKind::CharacterController => remove_if_present!(CharacterController),
        ComponentKind::Health => remove_if_present!(Health),
        ComponentKind::Inventory => remove_if_present!(Inventory),
        _ => return Err(format!("{} is intrinsic and cannot be removed", kind.label())),
    };
    if removed {
        Ok(())
    } else {
        Err(format!("entity does not have {}", kind.label()))
    }
}

/// Push the selected entity's inspector payload when flagged (exclusive so
/// `extract_components` can borrow the whole world).
pub fn push_selected_components(world: &mut World) {
    use crate::state::{IpcChannels, RuntimeFlags, Selection};
    let flags_ok = {
        let flags = world.resource::<RuntimeFlags>();
        let _selection = world.resource::<Selection>();
        flags.components_dirty
    };
    if !flags_ok {
        return;
    }
    let entity = {
        let selection = world.resource::<Selection>();
        selection.0
    };
    {
        let mut flags = world.resource_mut::<RuntimeFlags>();
        flags.components_dirty = false;
    }
    let Some(entity) = entity else { return };
    let channels = world.resource::<IpcChannels>().evt_tx.clone();
    if world.get_entity(entity).is_err() {
        let _ = channels.send(forge_ipc::RuntimeToEditor::EntityComponents {
            entity: entity.to_bits(),
            name: String::new(),
            components: Vec::new(),
        });
        return;
    }
    let (name, components) = extract_components(world, entity);
    let _ = channels.send(forge_ipc::RuntimeToEditor::EntityComponents {
        entity: entity.to_bits(),
        name,
        components,
    });
}

