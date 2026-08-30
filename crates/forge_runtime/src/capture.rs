//! Offscreen render-target capture and streaming.
//!
//! Pipeline (mirrors `bevy/examples/app/headless_renderer.rs`):
//! 1. the viewport camera renders into an `Rgba8UnormSrgb` `Image` render target
//! 2. a render-graph system copies that GPU texture into a mapped read-back buffer
//! 3. a `Render`-scheduled system maps the buffer and ships bytes over a channel
//! 4. the main world drains the channel, strips alpha padding and pushes
//!    `RuntimeToEditor::Frame` messages (viewport) or saves PNGs (screenshots)

use bevy::prelude::*;
use bevy::camera::RenderTarget as CameraRenderTarget;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    Buffer, BufferDescriptor, BufferUsages, CommandEncoderDescriptor, Extent3d, MapMode, PollType,
    TexelCopyBufferInfo, TexelCopyBufferLayout, TextureFormat, TextureUsages,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue};
use bevy::render::{Extract, Render, RenderApp, RenderSystems};

use crate::state::{IpcChannels, PendingScreenshot, StartupShot};

/// Render tag distinguishing viewport frames (0) from screenshot jobs (>0).
pub type CaptureTag = u32;

/// Bytes captured by the render world, delivered to the main world.
pub struct CapturedFrame {
    pub tag: CaptureTag,
    pub width: u32,
    pub height: u32,
    pub padded_bytes_per_row: usize,
    pub rgba: Vec<u8>,
}

/// Channel resource living in the render world.
#[derive(Resource)]
pub struct RenderWorldSender(crossbeam_channel::Sender<CapturedFrame>);

/// Channel resource living in the main world.
#[derive(Resource, Clone)]
pub struct MainWorldReceiver(crossbeam_channel::Receiver<CapturedFrame>);

/// GPU→CPU copy job attached to an entity; extracted into the render world.
#[derive(Component)]
pub struct ImageCopier {
    pub buffer: Buffer,
    pub src_image: Handle<Image>,
    pub tag: CaptureTag,
    pub size: Extent3d,
}

impl Clone for ImageCopier {
    fn clone(&self) -> Self {
        Self {
            buffer: self.buffer.clone(),
            src_image: self.src_image.clone(),
            tag: self.tag,
            size: self.size,
        }
    }
}

/// Aggregated copiers in the render world.
#[derive(Resource, Default)]
pub struct ImageCopiers(pub Vec<ImageCopier>);

/// Entity owning the viewport copier (tracked for resize rebuilds).
#[derive(Resource)]
pub struct ViewportCopier(pub Entity);

/// Current viewport render-target image handle (restored after screenshots).
#[derive(Resource, Clone)]
pub struct ViewportTargetHandle(pub Handle<Image>);

/// A running one-shot screenshot job.
#[derive(Resource, Default)]
pub struct ScreenshotJob {
    pub camera: Option<Entity>,
    pub copier: Option<Entity>,
    pub path: String,
    pub preroll_remaining: u32,
    pub done: bool,
}

/// Latest fully-captured viewport frame (RGB8, tightly packed rows).
#[derive(Resource, Default)]
pub struct LastViewportFrame(pub Option<(u32, u32, Vec<u8>)>);

pub struct CapturePlugin;

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        let (s, r) = crossbeam_channel::unbounded();
        app.insert_resource(MainWorldReceiver(r))
            .init_resource::<LastViewportFrame>()
            .init_resource::<ScreenshotJob>();

        let render_app = app.sub_app_mut(RenderApp);
        render_app
            .insert_resource(RenderWorldSender(s))
            .init_resource::<ImageCopiers>()
            .add_systems(bevy::render::ExtractSchedule, image_copy_extract)
            .add_systems(
                Render,
                receive_image_from_buffer.after(RenderSystems::Render),
            )
            .add_systems(bevy::render::renderer::RenderGraph, image_copy_driver);
    }
}

