//! The exclusive command executor: drains editor commands from the IPC
//! channel and performs the ECS mutations. Runs first in `Update`.

use bevy::prelude::*;

use forge_ipc::{EditorToRuntime, LogLevel, RuntimeToEditor};
use crate::state::EnvironmentSettingsHolder;

use crate::animation::{AnimPlayback, AnimationStore};
use crate::capture;

use crate::factory;
use crate::picking;
use crate::scene_io;
use crate::state::{
    CameraSource, EditorLocked, EditorOnlyTag, IpcChannels, PendingScreenshot, RuntimeFlags, ScenePath, Selection, ViewportRig, ViewportSize,
};

/// Drain and execute all pending commands.
pub fn execute_commands(world: &mut World) {
    let mut batch: Vec<EditorToRuntime> = Vec::new();
    {
        let channels = world.resource::<IpcChannels>();
        while let Ok(cmd) = channels.cmd_rx.try_recv() {
            batch.push(cmd);
            if batch.len() >= 256 {
                break;
            }
        }
    }
    if batch.is_empty() {
        return;
    }

    let channels = world.resource::<IpcChannels>().evt_tx.clone();
    let mut flags = world.resource::<RuntimeFlags>().clone();

    for cmd in batch {
        execute_one(world, cmd, &channels, &mut flags);
    }

    {
        let value: RuntimeFlags = flags.clone();
        let mut r = world.resource_mut::<RuntimeFlags>();
        *r = value;
    }
}

fn notify(channels: &crossbeam_channel::Sender<RuntimeToEditor>, level: LogLevel, msg: String) {
    let _ = channels.send(RuntimeToEditor::Notice { level, message: msg });
}

fn entity_of(id: forge_ipc::EntityId) -> Option<Entity> {
    if id == forge_ipc::NO_ENTITY {
        return None;
    }
    Some(Entity::from_bits(id))
}

