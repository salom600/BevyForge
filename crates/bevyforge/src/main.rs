//! # bevyforge — the editor process
//!
//! A standalone eframe/egui desktop application. Spawns (or attaches to) a
//! `bevyforge-runtime` child process that owns the ECS world and streams
//! rendered frames; every panel talks to it through the forge_ipc protocol.

#![allow(clippy::too_many_arguments)]

mod app;
mod gizmo;
mod icons;
mod net;
mod panels;
mod state;
mod theme;

use forge_editor_core::Project;

fn main() -> anyhow::Result<()> {
    let mut project_dir: Option<std::path::PathBuf> = None;
    let mut connect_port: Option<u16> = None;
    let mut port = forge_ipc::DEFAULT_PORT;
    let mut exit_after: Option<f64> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--project" => project_dir = args.next().map(std::path::PathBuf::from),
            "--connect" => connect_port = args.next().and_then(|v| v.parse().ok()),
            "--port" => port = args.next().and_then(|v| v.parse().ok()).unwrap_or(port),
            "--exit-after" => exit_after = args.next().and_then(|v| v.parse().ok()),
            "--version" => {
                println!("bevyforge {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            other => {
                eprintln!("unknown argument: {other}");
                eprintln!("usage: bevyforge [--project <dir>] [--connect <port>] [--port N] [--exit-after SECS]");
                std::process::exit(2);
            }
        }
    }

    // Resolve the project: given dir, else default demo project under $HOME.
    let project = match &project_dir {
        Some(dir) => Some(Project::open(dir)?),
        None => {
            let dir = default_project_dir();
            let proj = if dir.join("BevyForge.toml").exists() {
                Project::open(&dir)?
            } else {
                Project::create(&dir, "Demo")?
            };
            Some(proj)
        }
    };

    // Network: attach or spawn.
    let net = if let Some(p) = connect_port {
        net::Net::attach(p)?
    } else {
        // Locate the runtime binary next to the editor; fall back to $PATH.
        match net::Net::spawn_runtime(
            project.as_ref().map(|p| p.root.clone()).unwrap_or_default().as_path(),
            port,
        ) {
            Ok(n) => n,
            Err(e) => {
                // Run UI-only with a clear status instead of crashing.
                eprintln!("warning: runtime spawn failed: {e:#}");
                net::Net::offline()
            }
        }
    };

    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("BevyForge")
            .with_inner_size([1560.0, 980.0])
            .with_min_inner_size([1100.0, 680.0]),
        ..Default::default()
    };

    let app_project = project.clone();
    eframe::run_native(
        "BevyForge",
        native,
        Box::new(move |cc| {
            let fonts_done = install_fonts(&cc.egui_ctx);
            let _ = fonts_done;
            Ok(Box::new(app::BevyForgeApp::new(
                cc,
                app_project,
                Some(net),
                exit_after,
            )) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe error: {e}"))?;

    Ok(())
}

fn default_project_dir() -> std::path::PathBuf {
    let base = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default());
    base.join("BevyForgeProjects").join("Demo")
}

/// Ensure glyphs used by the UI (box-drawing, arrows, a few symbols) render.
fn install_fonts(ctx: &egui::Context) -> bool {
    let mut fonts = egui::FontDefinitions::default();
    // egui's default fonts already cover the symbol ranges we use through the
    // fallback chain (Noto Symbols shipped in eframe). Nothing extra needed.
    fonts.families.entry(egui::FontFamily::Proportional).or_default();
    ctx.set_fonts(fonts);
    true
}
