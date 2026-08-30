//! UI panels of the BevyForge editor.

pub mod central;
pub mod left;
pub mod menu;
pub mod right;

pub use central::central;
pub use left::{assets_panel, hierarchy_panel};
pub use menu::{draw_toasts, handle_shortcuts, status_bar, top_menu_bar};
pub use right::{environment_panel, inspector_panel};
pub use crate::theme::tool_button;

/// "12:45:10" wall clock stamp used by console/compile output lines.
pub fn clock_hms() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let secs = secs % 86_400;
    format!("{:02}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
}

/// Monochrome glyph per hierarchy node kind (design-style minimal icons).
pub fn node_glyph(icon: forge_ipc::NodeIcon) -> (&'static str, egui::Color32) {
    use egui::Color32;
    use forge_ipc::NodeIcon;
    match icon {
        NodeIcon::Camera => ("◧", Color32::from_rgb(0x64, 0xb5, 0xf6)),
        NodeIcon::Light => ("✳", Color32::from_rgb(0xff, 0xd5, 0x4f)),
        NodeIcon::Mesh => ("⬢", crate::theme::TEXT_DIM),
        NodeIcon::Player => ("☉", crate::theme::ORANGE),
        NodeIcon::Script => ("⚙", Color32::from_rgb(0x81, 0xc7, 0x84)),
        NodeIcon::Group => ("▤", crate::theme::TEXT_DIM),
        NodeIcon::Environment => ("❋", Color32::from_rgb(0x9f, 0xa8, 0xda)),
    }
}

/// File icon by extension for the asset browser.
pub fn file_glyph(name: &str, is_dir: bool) -> (&'static str, egui::Color32) {
    use egui::Color32;
    if is_dir {
        return ("▤", Color32::from_rgb(0xe8, 0xb3, 0x4b));
    }
    let lower = name.to_lowercase();
    if lower.ends_with(".scn.ron") || lower.ends_with(".ron") {
        ("❐", Color32::from_rgb(0x64, 0xb5, 0xf6))
    } else if lower.ends_with(".rs") {
        ("⬢", crate::theme::ORANGE)
    } else if lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        ("▩", Color32::from_rgb(0x81, 0xc7, 0x84))
    } else if lower.ends_with(".toml") {
        ("⚑", crate::theme::TEXT_DIM)
    } else if lower.ends_with(".gltf") || lower.ends_with(".glb") || lower.ends_with(".obj") {
        ("◈", Color32::from_rgb(0xb3, 0x9d, 0xdb))
    } else {
        ("▪", crate::theme::TEXT_DIM)
    }
}

pub fn level_color(level: forge_ipc::LogLevel) -> egui::Color32 {
    use forge_ipc::LogLevel;
    match level {
        LogLevel::Info => crate::theme::ACCENT,
        LogLevel::Warn => crate::theme::YELLOW,
        LogLevel::Error => crate::theme::RED,
        LogLevel::Debug => crate::theme::TEXT_DIM,
        LogLevel::Trace => crate::theme::TEXT_DIM,
    }
}