fn execute_one(
    world: &mut World,
    cmd: EditorToRuntime,
    channels: &crossbeam_channel::Sender<RuntimeToEditor>,
    flags: &mut RuntimeFlags,
) {
    match cmd {
        EditorToRuntime::Hello => {
            let _ = channels.send(RuntimeToEditor::Welcome {
                protocol: forge_ipc::PROTOCOL_VERSION,
                forge_version: env!("CARGO_PKG_VERSION").to_string(),
                bevy_version: crate::ENGINE_VERSION.to_string(),
                pid: std::process::id(),
            });
            flags.all_dirty();
        }
        EditorToRuntime::Ping(v) => {
            let _ = channels.send(RuntimeToEditor::Pong(v));
        }
        EditorToRuntime::SetViewportSize { width, height } => {
            if width == 0 || height == 0 || (width, height) == {
                let vp = world.resource::<ViewportSize>();
                (vp.width, vp.height)
            } {
                return;
            }
            // Rebuild the viewport target at the new size (sequenced borrows).
            let device = match world.get_resource::<bevy::render::renderer::RenderDevice>() {
                Some(d) => d.clone(),
                None => return,
            };
            let cam = match world
                .query_filtered::<Entity, (With<Camera3d>, With<EditorOnlyTag>)>()
                .iter(world)
                .next()
            {
                Some(c) => c,
                None => return,
            };
            let size = bevy::render::render_resource::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            };
            let bytes_per_row =
                bevy::render::renderer::RenderDevice::align_copy_bytes_per_row(width as usize * 4);
            let buffer = device.create_buffer(&bevy::render::render_resource::BufferDescriptor {
                label: Some("forge_capture_buffer"),
                size: (bytes_per_row * height as usize) as u64,
                usage: bevy::render::render_resource::BufferUsages::MAP_READ
                    | bevy::render::render_resource::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let handle = {
                let mut target = Image::new_target_texture(
                    width,
                    height,
                    bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
                    None,
                );
                target.texture_descriptor.usage |=
                    bevy::render::render_resource::TextureUsages::COPY_SRC;
                let mut images = world.resource_mut::<Assets<Image>>();
                images.add(target)
            };
            let copier_entity = {
                let copier = capture::ImageCopier {
                    buffer,
                    src_image: handle.clone(),
                    tag: 0,
                    size,
                };
                let mut cmds = world.commands();
                cmds.spawn(copier).id()
            };
            world.entity_mut(cam).insert(bevy::camera::RenderTarget::Image(handle.clone().into()));
            // Retire the previous copier.
            if let Some(old) = world.remove_resource::<capture::ViewportCopier>() {
                let _ = world.despawn(old.0);
            }
            world.insert_resource(capture::ViewportCopier(copier_entity));
            world.insert_resource(capture::ViewportTargetHandle(handle));
            world.insert_resource(ViewportSize { width, height });
        }
        EditorToRuntime::Select { entity } => {
            world.resource_mut::<Selection>().0 = entity.and_then(entity_of);
            flags.components_dirty = true;
        }
        EditorToRuntime::SpawnEntity { name, parent, kind } => {
            let parent_entity = parent.and_then(entity_of);
            let entity = factory::spawn_entity(world, &name, parent_entity, kind);
            flags.hierarchy_dirty = true;
            flags.scene_dirty = true;
            flags.components_dirty = true;
            world.resource_mut::<Selection>().0 = Some(entity);
            let bits = entity.to_bits();
            let _ = channels.send(RuntimeToEditor::PickResult { x: -1.0, y: -1.0, entity: Some(bits) });
            notify(channels, LogLevel::Info, format!("Spawned {}", name));
        }
        EditorToRuntime::DeleteEntity { entity } => {
            let Some(entity) = entity_of(entity) else { return };
            if world.get::<EditorOnlyTag>(entity).is_some() {
                notify(channels, LogLevel::Warn, "Cannot delete the editor camera".into());
                return;
            }
            if world.get::<EditorLocked>(entity).is_some() {
                notify(channels, LogLevel::Warn, "Entity is locked".into());
                return;
            }
            let name = factory::entity_name(world, entity);
            let _ = world.despawn(entity);
            if world.resource::<Selection>().0 == Some(entity) {
                world.resource_mut::<Selection>().0 = None;
            }
            world.resource_mut::<crate::animation::AnimationStore>().clear_entity(&name);
            flags.hierarchy_dirty = true;
            flags.scene_dirty = true;
            flags.components_dirty = true;
            flags.anim_dirty = true;
            notify(channels, LogLevel::Info, format!("Deleted {name}"));
        }
        EditorToRuntime::DuplicateEntity { entity } => {
            let Some(src) = entity_of(entity) else { return };
            duplicate_entity(world, src, flags, channels);
        }
        EditorToRuntime::Reparent { entity, new_parent } => {
            let Some(entity) = entity_of(entity) else { return };
            if world.get::<EditorLocked>(entity).is_some() {
                notify(channels, LogLevel::Warn, "Entity is locked".into());
                return;
            }
            let Some(parent) = new_parent.and_then(entity_of) else {
                // Detach: remove ChildOf.
                let _ = world.entity_mut(entity).remove::<bevy::ecs::hierarchy::ChildOf>();
                flags.hierarchy_dirty = true;
                flags.scene_dirty = true;
                return;
            };
            // Guard against cycles: new parent must not be a descendant.
            if is_descendant_of(world, parent, entity) {
                notify(channels, LogLevel::Warn, "Cannot parent to a descendant".into());
                return;
            }
            world.entity_mut(entity).insert(bevy::ecs::hierarchy::ChildOf(parent));
            flags.hierarchy_dirty = true;
            flags.scene_dirty = true;
        }
        EditorToRuntime::RenameEntity { entity, name } => {
            let Some(entity) = entity_of(entity) else { return };
            let old = factory::entity_name(world, entity);
            world.entity_mut(entity).insert(Name::new(name.clone()));
            // Re-key animation tracks.
            let mut store = world.resource_mut::<AnimationStore>();
            if let Some(tracks) = store.by_name.remove(&old) {
                store.by_name.insert(name.clone(), tracks);
            }
            flags.hierarchy_dirty = true;
            flags.scene_dirty = true;
            flags.components_dirty = true;
        }
        EditorToRuntime::SetLocked { entity, locked } => {
            let Some(entity) = entity_of(entity) else { return };
            if locked {
                world.entity_mut(entity).insert(EditorLocked);
            } else {
                let _ = world.entity_mut(entity).remove::<EditorLocked>();
            }
            flags.hierarchy_dirty = true;
        }
        EditorToRuntime::MoveEntity { entity, delta } => {
            let Some(entity) = entity_of(entity) else { return };
            if world.get::<EditorLocked>(entity).is_some() {
                return;
            }
            if let Some(mut t) = world.get_mut::<Transform>(entity) {
                t.translation += Vec3::new(delta[0], delta[1], delta[2]);
                flags.components_dirty = true;
                flags.scene_dirty = true;
            }
        }
        EditorToRuntime::RotateEntityWorld { entity, axis, angle_deg } => {
            let Some(entity) = entity_of(entity) else { return };
            if world.get::<EditorLocked>(entity).is_some() {
                return;
            }
            let axis = Vec3::new(axis[0], axis[1], axis[2]);
            let Some(axis) = axis.try_normalize() else { return };
            if let Some(mut t) = world.get_mut::<Transform>(entity) {
                let q = Quat::from_axis_angle(axis, angle_deg.to_radians());
                t.rotation = (q * t.rotation).normalize();
                flags.components_dirty = true;
                flags.scene_dirty = true;
            }
        }
        EditorToRuntime::ScaleEntityBy { entity, factor } => {
            let Some(entity) = entity_of(entity) else { return };
            if world.get::<EditorLocked>(entity).is_some() {
                return;
            }
            if let Some(mut t) = world.get_mut::<Transform>(entity) {
                let f = Vec3::new(factor[0], factor[1], factor[2]);
                t.scale = (t.scale * f).max(Vec3::splat(0.001));
                flags.components_dirty = true;
                flags.scene_dirty = true;
            }
        }
        EditorToRuntime::BeginGizmoGesture { entity } => {
            let Some(entity) = entity_of(entity) else { return };
            let snapshot = world.get::<Transform>(entity).map(capture_transform);
            world
                .resource_mut::<crate::state::GestureSnapshot>()
                .0 = snapshot.map(|(translation, euler_deg, scale)| forge_ipc::TransformAbs {
                entity: entity.to_bits(),
                translation,
                euler_deg,
                scale,
            });
        }
        EditorToRuntime::EndGizmoGesture { entity, label } => {
            let Some(entity) = entity_of(entity) else { return };
            let pre = world.resource::<crate::state::GestureSnapshot>().0;
            let post = world.get::<Transform>(entity).map(capture_transform).map(
                |(translation, euler_deg, scale)| forge_ipc::TransformAbs {
                    entity: entity.to_bits(),
                    translation,
                    euler_deg,
                    scale,
                },
            );
            world.resource_mut::<crate::state::GestureSnapshot>().0 = None;
            if let (Some(pre), Some(post)) = (pre, post) {
                let _ = channels.send(RuntimeToEditor::GestureDone {
                    entity: entity.to_bits(),
                    label,
                    pre,
                    post,
                });
            }
        }
        EditorToRuntime::SetField { entity, component, field, value } => {
            let Some(entity) = entity_of(entity) else { return };
            match factory::apply_set_field(world, entity, component, field, value) {
                Ok(()) => {
                    flags.components_dirty = true;
                    flags.scene_dirty = true;
                }
                Err(e) => notify(channels, LogLevel::Warn, e),
            }
        }
        EditorToRuntime::AddComponent { entity, component } => {
            let Some(entity) = entity_of(entity) else { return };
            match factory::apply_add_component(world, entity, component) {
                Ok(()) => {
                    flags.components_dirty = true;
                    flags.scene_dirty = true;
                    flags.hierarchy_dirty = true;
                    notify(channels, LogLevel::Info, format!("Added {}", component.label()));
                }
                Err(e) => notify(channels, LogLevel::Warn, e),
            }
        }
        EditorToRuntime::RemoveComponent { entity, component } => {
            let Some(entity) = entity_of(entity) else { return };
            match factory::apply_remove_component(world, entity, component) {
                Ok(()) => {
                    flags.components_dirty = true;
                    flags.scene_dirty = true;
                    flags.hierarchy_dirty = true;
                    notify(channels, LogLevel::Info, format!("Removed {}", component.label()));
                }
                Err(e) => notify(channels, LogLevel::Warn, e),
            }
        }
        EditorToRuntime::NewScene => {
            scene_io::clear_user_entities(world);
            scene_io::spawn_default_rig(world);
            world.resource_mut::<Selection>().0 = None;
            world.resource_mut::<ScenePath>().0 = None;
            world.resource_mut::<AnimationStore>().by_name.clear();
            world.resource_mut::<AnimPlayback>().time = 0.0;
            flags.all_dirty();
            notify(channels, LogLevel::Info, "New scene created".into());
        }
        EditorToRuntime::OpenScene { path } => {
            match scene_io::load_scene_from_path(world, std::path::Path::new(&path)) {
                Ok(count) => {
                    world.resource_mut::<Selection>().0 = None;
                    world.resource_mut::<ScenePath>().0 = Some(path.clone());
                    flags.all_dirty();
                    notify(channels, LogLevel::Info, format!("Opened {path} ({count} entities)"));
                }
                Err(e) => notify(channels, LogLevel::Error, format!("Open failed: {e:#}")),
            }
        }
        EditorToRuntime::SaveScene { path } => {
            match scene_io::save_scene_to_path(world, std::path::Path::new(&path)) {
                Ok(count) => {
                    world.resource_mut::<ScenePath>().0 = Some(path.clone());
                    flags.scene_dirty = true;
                    notify(channels, LogLevel::Info, format!("Saved {path} ({count} entities)"));
                }
                Err(e) => notify(channels, LogLevel::Error, format!("Save failed: {e:#}")),
            }
        }
        EditorToRuntime::SetPlayMode { playing } => {
            scene_io::set_play_mode(world, playing);
            flags.all_dirty();
        }
        EditorToRuntime::Pick { x, y } => {
            let hit = picking::pick_at(world, x, y);
            let bits = hit.map(|e| e.to_bits());
            let _ = channels.send(RuntimeToEditor::PickResult { x, y, entity: bits });
        }
        EditorToRuntime::AddKeyframe { entity, track, time, value } => {
            let Some(entity) = entity_of(entity) else { return };
            let name = factory::entity_name(world, entity);
            world.resource_mut::<AnimationStore>().track_mut(&name, track).push((time, value));
            // Keep sorted by time.
            let name_ref = name.clone();
            let mut store = world.resource_mut::<AnimationStore>();
            if let Some(entry) = store.by_name.get_mut(&name_ref) {
                if let Some((_, keys)) = entry.tracks.iter_mut().find(|(t, _)| *t == track) {
                    keys.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                }
            }
            flags.anim_dirty = true;
            flags.scene_dirty = true;
        }
        EditorToRuntime::RemoveKeyframe { entity, track, index } => {
            let Some(entity) = entity_of(entity) else { return };
            let name = factory::entity_name(world, entity);
            let mut store = world.resource_mut::<AnimationStore>();
            if let Some(entry) = store.by_name.get_mut(&name) {
                if let Some((_, keys)) = entry.tracks.iter_mut().find(|(t, _)| *t == track) {
                    if index < keys.len() {
                        keys.remove(index);
                    }
                }
            }
            flags.anim_dirty = true;
            flags.scene_dirty = true;
        }
        EditorToRuntime::MoveKeyframe { entity, track, index, new_time } => {
            let Some(entity) = entity_of(entity) else { return };
            let name = factory::entity_name(world, entity);
            let mut store = world.resource_mut::<AnimationStore>();
            if let Some(entry) = store.by_name.get_mut(&name) {
                if let Some((_, keys)) = entry.tracks.iter_mut().find(|(t, _)| *t == track) {
                    if index < keys.len() {
                        keys[index].0 = new_time;
                        keys.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                    }
                }
            }
            flags.anim_dirty = true;
            flags.scene_dirty = true;
        }
        EditorToRuntime::ClearTracks { entity } => {
            let Some(entity) = entity_of(entity) else { return };
            let name = factory::entity_name(world, entity);
            world.resource_mut::<AnimationStore>().clear_entity(&name);
            flags.anim_dirty = true;
            flags.scene_dirty = true;
        }
        EditorToRuntime::SetAnimTime(t) => {
            world.resource_mut::<AnimPlayback>().time = t.clamp(0.0, world.resource::<AnimPlayback>().duration);
            flags.anim_dirty = true;
        }
        EditorToRuntime::SetAnimPlaying(playing) => {
            world.resource_mut::<AnimPlayback>().playing = playing;
            flags.anim_dirty = true;
        }
        EditorToRuntime::SetAnimDuration(d) => {
            world.resource_mut::<AnimPlayback>().duration = d.max(0.1);
            flags.anim_dirty = true;
        }
        EditorToRuntime::SetAnimLooped(looped) => {
            world.resource_mut::<AnimPlayback>().looped = looped;
            flags.anim_dirty = true;
        }
        EditorToRuntime::SetEnvironment(settings) => {
            world.resource_mut::<EnvironmentSettingsHolder>().0 = settings;
            flags.env_dirty = true;
            flags.scene_dirty = true;
        }
        EditorToRuntime::SetEditorCamera { target, distance, yaw_deg, pitch_deg } => {
            let mut rig = world.resource_mut::<ViewportRig>();
            rig.target = Vec3::new(target[0], target[1], target[2]);
            rig.distance = distance.max(0.5);
            rig.yaw_deg = yaw_deg;
            rig.pitch_deg = pitch_deg.clamp(-89.0, 89.0);
            let eye = rig.eye();
            let look_target = rig.target;
            drop(rig);
            apply_editor_camera_transform(world, eye, look_target);
        }
        EditorToRuntime::SetViewportCamera { entity } => {
            match entity.and_then(entity_of) {
                Some(e) => {
                    if world.get::<Camera3d>(e).is_some() {
                        switch_viewport_camera(world, Some(e));
                        *world.resource_mut::<CameraSource>() = CameraSource::Scene(e);
                        notify(channels, LogLevel::Info, "Game view: rendering through scene camera".into());
                    } else {
                        notify(channels, LogLevel::Warn, "Entity is not a camera".into());
                    }
                }
                None => {
                    switch_viewport_camera(world, None);
                    *world.resource_mut::<CameraSource>() = CameraSource::Editor;
                }
            }
            flags.env_dirty = true;
        }
        EditorToRuntime::RequestFullState => {
            flags.all_dirty();
        }
        EditorToRuntime::RequestScreenshot { path } => {
            world.resource_mut::<PendingScreenshot>().request = Some((path, 1920, 1080));
        }
        EditorToRuntime::Shutdown => {
            info!("shutdown requested by editor");
            let _ = channels.send(RuntimeToEditor::Goodbye {
                reason: "editor requested shutdown".into(),
            });
            world.write_message(AppExit::Success);
        }
    }
}

/// Snapshot a transform with glam, euler in degrees (inspector convention).
fn capture_transform(t: &Transform) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let (x, y, z) = t.rotation.to_euler(EulerRot::XYZ);
    (
        t.translation.to_array(),
        [x.to_degrees(), y.to_degrees(), z.to_degrees()],
        t.scale.to_array(),
    )
}

