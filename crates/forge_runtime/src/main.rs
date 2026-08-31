//! # bevyforge-runtime — the engine process of BevyForge
//!
//! Owns the Bevy ECS [`World`], renders the scene offscreen (no window, no
//! winit) and streams RGB frames to the editor over the IPC protocol. All
//! editor commands are executed by an exclusive system so structural ECS
//! changes are safe.
//!
//! Usage:
//! ```text
//! bevyforge-runtime --project <dir> [--port 48470] [--screenshot out.png --width 1920 --height 1080]
//! ```

mod animation;
mod camera_info;
mod capture;
mod commands;
mod env;
mod factory;
mod gizmos;
mod logs;
mod picking;
mod scene_io;
mod stats;
mod state;

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::asset::AssetPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy::window::{ExitCondition, WindowPlugin};

use forge_ipc::DEFAULT_PORT;

use crate::state::{IpcChannels, PlaySnapshot, RuntimeFlags, ViewportRig, ViewportSize};

/// Bevy engine version this runtime is built against (kept in sync with Cargo.toml).
pub const ENGINE_VERSION: &str = "0.19.1";

fn main() -> anyhow::Result<()> {
    let mut project_dir = std::env::current_dir()?;
    let mut port = DEFAULT_PORT;
    let mut screenshot: Option<String> = None;
    let mut init_demo = false;
    let mut width = 1920u32;
    let mut height = 1080u32;
    // wgpu backend selection: "vulkan" | "gl" | "dx12" | "metal" | "all".
    // Empty means "let wgpu decide" (works on every supported platform).
    let mut backend = std::env::var("FORGE_BACKEND").unwrap_or_default();

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--project" => {
                project_dir = args.next().map(std::path::PathBuf::from).unwrap_or(project_dir)
            }
            "--port" => port = args.next().and_then(|v| v.parse().ok()).unwrap_or(port),
            "--screenshot" => screenshot = args.next(),
            "--init-demo" => init_demo = true,
            "--width" => width = args.next().and_then(|v| v.parse().ok()).unwrap_or(width),
            "--height" => height = args.next().and_then(|v| v.parse().ok()).unwrap_or(height),
            "--backend" => backend = args.next().unwrap_or_default(),
            "--version" => {
                println!("bevyforge-runtime {} (bevy {ENGINE_VERSION})", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }

    std::fs::create_dir_all(&project_dir)?;
    std::env::set_current_dir(&project_dir)?;
    // Assets resolve relative to the project root from here on.
    std::fs::create_dir_all("assets/scenes")?;

    // Bind the IPC listener and announce the port on stdout (the spawner
    // parses the FORGE_PORT line). A busy port fails FAST with exit code 3 so
    // the editor's retry loop can pick up as soon as a leftover engine
    // instance releases it (the engine lingers only briefly after its editor
    // disconnects).
    let listener = match forge_ipc::listen(port) {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            println!("FORGE_PORT_BUSY={port}");
            eprintln!(
                "Error: IPC port {port} is already in use — a leftover engine \
                 instance may still be shutting down. The editor retries automatically."
            );
            use std::io::Write as _;
            let _ = std::io::stdout().flush();
            std::process::exit(3);
        }
        Err(e) => return Err(e.into()),
    };
    let bound_port = listener.local_addr()?.port();
    println!("FORGE_PORT={bound_port}");
    println!("bevyforge-runtime serving project {}", project_dir.display());
    use std::io::Write as _;
    std::io::stdout().flush()?;

    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
    let (evt_tx, evt_rx) = crossbeam_channel::unbounded();
    forge_ipc::transport::spawn_relay(listener, cmd_tx, evt_rx);

    let screenshot_mode = screenshot.clone();
    let init_demo_flag = init_demo;
    let _shot_size = state::ViewportSize { width, height };
    let backends = parse_backends(&backend);

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .set(AssetPlugin::default())
            .set(bevy::render::RenderPlugin {
                render_creation: bevy::render::settings::RenderCreation::Automatic(
                    Box::new(bevy::render::settings::WgpuSettings {
                        backends,
                        power_preference:
                            bevy::render::settings::PowerPreference::HighPerformance,
                        ..default()
                    }),
                ),
                ..default()
            })
            .set(LogPlugin {
                filter: "wgpu=error,naga=warn,bevy_asset=warn".to_string(),
                level: bevy::log::Level::INFO,
                custom_layer: logs::ipc_log_layer,
                ..default()
            }),
    )
    // No bevy_winit in this build: drive the loop ourselves.
    .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / 60.0)))
    .add_plugins(forge_scripts::ForgeScriptsPlugin)
    .add_plugins(capture::CapturePlugin)
    .insert_resource(IpcChannels { cmd_rx, evt_tx })
    .insert_resource(RuntimeFlags::default())
    .insert_resource(ViewportRig::default())
    .insert_resource(state::Selection(None))
    .insert_resource(state::CameraSource::Editor)
    .insert_resource(state::EnvironmentSettingsHolder(forge_ipc::EnvironmentSettings::default()))
    .insert_resource(animation::AnimationStore::default())
    .insert_resource(animation::AnimPlayback::default())
    .insert_resource(state::PlayState { playing: false })
    .insert_resource(PlaySnapshot::default())
    .insert_resource(state::PendingScreenshot::default())
    .insert_resource(ViewportSize { width: 1280, height: 720 })
    .insert_resource(camera_info::LastCameraInfo::default())
    .insert_resource(state::GestureSnapshot::default())
    .insert_resource(state::StartupShot {
        path: screenshot_mode.clone(),
        width,
        height,
    })
    .insert_resource(state::InitDemo(init_demo_flag))
    .insert_resource(capture::ViewportTargetHandle(bevy::asset::Handle::default()))
    .add_systems(Startup, (setup_editor_rig, build_offscreen_target, startup_scene).chain())
    .add_systems(
        Update,
        (
            commands::execute_commands,
            env::apply_environment,
            animation::advance_time,
            animation::apply_animation,
            // Gameplay systems only tick in Play Mode.
            forge_scripts::rotate_system.run_if(state::playing),
            forge_scripts::orbit_system.run_if(state::playing),
            forge_scripts::linear_move_system.run_if(state::playing),
            forge_scripts::ping_pong_system.run_if(state::playing),
            forge_scripts::player_patrol_system.run_if(state::playing),
            gizmos::draw_editor_overlays,
            capture::manage_screenshot_jobs,
            capture::route_captured_frames,
            stats::collect_stats,
        )
            .chain(),
    )
    .add_systems(
        PostUpdate,
        (
            camera_info::push_camera_info,
            scene_io::push_hierarchy,
            factory::push_selected_components,
            animation::push_anim_state,
            env::push_env_state,
            scene_io::push_scene_info,
            capture::push_frames,
            logs::push_logs,
        )
            .chain(),
    );

    if screenshot_mode.is_none() {
        info!("bevyforge-runtime entering main loop");
    }
    app.run();
    Ok(())
}

