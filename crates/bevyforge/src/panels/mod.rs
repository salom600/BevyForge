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

/// Vector icon per hierarchy node kind.
pub fn node_icon(icon: forge_ipc::NodeIcon) -> (crate::icons::Icon, egui::Color32) {
    use egui::Color32;
    use forge_ipc::NodeIcon;
    match icon {
        NodeIcon::Camera => (crate::icons::Icon::Camera, Color32::from_rgb(0x64, 0xb5, 0xf6)),
        NodeIcon::Light => (crate::icons::Icon::Light, Color32::from_rgb(0xff, 0xd5, 0x4f)),
        NodeIcon::Mesh => (crate::icons::Icon::Cube, Color32::from_rgb(0x9a, 0xb4, 0xd0)),
        NodeIcon::Player => (crate::icons::Icon::Player, crate::theme::ORANGE),
        NodeIcon::Script => (crate::icons::Icon::Script, Color32::from_rgb(0x81, 0xc7, 0x84)),
        NodeIcon::Group => (crate::icons::Icon::Group, crate::theme::TEXT_DIM),
        NodeIcon::Environment => (crate::icons::Icon::Env, Color32::from_rgb(0x9f, 0xa8, 0xda)),
    }
}

/// Vector file icon by extension for the asset browser.
pub fn file_icon(name: &str, is_dir: bool) -> (crate::icons::Icon, egui::Color32) {
    use egui::Color32;
    if is_dir {
        return (crate::icons::Icon::Folder, Color32::from_rgb(0xe8, 0xb3, 0x4b));
    }
    let lower = name.to_lowercase();
    if lower.ends_with(".scn.ron") || lower.ends_with(".ron") {
        (crate::icons::Icon::Scene, Color32::from_rgb(0x64, 0xb5, 0xf6))
    } else if lower.ends_with(".rs") {
        (crate::icons::Icon::Script, crate::theme::ORANGE)
    } else if lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        (crate::icons::Icon::Image, Color32::from_rgb(0x81, 0xc7, 0x84))
    } else if lower.ends_with(".toml") {
        (crate::icons::Icon::Config, crate::theme::TEXT_DIM)
    } else if lower.ends_with(".gltf") || lower.ends_with(".glb") || lower.ends_with(".obj") {
        (crate::icons::Icon::Material, Color32::from_rgb(0xb3, 0x9d, 0xdb))
    } else {
        (crate::icons::Icon::File, crate::theme::TEXT_DIM)
    }
}

/// Icon for a console / toast log level.
pub fn level_icon(level: forge_ipc::LogLevel) -> crate::icons::Icon {
    use forge_ipc::LogLevel;
    match level {
        LogLevel::Info => crate::icons::Icon::Info,
        LogLevel::Warn => crate::icons::Icon::Warn,
        LogLevel::Error => crate::icons::Icon::Error,
        LogLevel::Debug | LogLevel::Trace => crate::icons::Icon::Console,
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
