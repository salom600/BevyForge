//! BevyForge's typed scene format (`*.scn.ron`).
//!
//! Bevy 0.19 moved to code-first BSN scenes; an editor, however, needs a
//! serialisable project format. `ForgeScene` mirrors the IPC data model with
//! plain serde types: primitives are recreated deterministically on load, so
//! there are no asset-handle serialisation pitfalls, and files round-trip
//! byte-for-byte stable for identical scenes.

use bevy::color::Color;
use bevy::math::{EulerRot, Quat, Vec3};
use bevy::light::{DirectionalLight, PointLight, SpotLight};
use bevy::pbr::StandardMaterial;
use bevy::prelude::*;

use forge_ipc::{
    AnimEntityTracks, AnimTrack, ComponentKind, EntityId, EnvironmentSettings, MeshPrimitive,
};
use ron::ser::PrettyConfig;

use crate::factory;
use crate::state::{
    EditorLocked, EditorOnlyTag, EnvironmentSettingsHolder, IpcChannels, PlaySnapshot, RuntimeFlags,
    Selection, UserScene,
};
use forge_scripts as scripts;

// ---------------------------------------------------------------------------
// File model
// ---------------------------------------------------------------------------

/// Root document of a `*.scn.ron` file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ForgeScene {
    pub engine: String,
    pub environment: EnvironmentSettings,
    #[serde(default)]
    pub animation: SceneAnimation,
    pub entities: Vec<SceneEntity>,
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
    pub tracks: Vec<(AnimTrack, Vec<(f32, [f32; 3])>)>,
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
    Mesh(MeshPrimitive),
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

// ---------------------------------------------------------------------------
// Capture (world -> ForgeScene)
// ---------------------------------------------------------------------------