/// Spawns the free-orbit editor camera (owns the offscreen render target).
fn setup_editor_rig(mut commands: Commands) {
    commands.spawn((
        Name::new("__EditorCamera"),
        state::EditorOnlyTag,
        Camera3d::default(),
        Transform::from_xyz(-6.0, 5.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

/// Builds the viewport render target and attaches the capture pipeline to the
/// editor camera. Runs once at startup (resizing later reuses the same path).
fn build_offscreen_target(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    editor_camera: Query<Entity, (With<Camera3d>, With<state::EditorOnlyTag>)>,
    render_device: Option<Res<bevy::render::renderer::RenderDevice>>,
    viewport: Res<ViewportSize>,
) {
    let Some(cam) = editor_camera.iter().next() else {
        warn!("no editor camera at startup");
        return;
    };
    let Some(device) = render_device else {
        warn!("render device not ready at startup (headless smoke test?)");
        return;
    };
    let (copier_entity, target_handle) = capture::attach_render_target(
        &mut commands,
        &mut images,
        &device,
        cam,
        viewport.width,
        viewport.height,
        0,
    );
    commands.insert_resource(capture::ViewportCopier(copier_entity));
    commands.insert_resource(capture::ViewportTargetHandle(target_handle));
}

/// Loads `assets/scenes/main.scn.ron` when present, otherwise creates the
/// default rig (Main Camera + Directional Light).
fn startup_scene(world: &mut World) {
    let main_scene = std::path::Path::new("assets/scenes/main.scn.ron");
    if main_scene.exists() {
        match scene_io::load_scene_from_path(world, main_scene) {
            Ok(count) => {
                info!("loaded main scene ({count} entities)");
                let mut flags = world.resource_mut::<RuntimeFlags>();
                flags.scene_dirty = true;
                flags.hierarchy_dirty = true;
                flags.env_dirty = true;
            }
            Err(e) => {
                warn!("failed to load main scene: {e:#}; creating default rig");
                scene_io::spawn_default_rig(world);
            }
        }
    } else {
        scene_io::spawn_default_rig(world);
        let mut flags = world.resource_mut::<RuntimeFlags>();
        flags.hierarchy_dirty = true;
        flags.env_dirty = true;
        flags.scene_dirty = true;
    }

    // Demo project initialisation (bevyforge-runtime --init-demo):
    // author the design-mirroring starter scene, save it and exit.
    if world.resource::<crate::state::InitDemo>().0 {
        scene_io::spawn_demo_scene(world);
        match scene_io::save_scene_to_path(world, std::path::Path::new("assets/scenes/main.scn.ron")) {
            Ok(n) => info!("demo scene written ({} entities)", n),
            Err(e) => warn!("demo save failed: {e:#}"),
        }
        world.write_message(AppExit::Success);
    }
}

/// Parse a `--backend` / `FORGE_BACKEND` value into a wgpu `Backends` mask.
///
/// Accepted values (case-insensitive): `vulkan`, `gl`/`opengl`, `dx12`,
/// `metal`, `all`. The empty string / `all` / `auto` allows every backend,
/// letting wgpu apply its own default adapter selection — Vulkan on Linux,
/// DX12 on Windows, Metal on macOS. This is what makes the engine run on any
/// GPU: machines without Vulkan can force `--backend gl`, and VMs / old
/// laptops fall back to software rasterizers (lavapipe / llvmpipe / WARP)
/// automatically. Never returns `None`: an empty mask would disable the
/// render app entirely.
fn parse_backends(spec: &str) -> Option<bevy::render::settings::Backends> {
    use bevy::render::settings::Backends;
    match spec.trim().to_lowercase().as_str() {
        // wgpu applies its own default adapter selection when all backends are
        // allowed (Vulkan on Linux, DX12 on Windows, Metal on macOS).
        "" | "all" | "auto" => Some(Backends::all()),
        "vulkan" | "vk" => Some(Backends::VULKAN),
        "gl" | "opengl" | "gles" => Some(Backends::GL),
        "dx12" | "d3d12" | "directx" => Some(Backends::DX12),
        "metal" | "mtl" => Some(Backends::METAL),
        other => {
            eprintln!("bevyforge-runtime: unknown backend '{other}', using default selection");
            Some(Backends::all())
        }
    }
}