/// Builds an `Rgba8UnormSrgb` target image + mapped read-back copier and swaps
/// `camera` onto the new target. Returns (copier entity, target image handle).
pub fn attach_render_target(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    render_device: &RenderDevice,
    camera: Entity,
    width: u32,
    height: u32,
    tag: CaptureTag,
) -> (Entity, Handle<Image>) {
    let size = Extent3d { width, height, depth_or_array_layers: 1 };

    let mut target_image =
        Image::new_target_texture(size.width, size.height, TextureFormat::Rgba8UnormSrgb, None);
    target_image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let target_handle = images.add(target_image);

    let bytes_per_row = RenderDevice::align_copy_bytes_per_row(width as usize * 4);
    let buffer = render_device.create_buffer(&BufferDescriptor {
        label: Some("forge_capture_buffer"),
        size: (bytes_per_row * height as usize) as u64,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let copier = ImageCopier {
        buffer,
        src_image: target_handle.clone(),
        tag,
        size,
    };
    let copier_entity = commands.spawn(copier).id();
    commands
        .entity(camera)
        .try_insert(CameraRenderTarget::Image(target_handle.clone().into()));
    (copier_entity, target_handle)
}

fn image_copy_extract(mut commands: Commands, copiers: Extract<Query<&ImageCopier>>) {
    let list = copiers.iter().cloned().collect::<Vec<_>>();
    if !list.is_empty() {
        commands.insert_resource(ImageCopiers(list));
    }
}

fn image_copy_driver(
    render_context: RenderContext,
    image_copiers: Res<ImageCopiers>,
    render_queue: Res<RenderQueue>,
    gpu_images: Res<RenderAssets<bevy::render::texture::GpuImage>>,
) {
    for copier in image_copiers.0.iter() {
        let Some(src) = gpu_images.get(&copier.src_image) else { continue };

        let mut encoder = render_context
            .render_device()
            .create_command_encoder(&CommandEncoderDescriptor::default());

        let padded_bytes_per_row =
            RenderDevice::align_copy_bytes_per_row(copier.size.width as usize * 4);

        encoder.copy_texture_to_buffer(
            src.texture.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &copier.buffer,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row as u32),
                    rows_per_image: None,
                },
            },
            copier.size,
        );
        render_queue.submit(std::iter::once(encoder.finish()));
    }
}