/// Apply the orbit rig transform to the editor camera.
fn apply_editor_camera_transform(world: &mut World, eye: Vec3, target: Vec3) {
    let mut q = world.query_filtered::<Entity, (With<Camera3d>, With<EditorOnlyTag>)>();
    let Some(cam) = q.iter(world).next() else { return };
    world.entity_mut(cam).insert(Transform::from_translation(eye).looking_at(target, Vec3::Y));
}

/// Enable a scene camera for viewport rendering / restore the editor rig.
fn switch_viewport_camera(world: &mut World, scene_camera: Option<Entity>) {
    use bevy::camera::visibility::Visibility;

    // Deactivate every scene camera and strip special targets.
    let mut q = world.query::<(Entity, &Camera3d, Option<&EditorOnlyTag>)>();
    let scene_cameras: Vec<Entity> = q
        .iter(world)
        .filter_map(|(e, _, tag)| if tag.is_none() { Some(e) } else { None })
        .collect();
    for e in scene_cameras {
        if Some(e) != scene_camera {
            if let Some(mut cam) = world.get_mut::<Camera>(e) {
                cam.is_active = false;
            }
        }
    }

    match scene_camera {
        Some(e) => {
            // Point the scene camera at the viewport image target.
            let handle = world
                .get_resource::<capture::ViewportTargetHandle>()
                .map(|h| h.0.clone());
            if let Some(handle) = handle {
                world.entity_mut(e).insert(bevy::camera::RenderTarget::Image(handle.into()));
            }
            if let Some(mut cam) = world.get_mut::<Camera>(e) {
                cam.is_active = true;
            }
            let _ = world.entity_mut(e).remove::<Visibility>();
        }
        None => {
            // Reactivate the editor rig (it keeps its own target component).
            let mut q = world.query_filtered::<Entity, (With<Camera3d>, With<EditorOnlyTag>)>();
            let Some(editor_cam) = q.iter(world).next() else { return };
            if let Some(mut cam) = world.get_mut::<Camera>(editor_cam) {
                cam.is_active = true;
            }
        }
    }
}