fn v3(v: Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

fn euler_deg(q: &Quat) -> [f32; 3] {
    let (x, y, z) = q.to_euler(EulerRot::XYZ);
    [x.to_degrees(), y.to_degrees(), z.to_degrees()]
}

/// Serialise all user entities into a scene (used for save + play snapshots).
pub fn capture_scene(world: &mut World) -> ForgeScene {
    use bevy::camera::visibility::Visibility;
    use bevy::ecs::hierarchy::{ChildOf, Children};

    // Depth-first collect preserving parent order.
    let mut ordered: Vec<Entity> = Vec::new();
    {
        let mut q = world.query::<(Entity, Option<&ChildOf>)>();
        let mut roots: Vec<Entity> = q
            .iter(world)
            .filter_map(|(e, p)| if p.is_none() { Some(e) } else { None })
            .filter(|e| world.get::<UserScene>(*e).is_some())
            .collect();
        roots.sort_by_key(|e| factory::entity_name(world, *e));
        let mut stack: Vec<Entity> = roots;
        while let Some(e) = stack.pop() {
            ordered.push(e);
            let mut kids: Vec<Entity> = world
                .get::<Children>(e)
                .map(|c| c.to_vec())
                .unwrap_or_default()
                .into_iter()
                .filter(|c| world.get::<UserScene>(*c).is_some())
                .collect();
            kids.sort_by_key(|c| factory::entity_name(world, *c));
            kids.reverse();
            stack.extend(kids);
        }
    }

    let mut entities = Vec::new();
    for entity in ordered {
        let name = factory::entity_name(world, entity);
        let parent = world
            .get::<ChildOf>(entity)
            .map(|p| factory::entity_name(world, p.0));

        let kind = if let Some(src) = world.get::<factory::SourceMesh>(entity) {
            SceneEntityKind::Mesh(src.0)
        } else if world.get::<Camera3d>(entity).is_some() {
            SceneEntityKind::Camera
        } else if world.get::<DirectionalLight>(entity).is_some() {
            SceneEntityKind::DirectionalLight
        } else if world.get::<PointLight>(entity).is_some() {
            SceneEntityKind::PointLight
        } else if world.get::<SpotLight>(entity).is_some() {
            SceneEntityKind::SpotLight
        } else {
            SceneEntityKind::Empty
        };

        let (t, r, s) = world
            .get::<Transform>(entity)
            .map(|tr| (v3(tr.translation), euler_deg(&tr.rotation), v3(tr.scale)))
            .unwrap_or(([0.0; 3], [0.0; 3], [1.0; 3]));
        let visible = !matches!(world.get::<Visibility>(entity), Some(Visibility::Hidden));
        let locked = world.get::<EditorLocked>(entity).is_some();

        let material = world
            .get::<bevy::pbr::MeshMaterial3d<StandardMaterial>>(entity)
            .and_then(|h| world.get_resource::<Assets<StandardMaterial>>())
            .and_then(|_| None::<SceneMaterial>); // replaced below (borrow rules)

        // Material extraction done separately to satisfy borrows.
        let material = material.or_else(|| {
            let handle = world
                .get::<bevy::pbr::MeshMaterial3d<StandardMaterial>>(entity)
                .map(|h| h.0.clone())?;
            let materials = world.get_resource::<Assets<StandardMaterial>>()?;
            let m = materials.get(&handle)?;
            let c = m.base_color.to_srgba();
            let e = m.emissive;
            Some(SceneMaterial {
                base_color: [c.red, c.green, c.blue, c.alpha],
                metallic: m.metallic,
                roughness: m.perceptual_roughness,
                emissive: [e.red, e.green, e.blue, e.alpha],
            })
        });

        let camera = world.get::<Camera3d>(entity).and_then(|_| {
            let fov = match world.get::<bevy::camera::Projection>(entity) {
                Some(bevy::camera::Projection::Perspective(p)) => p.fov.to_degrees(),
                _ => 45.0,
            };
            Some(SceneCamera { fov_deg: fov })
        });

        let light = if let Some(l) = world.get::<DirectionalLight>(entity) {
            let c = l.color.to_srgba();
            Some(SceneLight::Directional {
                color: [c.red, c.green, c.blue, c.alpha],
                illuminance: l.illuminance,
                shadows: l.shadow_maps_enabled,
            })
        } else if let Some(l) = world.get::<PointLight>(entity) {
            let c = l.color.to_srgba();
            Some(SceneLight::Point {
                color: [c.red, c.green, c.blue, c.alpha],
                intensity: l.intensity,
                radius: l.radius,
                shadows: l.shadow_maps_enabled,
            })
        } else if let Some(l) = world.get::<SpotLight>(entity) {
            let c = l.color.to_srgba();
            Some(SceneLight::Spot {
                color: [c.red, c.green, c.blue, c.alpha],
                intensity: l.intensity,
                range: l.range,
                outer_angle_deg: l.outer_angle.to_degrees(),
                shadows: l.shadow_maps_enabled,
            })
        } else {
            None
        };

        let mut script_list: Vec<SceneScript> = Vec::new();
        if let Some(r) = world.get::<scripts::Rotator>(entity) {
            script_list.push(SceneScript::Rotator { speed: v3(r.speed) });
        }
        if let Some(o) = world.get::<scripts::Orbiter>(entity) {
            script_list.push(SceneScript::Orbiter { center: v3(o.center), radius: o.radius, speed: o.speed });
        }
        if let Some(m) = world.get::<scripts::LinearMover>(entity) {
            script_list.push(SceneScript::LinearMover { velocity: v3(m.velocity), ping_pong: m.ping_pong });
        }
        if let Some(m) = world.get::<scripts::PingPongMover>(entity) {
            script_list.push(SceneScript::PingPongMover { offset: v3(m.offset), period: m.period });
        }
        if let Some(p) = world.get::<scripts::Player>(entity) {
            script_list.push(SceneScript::Player {
                speed: p.speed,
                jump_force: p.jump_force,
                sprint_multiplier: p.sprint_multiplier,
            });
        }
        if let Some(c) = world.get::<scripts::CharacterController>(entity) {
            script_list.push(SceneScript::CharacterController {
                height: c.height,
                radius: c.radius,
                step_offset: c.step_offset,
                slope_limit: c.slope_limit,
            });
        }
        if let Some(h) = world.get::<scripts::Health>(entity) {
            script_list.push(SceneScript::Health { current: h.current, max: h.max });
        }
        if let Some(i) = world.get::<scripts::Inventory>(entity) {
            script_list.push(SceneScript::Inventory { slots: i.slots });
        }

        entities.push(SceneEntity {
            name,
            parent,
            kind,
            transform: (t, r, s),
            visible,
            locked,
            material,
            camera,
            light,
            scripts: script_list,
        });
    }

    let anim = world.resource::<crate::animation::AnimationStore>();
    let playback = world.resource::<crate::animation::AnimPlayback>();
    let mut entries = Vec::new();
    for (name, tracks) in anim.by_name.iter() {
        entries.push(SceneAnimEntry {
            name: name.clone(),
            tracks: tracks
                .tracks
                .iter()
                .map(|(t, keys)| (*t, keys.iter().map(|(time, v)| (*time, *v)).collect()))
                .collect(),
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    ForgeScene {
        engine: crate::ENGINE_VERSION.to_string(),
        environment: world.resource::<EnvironmentSettingsHolder>().0.clone(),
        animation: SceneAnimation { duration: playback.duration, entries },
        entities,
    }
}

// ---------------------------------------------------------------------------
// Apply (ForgeScene -> world)
// ---------------------------------------------------------------------------

fn script_from(scene: &SceneScript) -> scripts::AnyScript {
    use scripts::*;
    match scene {
        SceneScript::Rotator { speed } => AnyScript::Rotator(Rotator {
            speed: Vec3::new(speed[0], speed[1], speed[2]),
            ..Default::default()
        }),
        SceneScript::Orbiter { center, radius, speed } => AnyScript::Orbiter(Orbiter {
            center: Vec3::new(center[0], center[1], center[2]),
            radius: *radius,
            speed: *speed,
            ..Default::default()
        }),
        SceneScript::LinearMover { velocity, ping_pong } => AnyScript::LinearMover(LinearMover {
            velocity: Vec3::new(velocity[0], velocity[1], velocity[2]),
            ping_pong: *ping_pong,
            ..Default::default()
        }),
        SceneScript::PingPongMover { offset, period } => AnyScript::PingPongMover(PingPongMover {
            offset: Vec3::new(offset[0], offset[1], offset[2]),
            period: *period,
            ..Default::default()
        }),
        SceneScript::Player { speed, jump_force, sprint_multiplier } => AnyScript::Player(Player {
            speed: *speed,
            jump_force: *jump_force,
            sprint_multiplier: *sprint_multiplier,
        }),
        SceneScript::CharacterController { height, radius, step_offset, slope_limit } => {
            AnyScript::CharacterController(CharacterController {
                height: *height,
                radius: *radius,
                step_offset: *step_offset,
                slope_limit: *slope_limit,
            })
        }
        SceneScript::Health { current, max } => AnyScript::Health(Health { current: *current, max: *max }),
        SceneScript::Inventory { slots } => AnyScript::Inventory(Inventory { slots: *slots }),
    }
}

/// Despawn every user entity (anything tagged `UserScene`).
pub fn clear_user_entities(world: &mut World) {
    let mut q = world.query::<(Entity, Option<&UserScene>)>();
    let mut doomed: Vec<Entity> = q
        .iter(world)
        .filter_map(|(e, user)| if user.is_some() { Some(e) } else { None })
        .collect();
    // Despawn deepest-first (children before parents) and skip entities that
    // a parent's despawn already removed.
    doomed.reverse();
    for e in doomed {
        if world.get_entity(e).is_ok() {
            let _ = world.despawn(e);
        }
    }
}

/// Spawn the default rig into an empty world.
pub fn spawn_default_rig(world: &mut World) {
    factory::spawn_default_rig_contents(world);
}

/// Load a scene file from disk, replacing current contents.
pub fn load_scene_from_path(world: &mut World, path: &std::path::Path) -> anyhow::Result<usize> {
    let text = std::fs::read_to_string(path)?;
    let scene: ForgeScene = ron::from_str(&text)?;
    let count = apply_scene(world, &scene);
    Ok(count)
}

/// Replace world contents with `scene`; returns entity count.
pub fn apply_scene(world: &mut World, scene: &ForgeScene) -> usize {
    clear_user_entities(world);
    world.resource_mut::<EnvironmentSettingsHolder>().0 = scene.environment.clone();

    let mut name_to_entity: std::collections::HashMap<String, Entity> = std::collections::HashMap::new();
    let mut spawned = 0usize;

    for se in &scene.entities {
        let kind = match se.kind {
            SceneEntityKind::Empty => forge_ipc::EntityKind::Empty,
            SceneEntityKind::Mesh(p) => forge_ipc::EntityKind::Mesh(p),
            SceneEntityKind::Camera => forge_ipc::EntityKind::Camera,
            SceneEntityKind::DirectionalLight => forge_ipc::EntityKind::DirectionalLight,
            SceneEntityKind::PointLight => forge_ipc::EntityKind::PointLight,
            SceneEntityKind::SpotLight => forge_ipc::EntityKind::SpotLight,
        };
        let parent = se
            .parent
            .as_ref()
            .and_then(|p| name_to_entity.get(p).copied());
        let entity = factory::spawn_entity(world, &se.name, parent, kind);

        // Transform (euler-deg round trip).
        let (t, r, s) = se.transform;
        let mut e = world.entity_mut(entity);
        e.insert(Transform {
            translation: Vec3::new(t[0], t[1], t[2]),
            rotation: Quat::from_euler(
                EulerRot::XYZ,
                r[0].to_radians(),
                r[1].to_radians(),
                r[2].to_radians(),
            ),
            scale: Vec3::new(s[0], s[1], s[2]),
        });
        drop(e);

        if !se.visible {
            world.entity_mut(entity).insert(bevy::camera::visibility::Visibility::Hidden);
        }
        if se.locked {
            world.entity_mut(entity).insert(EditorLocked);
        }

        if let Some(mat) = &se.material {
            let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
            let new_mat = materials.add(StandardMaterial {
                base_color: Color::srgba(mat.base_color[0], mat.base_color[1], mat.base_color[2], mat.base_color[3]),
                metallic: mat.metallic,
                perceptual_roughness: mat.roughness,
                emissive: bevy::color::LinearRgba::rgb(mat.emissive[0], mat.emissive[1], mat.emissive[2]),
                ..default()
            });
            drop(materials);
            if let Some(mut h) = world.get_mut::<bevy::pbr::MeshMaterial3d<StandardMaterial>>(entity) {
                h.0 = new_mat;
            }
        }

        if let Some(cam) = &se.camera {
            if let Some(mut proj) = world.get_mut::<bevy::camera::Projection>(entity) {
                if let bevy::camera::Projection::Perspective(ref mut p) = *proj {
                    p.fov = cam.fov_deg.to_radians();
                }
            }
        }

        if let Some(light) = &se.light {
            match light {
                SceneLight::Directional { color, illuminance, shadows } => {
                    if let Some(mut l) = world.get_mut::<DirectionalLight>(entity) {
                        l.color = Color::srgba(color[0], color[1], color[2], color[3]);
                        l.illuminance = *illuminance;
                        l.shadow_maps_enabled = *shadows;
                    }
                }
                SceneLight::Point { color, intensity, radius, shadows } => {
                    if let Some(mut l) = world.get_mut::<PointLight>(entity) {
                        l.color = Color::srgba(color[0], color[1], color[2], color[3]);
                        l.intensity = *intensity;
                        l.radius = *radius;
                        l.shadow_maps_enabled = *shadows;
                    }
                }
                SceneLight::Spot { color, intensity, range, outer_angle_deg, shadows } => {
                    if let Some(mut l) = world.get_mut::<SpotLight>(entity) {
                        l.color = Color::srgba(color[0], color[1], color[2], color[3]);
                        l.intensity = *intensity;
                        l.range = *range;
                        l.outer_angle = outer_angle_deg.to_radians();
                        l.shadow_maps_enabled = *shadows;
                    }
                }
            }
        }

        for script in &se.scripts {
            match script_from(script) {
                scripts::AnyScript::Rotator(v) => {
                    world.entity_mut(entity).insert(v);
                }
                scripts::AnyScript::Orbiter(v) => {
                    world.entity_mut(entity).insert(v);
                }
                scripts::AnyScript::LinearMover(v) => {
                    world.entity_mut(entity).insert(v);
                }
                scripts::AnyScript::PingPongMover(v) => {
                    world.entity_mut(entity).insert(v);
                }
                scripts::AnyScript::Player(v) => {
                    world.entity_mut(entity).insert(v);
                }
                scripts::AnyScript::CharacterController(v) => {
                    world.entity_mut(entity).insert(v);
                }
                scripts::AnyScript::Health(v) => {
                    world.entity_mut(entity).insert(v);
                }
                scripts::AnyScript::Inventory(v) => {
                    world.entity_mut(entity).insert(v);
                }
            }
        }

        name_to_entity.insert(se.name.clone(), entity);
        spawned += 1;
    }

    // Restore animation tracks by name.
    {
        let mut store = world.resource_mut::<crate::animation::AnimationStore>();
        store.by_name.clear();
        for entry in &scene.animation.entries {
            store.by_name.insert(
                entry.name.clone(),
                crate::animation::EntityTracks {
                    tracks: entry
                        .tracks
                        .iter()
                        .map(|(t, keys)| (*t, keys.clone()))
                        .collect(),
                },
            );
        }
    }
    let mut playback = world.resource_mut::<crate::animation::AnimPlayback>();
    playback.duration = if scene.animation.duration > 0.0 { scene.animation.duration } else { 30.0 };
    playback.time = 0.0;
    playback.playing = false;

    spawned
}

/// Spawn the BevyForge starter scene mirroring the design blueprint:
/// ground, platforms, props, a rigged Player prefab and an animated demo cube.
pub fn spawn_demo_scene(world: &mut World) {
    use forge_ipc::{EntityKind, MeshPrimitive};

    // NOTE: the default rig (Main Camera + Directional Light) is spawned by
    // startup_scene before this runs — do NOT spawn a second one here.

    let ground = factory::spawn_entity(world, "Ground", None, EntityKind::Mesh(MeshPrimitive::Plane));
    world.entity_mut(ground).insert(Transform::from_scale(Vec3::splat(12.0)));
    if let Some(handle) = world.get::<bevy::pbr::MeshMaterial3d<StandardMaterial>>(ground).map(|m| m.0.clone()) {
        if let Some(mut mats) = world.get_resource_mut::<Assets<StandardMaterial>>() {
            if let Some(mut m) = mats.get_mut(&handle) {
                m.base_color = Color::srgb(0.16, 0.19, 0.24);
                m.perceptual_roughness = 0.9;
            }
        }
    }

    let platform = |world: &mut World, name: &str, x: f32, z: f32, y: f32| {
        let e = factory::spawn_entity(world, name, None, EntityKind::Mesh(MeshPrimitive::Cube));
        world.entity_mut(e).insert(Transform::from_xyz(x, y, z));
        if let Some(handle) = world.get::<bevy::pbr::MeshMaterial3d<StandardMaterial>>(e).map(|m| m.0.clone()) {
            if let Some(mut mats) = world.get_resource_mut::<Assets<StandardMaterial>>() {
                if let Some(mut m) = mats.get_mut(&handle) {
                    m.base_color = Color::srgb(0.22, 0.28, 0.38);
                    m.metallic = 0.25;
                }
            }
        }
        e
    };
    let plat1 = platform(world, "Platform.001", -4.0, -2.0, 0.5);
    let plat2 = platform(world, "Platform.002", 0.0, -2.0, 0.5);
    let _plat3 = platform(world, "Platform.003", 4.0, -2.0, 0.5);

    // Prop cluster under Platforms group.
    let group = {
        let name = factory::unique_name(world, "Props");
        let mut e = world.spawn((Name::new(name), crate::state::UserScene));
        e.insert(Transform::IDENTITY);
        e.id()
    };
    for (existing, parent) in [(plat1, Some(group)), (plat2, Some(group))] {
        world.entity_mut(existing).insert(bevy::ecs::hierarchy::ChildOf(parent.unwrap()));
    }
    let crate_e = factory::spawn_entity(world, "Crate", Some(group), EntityKind::Mesh(MeshPrimitive::Cube));
    world.entity_mut(crate_e).insert(Transform::from_xyz(1.4, 0.5, 1.2));
    let barrel = factory::spawn_entity(world, "Barrel", Some(group), EntityKind::Mesh(MeshPrimitive::Cylinder));
    world.entity_mut(barrel).insert(Transform::from_xyz(2.2, 1.0, 0.4));
    let terminal = factory::spawn_entity(world, "Terminal", Some(group), EntityKind::Mesh(MeshPrimitive::Cone));
    world.entity_mut(terminal).insert(Transform::from_xyz(0.6, 1.0, 1.6));

    // Animated showcase cube.
    let spinner = factory::spawn_entity(world, "Spinning Cube", None, EntityKind::Mesh(MeshPrimitive::Cube));
    world.entity_mut(spinner).insert((
        Transform::from_xyz(-2.5, 1.5, 2.5),
        scripts::Rotator { speed: Vec3::new(0.4, 1.6, 0.0) },
    ));

    // Player prefab at origin-ish.
    let player = factory::spawn_entity(world, "Player", None, EntityKind::PlayerPrefab);
    world.entity_mut(player).insert(Transform::from_xyz(0.0, 1.0, 3.5));

    // A warm point light accent.
    let light = factory::spawn_entity(world, "Point Light", None, EntityKind::PointLight);
    world.entity_mut(light).insert(Transform::from_xyz(2.5, 3.0, 2.5));
}

/// Save current world to `*.scn.ron` on disk.
pub fn save_scene_to_path(world: &mut World, path: &std::path::Path) -> anyhow::Result<usize> {
    let scene = capture_scene(world);
    let count = scene.entities.len();
    let text = ron::ser::to_string_pretty(&scene, PrettyConfig::default())?;
    std::fs::write(path, text)?;
    Ok(count)
}

// ---------------------------------------------------------------------------
// Editor pushes
// ---------------------------------------------------------------------------

/// Push hierarchy when dirty (or every 15th frame as a safety net).
pub fn push_hierarchy(world: &mut World) {
    use crate::animation::AnimPlayback;
    use crate::state::{CameraSource, PlayState};
    let (dirty, frame_hint) = {
        let flags = world.resource::<RuntimeFlags>();
        (flags.hierarchy_dirty, false)
    };
    static FRAME_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let tick = FRAME_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if !dirty && tick % 15 != 0 {
        return;
    }
    let selected = world.resource::<Selection>().0;
    let nodes = factory::build_hierarchy(world, selected);
    let flags_dirty = {
        let mut flags = world.resource_mut::<RuntimeFlags>();
        flags.hierarchy_dirty = false;
        flags.scene_dirty
    };
    let _ = flags_dirty;
    let channels = world.resource::<IpcChannels>().evt_tx.clone();
    let _ = channels.send(forge_ipc::RuntimeToEditor::Hierarchy { nodes });
    let _ = (frame_hint, 0u32);
    let _ = std::any::TypeId::of::<(AnimPlayback, CameraSource, PlayState)>();
}

/// Push SceneInfo (path + dirty).
pub fn push_scene_info(world: &mut World) {
    use crate::state::RuntimeFlags;
    let flags = world.resource::<RuntimeFlags>();
    if !flags.scene_dirty {
        return;
    }
    let path = world
        .get_resource::<crate::state::ScenePath>()
        .and_then(|p| p.0.clone());
    let mut flags = world.resource_mut::<RuntimeFlags>();
    flags.scene_dirty = false;
    let channels = world.resource::<IpcChannels>().evt_tx.clone();
    let _ = channels.send(forge_ipc::RuntimeToEditor::SceneInfo {
        path,
        dirty: false,
    });
}

// ---------------------------------------------------------------------------
// Play mode
// ---------------------------------------------------------------------------

/// Enter/exit play mode with full snapshot rollback.
pub fn set_play_mode(world: &mut World, playing: bool) {
    let channels = world.resource::<IpcChannels>().evt_tx.clone();
    if playing {
        let snapshot = capture_scene(world);
        world.resource_mut::<PlaySnapshot>().0 = Some(snapshot);
        world.resource_mut::<crate::state::PlayState>().playing = true;
        let _ = channels.send(forge_ipc::RuntimeToEditor::Notice {
            level: forge_ipc::LogLevel::Info,
            message: "Entered Play Mode (systems running, state snapshot taken)".into(),
        });
    } else {
        world.resource_mut::<crate::state::PlayState>().playing = false;
        if let Some(snapshot) = world.resource::<PlaySnapshot>().0.clone() {
            apply_scene(world, &snapshot);
        }
        world.resource_mut::<PlaySnapshot>().0 = None;
        // Selection may reference despawned entities.
        world.resource_mut::<Selection>().0 = None;
        let mut flags = world.resource_mut::<RuntimeFlags>();
        flags.hierarchy_dirty = true;
        flags.components_dirty = true;
        flags.anim_dirty = true;
        drop(flags);
        let _ = channels.send(forge_ipc::RuntimeToEditor::Notice {
            level: forge_ipc::LogLevel::Info,
            message: "Stopped Play Mode (state restored from snapshot)".into(),
        });
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_roundtrips_through_ron() {
        let scene = ForgeScene {
            engine: "0.19.1".into(),
            environment: EnvironmentSettings::default(),
            animation: SceneAnimation::default(),
            entities: vec![SceneEntity {
                name: "Cube".into(),
                parent: None,
                kind: SceneEntityKind::Mesh(MeshPrimitive::Cube),
                transform: ([0.0, 0.5, 0.0], [0.0, 90.0, 0.0], [1.0; 3]),
                visible: true,
                locked: false,
                material: Some(SceneMaterial {
                    base_color: [0.7, 0.7, 0.75, 1.0],
                    metallic: 0.1,
                    roughness: 0.6,
                    emissive: [0.0; 4],
                }),
                camera: None,
                light: None,
                scripts: vec![SceneScript::Rotator { speed: [0.0, 1.0, 0.0] }],
            }],
        };
        let text = ron::ser::to_string_pretty(&scene, PrettyConfig::default()).unwrap();
        let back: ForgeScene = ron::from_str(&text).unwrap();
        assert_eq!(back.entities.len(), 1);
        assert_eq!(back.entities[0].name, "Cube");
        assert!(matches!(back.entities[0].kind, SceneEntityKind::Mesh(MeshPrimitive::Cube)));
    }
}
