//! # bevyforge — the editor process
//!
//! A standalone eframe/egui desktop application. Spawns (or attaches to) a
//! `bevyforge-runtime` child process that owns the ECS world and streams
//! rendered frames; every panel talks to it through the forge_ipc protocol.
//!
//! Startup is **never blocking**: the window opens immediately, the engine
//! spawn/attach runs on a worker thread, and while the engine is down the
//! editor stays fully usable through its offline scene document
//! (`crate::offline`).

#![allow(clippy::too_many_arguments)]
// Release builds on Windows hide the console (a GUI app); all diagnostics go
// to the log file and the in-app console instead.
#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

mod app;
mod doctor;
mod gizmo;
mod icons;
mod logging;
mod net;
mod offline;
mod panels;
mod state;
mod theme;

use forge_editor_core::Project;

fn main() -> anyhow::Result<()> {
    let mut project_dir: Option<std::path::PathBuf> = None;
    let mut connect_port: Option<u16> = None;
    let mut port = forge_ipc::DEFAULT_PORT;
    let mut exit_after: Option<f64> = None;
    let mut doctor_mode = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--project" => project_dir = args.next().map(std::path::PathBuf::from),
            "--connect" => connect_port = args.next().and_then(|v| v.parse().ok()),
            "--port" => port = args.next().and_then(|v| v.parse().ok()).unwrap_or(port),
            "--exit-after" => exit_after = args.next().and_then(|v| v.parse().ok()),
            "--doctor" | "--selftest" => doctor_mode = true,
            "--version" => {
                println!("bevyforge {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            other => {
                eprintln!("unknown argument: {other}");
                eprintln!(
                    "usage: bevyforge [--project <dir>] [--connect <port>] [--port N] \
                     [--exit-after SECS] [--doctor] [--version]"
                );
                std::process::exit(2);
            }
        }
    }

    logging::init();
    logging::info(&format!(
        "BevyForge editor {} starting (pid {})",
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    ));
    logging::info(&format!(
        "exe: {}",
        std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()
    ));

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

    // Self-test mode: run every engine-related check headlessly, write a
    // report next to the executable, print it, and exit before any GUI.
    if doctor_mode {
        let report = doctor::run(project.as_ref(), port);
        print!("{report}");
        let path = doctor::write_report(&report);
        logging::info(&format!("doctor report written: {}", path.display()));
        let healthy = report.contains("FAIL");
        if !healthy {
            println!("doctor: ALL CHECKS PASSED");
        }
        return Ok(());
    }

    // NOTE: no engine spawn here. The window opens immediately; the app's
    // lifecycle pump spawns/attaches on a worker thread during the first
    // frame, so a slow engine (or a fully broken machine) can never freeze
    // or delay the UI.
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("BevyForge {}", env!("CARGO_PKG_VERSION")))
            .with_inner_size([1560.0, 980.0])
            .with_min_inner_size([1100.0, 680.0]),
        ..Default::default()
    };

    let app_project = project.clone();
    let result = eframe::run_native(
        "BevyForge",
        native,
        Box::new(move |cc| {
            let _ = install_fonts(&cc.egui_ctx);
            Ok(Box::new(app::BevyForgeApp::new(
                cc,
                app_project,
                Some(net::Net::offline()),
                None,
                exit_after,
                connect_port,
            )) as Box<dyn eframe::App>)
        }),
    );
    match result {
        Ok(()) => {
            logging::info("editor exited normally");
            Ok(())
        }
        Err(e) => {
            let msg = format!("eframe failed to start: {e}");
            logging::error(&msg);
            logging::error(&format!(
                "hint: the editor needs OpenGL 3.3+; see TROUBLESHOOTING.md section 5"
            ));
            fatal_message_box("BevyForge — cannot start", &msg);
            Err(anyhow::anyhow!("eframe error: {e}"))
        }
    }
}

#[cfg(windows)]
fn fatal_message_box(title: &str, text: &str) {
    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(hwnd: *mut core::ffi::c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
    let full = format!("{text}\n\nDetails were written to the bevyforge.log file.");
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            wide(&full).as_ptr(),
            wide(title).as_ptr(),
            0x10, // MB_ICONERROR
        );
    }
}

#[cfg(not(windows))]
fn fatal_message_box(_title: &str, _text: &str) {}

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