/// True when `candidate` is a descendant of (or equal to) `ancestor`.
fn is_descendant_of(world: &mut World, candidate: Entity, ancestor: Entity) -> bool {
    if candidate == ancestor {
        return true;
    }
    let mut stack = vec![candidate];
    let mut seen = std::collections::HashSet::new();
    while let Some(e) = stack.pop() {
        if !seen.insert(e) {
            continue;
        }
        if e == ancestor {
            return true;
        }
        if let Some(children) = world.get::<bevy::ecs::hierarchy::Children>(e) {
            stack.extend(children.to_vec());
        }
    }
    false
}

/// Duplicate an entity with its full subtree.
fn duplicate_entity(
    world: &mut World,
    src: Entity,
    flags: &mut RuntimeFlags,
    channels: &crossbeam_channel::Sender<RuntimeToEditor>,
) {
    use bevy::light::{DirectionalLight, PointLight, SpotLight};
use bevy::pbr::StandardMaterial;

    // Snapshot the subtree (BFS), then re-spawn.
    let mut order: Vec<Entity> = Vec::new();
    let mut stack = vec![src];
    while let Some(e) = stack.pop() {
        order.push(e);
        if let Some(children) = world.get::<bevy::ecs::hierarchy::Children>(e) {
            stack.extend(children.to_vec());
        }
    }

    let mut map: std::collections::HashMap<Entity, Entity> = std::collections::HashMap::new();
    for e in &order {
        let base = factory::entity_name(world, *e);
        let name = if *e == src {
            factory::unique_name(world, &base)
        } else {
            base
        };
        let parent = world
            .get::<bevy::ecs::hierarchy::ChildOf>(*e)
            .and_then(|p| map.get(&p.0).copied());

        let kind = if let Some(m) = world.get::<factory::SourceMesh>(*e) {
            Some(forge_ipc::EntityKind::Mesh(m.0))
        } else if world.get::<Camera3d>(*e).is_some() {
            Some(forge_ipc::EntityKind::Camera)
        } else if world.get::<DirectionalLight>(*e).is_some() {
            Some(forge_ipc::EntityKind::DirectionalLight)
        } else if world.get::<PointLight>(*e).is_some() {
            Some(forge_ipc::EntityKind::PointLight)
        } else if world.get::<SpotLight>(*e).is_some() {
            Some(forge_ipc::EntityKind::SpotLight)
        } else {
            Some(forge_ipc::EntityKind::Empty)
        };

        let new_entity = factory::spawn_entity(world, &name, parent, kind.unwrap());
        map.insert(*e, new_entity);

        // Copy transform.
        let t = world.get::<Transform>(*e).copied();
        if let Some(t) = t {
            world.entity_mut(new_entity).insert(t);
        }
        // Copy material (share handle — materials are assets).
        let mat_handle = world
            .get::<bevy::pbr::MeshMaterial3d<StandardMaterial>>(*e)
            .map(|m| m.0.clone());
        if let Some(h) = mat_handle {
            world.entity_mut(new_entity).insert(bevy::pbr::MeshMaterial3d(h));
        }
        // Copy scripts.
        macro_rules! copy_comp {
            ($ty:ty) => {{
                let cloned = world.get::<$ty>(*e).cloned();
                if let Some(c) = cloned {
                    world.entity_mut(new_entity).insert(c);
                }
            }};
        }
        use forge_scripts as s;
        copy_comp!(s::Rotator);
        copy_comp!(s::Orbiter);
        copy_comp!(s::LinearMover);
        copy_comp!(s::PingPongMover);
        copy_comp!(s::Player);
        copy_comp!(s::CharacterController);
        copy_comp!(s::Health);
        copy_comp!(s::Inventory);
        // Camera settings.
        let proj = world.get::<bevy::camera::Projection>(*e).cloned();
        if let Some(p) = proj {
            world.entity_mut(new_entity).insert(p);
        }
        // Light parameters.
        let sun = world.get::<DirectionalLight>(*e).cloned();
        if let Some(l) = sun {
            world.entity_mut(new_entity).insert(l);
        }
        let point = world.get::<PointLight>(*e).cloned();
        if let Some(l) = point {
            world.entity_mut(new_entity).insert(l);
        }
        let spot = world.get::<SpotLight>(*e).cloned();
        if let Some(l) = spot {
            world.entity_mut(new_entity).insert(l);
        }
    }

    flags.hierarchy_dirty = true;
    flags.scene_dirty = true;
    flags.components_dirty = true;
    if let Some(new_root) = map.get(&src) {
        world.resource_mut::<Selection>().0 = Some(*new_root);
        let bits = new_root.to_bits();
        let _ = channels.send(RuntimeToEditor::PickResult { x: -1.0, y: -1.0, entity: Some(bits) });
    }
    notify(channels, LogLevel::Info, "Entity duplicated".into());
}