fn receive_image_from_buffer(
    image_copiers: Res<ImageCopiers>,
    render_device: Res<RenderDevice>,
    sender: Res<RenderWorldSender>,
) {
    for copier in image_copiers.0.iter() {
        let buffer_slice = copier.buffer.slice(..);
        let (tx, rx) = crossbeam_channel::bounded(1);
        buffer_slice.map_async(MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        if render_device.poll(PollType::wait_indefinitely()).is_err() {
            error!("capture: device poll failed");
            continue;
        }
        if rx.recv().is_err() {
            continue;
        }
        let bytes = buffer_slice.get_mapped_range().to_vec();
        let _ = sender.0.send(CapturedFrame {
            tag: copier.tag,
            width: copier.size.width,
            height: copier.size.height,
            padded_bytes_per_row: RenderDevice::align_copy_bytes_per_row(
                copier.size.width as usize * 4,
            ),
            rgba: bytes,
        });
        copier.buffer.unmap();
    }
}

// ---------------------------------------------------------------------------
// Main-world side
// ---------------------------------------------------------------------------

/// Strips alpha + row padding, stores viewport frames, saves screenshots.
pub fn route_captured_frames(
    receiver: Res<MainWorldReceiver>,
    mut last_frame: ResMut<LastViewportFrame>,
    mut job: ResMut<ScreenshotJob>,
    mut commands: Commands,
    channels: Res<IpcChannels>,
    mut pending: ResMut<PendingScreenshot>,
    viewport_target: Res<ViewportTargetHandle>,
) {
    while let Ok(frame) = receiver.0.try_recv() {
        if frame.tag == 0 {
            // Viewport frame → tightly packed RGB8 for the editor.
            let mut rgb = Vec::with_capacity((frame.width * frame.height * 3) as usize);
            let row_stride = frame.padded_bytes_per_row;
            for y in 0..frame.height as usize {
                let row = &frame.rgba[y * row_stride..y * row_stride + frame.width as usize * 4];
                for px in row.chunks_exact(4) {
                    rgb.extend_from_slice(&px[..3]);
                }
            }
            last_frame.0 = Some((frame.width, frame.height, rgb));
        } else if job.preroll_remaining == 0 && !job.done {
            // Screenshot frame after pre-roll — save + tear down.
            let mut rgba = Vec::with_capacity((frame.width * frame.height * 4) as usize);
            let row_stride = frame.padded_bytes_per_row;
            for y in 0..frame.height as usize {
                let row = &frame.rgba[y * row_stride..y * row_stride + frame.width as usize * 4];
                rgba.extend_from_slice(row);
            }
            let save_path = job.path.clone();
            let ok = save_png(&save_path, frame.width, frame.height, &rgba);
            let msg = if ok {
                format!("screenshot saved to {save_path}")
            } else {
                format!("failed to write screenshot {save_path}")
            };
            info!("{msg}");
            let _ = channels.evt_tx.send(forge_ipc::RuntimeToEditor::ScreenshotDone {
                path: save_path,
                success: ok,
                message: msg,
            });
            // Restore the editor camera back onto the viewport target.
            if let Some(cam) = job.camera.take() {
                if let Ok(mut ec) = commands.get_entity(cam) {
                    ec.try_insert(CameraRenderTarget::Image(
                        viewport_target.0.clone().into(),
                    ));
                }
            }
            if let Some(cop) = job.copier.take() {
                if let Ok(mut ec) = commands.get_entity(cop) {
                    ec.despawn();
                }
            }
            job.done = true;
            pending.request = None;
        }
        // Screenshot frames during pre-roll are discarded.
    }
}

/// Spawns screenshot jobs and drives their pre-roll countdown. Also handles
/// the `--screenshot` startup one-shot (exits the app when done).
pub fn manage_screenshot_jobs(
    mut commands: Commands,
    render_device: Option<Res<RenderDevice>>,
    mut images: ResMut<Assets<Image>>,
    editor_camera: Query<Entity, With<crate::state::EditorOnlyTag>>,
    mut job: ResMut<ScreenshotJob>,
    mut pending: ResMut<PendingScreenshot>,
    startup: Res<StartupShot>,
    mut app_exit: MessageWriter<AppExit>,
    mut queued_startup: Local<bool>,
    mut exit_delay: Local<u32>,
) {
    // Startup one-shot: queue once.
    if !*queued_startup && startup.path.is_some() && render_device.is_some() {
        *queued_startup = true;
        pending.request =
            Some((startup.path.clone().unwrap_or_default(), startup.width, startup.height));
    }

    // Begin a queued job: swap the editor camera onto a hi-res target.
    if job.camera.is_none() {
        if let Some((path, width, height)) = pending.request.take() {
            let Some(device) = render_device.as_deref() else { return };
            let Some(cam) = editor_camera.iter().next() else { return };
            let (copier_entity, _shot_handle) =
                attach_render_target(&mut commands, &mut images, device, cam, width, height, 1);
            job.camera = Some(cam);
            job.copier = Some(copier_entity);
            job.path = path;
            job.preroll_remaining = 45;
            job.done = false;
        }
    }

    // Pre-roll countdown.
    if job.camera.is_some() && job.preroll_remaining > 0 {
        job.preroll_remaining -= 1;
    }

    // Startup one-shot completion → exit.
    if *queued_startup && startup.path.is_some() && job.done {
        *exit_delay += 1;
        if *exit_delay > 3 {
            info!("screenshot mode complete; exiting");
            app_exit.write(AppExit::Success);
        }
    }
}

fn save_png(path: &str, width: u32, height: u32, rgba: &[u8]) -> bool {
    let Some(img) = image::RgbaImage::from_raw(width, height, rgba.to_vec()) else {
        return false;
    };
    img.save(path).is_ok()
}

/// Pushes the latest viewport frame to the editor at half the render rate.
pub fn push_frames(
    mut counter: Local<u32>,
    last_frame: Res<LastViewportFrame>,
    channels: Res<IpcChannels>,
) {
    *counter += 1;
    if *counter % 2 != 0 {
        return;
    }
    if last_frame.is_changed() {
        if let Some((w, h, rgb)) = last_frame.0.as_ref() {
            let _ = channels.evt_tx.send(forge_ipc::RuntimeToEditor::Frame {
                width: *w,
                height: *h,
                rgb: rgb.clone(),
            });
        }
    }
}
