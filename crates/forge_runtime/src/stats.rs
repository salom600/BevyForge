//! Status-bar statistics: FPS, frame time, entity count, system count,
//! resident memory (Linux), and the active wgpu backend.

use bevy::render::renderer::RenderAdapterInfo;
use bevy::prelude::*;

use crate::state::IpcChannels;

/// Honest count of systems registered by BevyForge itself (runtime plugin
/// systems + forge_scripts systems). Maintained as a constant here and
/// incremented when the runtime gains systems.
pub const FORGE_SYSTEM_COUNT: u32 = 22;

#[derive(Resource)]
struct StatsThrottle(u32);

/// Sample stats every 15 frames and ship them to the editor.
pub fn collect_stats(world: &mut World) {
    static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let tick = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if tick % 15 != 0 {
        return;
    }

    let dt = world.resource::<Time>().delta_secs();
    let fps = if dt > 0.0 { 1.0 / dt } else { 0.0 };
    let frame_ms = dt * 1000.0;
    let entity_count = world.entities().len() as u32;

    let backend = world
        .get_resource::<RenderAdapterInfo>()
        .map(|a| format!("{} ({:?})", a.name, a.backend))
        .unwrap_or_else(|| "unknown".to_string());

    let mem_mib = resident_memory_mib();
    let stats = forge_ipc::Stats {
        fps,
        frame_ms,
        entity_count,
        system_count: FORGE_SYSTEM_COUNT,
        mem_mib,
        backend,
    };
    let channels = world.resource::<IpcChannels>().evt_tx.clone();
    let _ = channels.send(forge_ipc::RuntimeToEditor::Stats(stats));
}

fn resident_memory_mib() -> f32 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(statm) = std::fs::read_to_string("/proc/self/statm") {
            if let Some(resident_pages) = statm.split_whitespace().nth(1) {
                if let Ok(pages) = resident_pages.parse::<u64>() {
                    return (pages * 4096) as f32 / (1024.0 * 1024.0);
                }
            }
        }
        0.0
    }
    #[cfg(not(target_os = "linux"))]
    {
        0.0
    }
}
