//! Applies the editor's environment/lighting settings to engine resources and
//! camera components, and pushes the current settings back to the panel.

use bevy::camera::Exposure;
use bevy::color::Color;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::math::Vec3;
use bevy::light::DirectionalLight;
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;

use forge_ipc::{EnvironmentSettings, TonemappingKind};

use crate::state::{EditorOnlyTag, EnvironmentSettingsHolder, IpcChannels, RuntimeFlags};

fn tonemapping(k: TonemappingKind) -> Tonemapping {
    match k {
        TonemappingKind::None => Tonemapping::None,
        TonemappingKind::Reinhard => Tonemapping::Reinhard,
        TonemappingKind::AcesFitted => Tonemapping::AcesFitted,
        TonemappingKind::AgX => Tonemapping::AgX,
        TonemappingKind::TonyMcMapface => Tonemapping::TonyMcMapface,
        TonemappingKind::BlenderFilmic => Tonemapping::BlenderFilmic,
    }
}

fn rgba(c: [f32; 4]) -> Color {
    Color::srgba(c[0], c[1], c[2], c[3])
}

/// Apply environment settings to the world (runs every frame; cheap).
pub fn apply_environment(world: &mut World) {
    let settings = world.resource::<EnvironmentSettingsHolder>().0.clone();

    // Global ambient light.
    if let Some(mut ambient) = world.get_resource_mut::<bevy::light::GlobalAmbientLight>() {
        ambient.color = rgba(settings.ambient_color);
        ambient.brightness = settings.ambient_brightness;
    }

    // Clear color.
    world.insert_resource(ClearColor(rgba(settings.clear_color)));

    // Camera-side settings target whichever camera currently renders the
    // viewport (editor rig or a scene camera).
    let target_camera = viewport_camera_entity(world);
    if let Some(cam) = target_camera {
        if let Some(mut tm) = world.get_mut::<Tonemapping>(cam) {
            *tm = tonemapping(settings.tonemapping);
        } else {
            world.entity_mut(cam).insert(tonemapping(settings.tonemapping));
        }
        if let Some(mut ex) = world.get_mut::<Exposure>(cam) {
            ex.ev100 = settings.exposure_ev100;
        } else {
            world.entity_mut(cam).insert(Exposure { ev100: settings.exposure_ev100 });
        }
        if settings.fog_enabled {
            let fog = DistanceFog {
                color: rgba(settings.fog_color),
                falloff: FogFalloff::Linear {
                    start: settings.fog_start,
                    end: settings.fog_end,
                },
                ..default()
            };
            if world.get::<DistanceFog>(cam).is_some() {
                world.entity_mut(cam).insert(fog);
            } else {
                world.entity_mut(cam).insert(fog);
            }
        } else {
            let _ = world.entity_mut(cam).remove::<DistanceFog>();
        }
    }

    // Sun (first directional light): elevation/azimuth orbit + illuminance.
    apply_sun(world, &settings);
}

fn viewport_camera_entity(world: &mut World) -> Option<Entity> {
    use crate::state::CameraSource;
    match *world.resource::<CameraSource>() {
        CameraSource::Editor => world
            .query_filtered::<Entity, (With<Camera3d>, With<EditorOnlyTag>)>()
            .iter(world)
            .next(),
        CameraSource::Scene(e) => Some(e),
    }
}

fn apply_sun(world: &mut World, settings: &EnvironmentSettings) {
    let sun = {
        let mut q = world.query_filtered::<Entity, With<DirectionalLight>>();
        q.iter(world).next()
    };
    let Some(e) = sun else { return };
    if let Some(mut light) = world.get_mut::<DirectionalLight>(e) {
        light.illuminance = settings.sun_illuminance;
        light.shadow_maps_enabled = settings.sun_shadows;
    }
    // Sun sits on a sphere of radius 20 around the origin.
    let elev = settings.sun_elevation_deg.to_radians();
    let azim = settings.sun_azimuth_deg.to_radians();
    let dir = Vec3::new(elev.cos() * azim.cos(), elev.sin(), elev.cos() * azim.sin());
    let pos = dir * 20.0;
    if let Some(mut transform) = world.get_mut::<Transform>(e) {
        *transform = Transform::from_translation(pos).looking_at(Vec3::ZERO, Vec3::Y);
    }
}

/// Push env settings to the editor when flagged (or every 60 frames).
pub fn push_env_state(world: &mut World) {
    let dirty = world.resource::<RuntimeFlags>().env_dirty;
    static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let tick = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if !dirty && tick % 60 != 0 {
        return;
    }
    world.resource_mut::<RuntimeFlags>().env_dirty = false;
    let settings = world.resource::<EnvironmentSettingsHolder>().0.clone();
    let channels = world.resource::<IpcChannels>().evt_tx.clone();
    let _ = channels.send(forge_ipc::RuntimeToEditor::EnvState(settings));
}
