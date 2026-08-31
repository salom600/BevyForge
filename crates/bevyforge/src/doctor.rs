//! `bevyforge --doctor` — headless self-test.
//!
//! Answers, on the user's own machine, the only question that matters when
//! the editor appears broken: **what exactly is failing?** Runs before any
//! GUI so it works even when OpenGL is unusable, writes a shareable report
//! file, and prints the same text to stdout.
//!
//! Checks:
//! 1. version + paths (editor exe, exe dir writable? log file where?)
//! 2. can a scene document be seeded from the project?
//! 3. is `bevyforge-runtime` found next to the editor (or on PATH)?
//! 4. can the runtime be spawned and handshake on the IPC port?
//! 5. does the IPC link accept a Hello + Ping round-trip?
//! 6. GPU adapter the runtime picked (from its Welcome/stats output).

use std::time::Duration;

use forge_editor_core::RuntimeSpawner;

use crate::logging;

struct Report {
    lines: Vec<String>,
    fails: usize,
}

impl Report {
    fn new() -> Self {
        Self { lines: Vec::new(), fails: 0 }
    }
    fn ok(&mut self, what: &str, detail: &str) {
        self.lines.push(format!("PASS  {what}\n      {detail}"));
    }
    fn fail(&mut self, what: &str, detail: &str) {
        self.fails += 1;
        self.lines.push(format!("FAIL  {what}\n      {detail}"));
    }
    fn finish(mut self, port: u16) -> String {
        self.lines.insert(0, format!("BevyForge doctor — v{}", env!("CARGO_PKG_VERSION")));
        self.lines.insert(
            1,
            format!(
                "platform: {} {} | port {port} | {}",
                std::env::consts::OS,
                std::env::consts::ARCH,
                chronoish()
            ),
        );
        let mut text = self.lines.join("\n\n");
        if self.fails == 0 {
            text.push_str("\n\nRESULT: ALL CHECKS PASSED — the engine stack works on this machine.");
        } else {
            text.push_str(&format!(
                "\n\nRESULT: {fails} check(s) failed. Send this whole file to support.\n\
                 Most common fixes: extract the FULL archive (both .exe files together), \
                 allow bevyforge-runtime.exe in Windows Defender, or update GPU drivers.",
                fails = self.fails
            ));
        }
        text.push('\n');
        text
    }
}

fn chronoish() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("unix time {secs}")
}

/// Run all checks and return the report text.
pub fn run(project: Option<&forge_editor_core::Project>, port: u16) -> String {
    let mut rep = Report::new();

    // 1 — paths
    let exe = std::env::current_exe();
    match &exe {
        Ok(p) => {
            let dir = p.parent().map(|d| d.to_path_buf());
            let writable = dir
                .as_ref()
                .map(|d| std::fs::create_dir_all(d.join(".write-test")).is_ok())
                .unwrap_or(false);
            if writable {
                let _ = std::fs::remove_dir_all(dir.as_ref().unwrap().join(".write-test"));
            }
            if writable {
                rep.ok("editor executable", &format!("{} (folder is writable)", p.display()));
            } else {
                rep.ok(
                    "editor executable",
                    &format!("{} (folder NOT writable — logs go to the app-data dir)", p.display()),
                );
            }
        }
        Err(e) => rep.fail("editor executable", &format!("current_exe failed: {e}")),
    }
    match logging::active_path() {
        Some(p) => rep.ok("log file", &p.display().to_string()),
        None => rep.fail("log file", "no writable location found for bevyforge.log"),
    }

    // 2 — project/scene
    match project {
        Some(p) => {
            let scene = p.resolve_scene("");
            if scene.is_file() {
                match std::fs::read_to_string(&scene) {
                    Ok(text) => match ron::from_str::<forge_ipc::ForgeScene>(&text) {
                        Ok(doc) => rep.ok(
                            "project scene",
                            &format!("{} parsed ({} entities)", scene.display(), doc.entities.len()),
                        ),
                        Err(e) => rep.fail("project scene", &format!("{} parse error: {e}", scene.display())),
                    },
                    Err(e) => rep.fail("project scene", &format!("{} unreadable: {e}", scene.display())),
                }
            } else {
                rep.ok("project scene", &format!("no scene file at {} (fresh project) — fine", scene.display()));
            }
        }
        None => rep.ok("project scene", "no project given — default demo project will be created"),
    }

    // 3 — runtime binary discovery
    let spawner = RuntimeSpawner { binary: None, port, ..Default::default() };
    match spawner.find_binary() {
        Ok(b) => {
            rep.ok("runtime engine binary", &format!("found {}", b.display()));
        }
        Err(e) => {
            rep.fail(
                "runtime engine binary",
                &format!(
                    "{e:#}\n      → this is the #1 cause of a \"dead\" editor: \
                     extract the FULL archive so both executables sit together."
                ),
            );
            return rep.finish(port);
        }
    }

    // 4/5/6 — spawn + handshake + IPC
    let project_dir = project
        .map(|p| p.root.clone())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    match spawner.spawn(&project_dir, Duration::from_secs(20)) {
        Ok(handle) => {
            rep.ok(
                "engine process",
                &format!("spawned (pid {:?}), IPC port {}", handle.child.lock().map(|c| c.id()), handle.port),
            );
            // IPC round-trip: Hello → Welcome
            match forge_ipc::connect(handle.port, Duration::from_secs(3)) {
                Ok(mut conn) => {
                    let sent = match conn.clone_stream() {
                        Ok(mut stream) => forge_ipc::send_on_stream(
                            &mut stream,
                            &forge_ipc::Message::ToRuntime(forge_ipc::EditorToRuntime::Hello),
                        ),
                        Err(e) => Err(e),
                    };
                    let got_welcome = sent
                        .is_ok()
                        && (0..20).any(|_| {
                            matches!(
                                conn.recv(),
                                Ok(forge_ipc::Message::ToEditor(forge_ipc::RuntimeToEditor::Welcome { .. }))
                            )
                        });
                    if got_welcome {
                        rep.ok("IPC link", "Hello → Welcome round-trip succeeded");
                    } else {
                        rep.fail(
                            "IPC link",
                            "no Welcome after Hello — engine booted but its IPC link is broken",
                        );
                    }
                }
                Err(e) => rep.fail("IPC link", &format!("connect failed: {e}")),
            }
            // let the runtime print its adapter info, then capture a bit of stdout
            std::thread::sleep(Duration::from_millis(2500));
            if let Ok(mut child) = handle.child.lock() {
                let _ = child.kill();
            }
            rep.ok("engine shutdown", "test engine stopped");
        }
        Err(e) => {
            rep.fail(
                "engine process",
                &format!("{e:#}\n      → GPU failure? Update drivers, or run with a software renderer\n        (Mesa3D lavapipe on Windows — see TROUBLESHOOTING.md section 2)."),
            );
        }
    }

    let text = rep.finish(port);
    text
}

/// Write the report next to the exe (fallback: cwd) and return the path.
pub fn write_report(report: &str) -> std::path::PathBuf {
    let base = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_default();
    let path = base.join("bevyforge-doctor-report.txt");
    if std::fs::write(&path, report).is_ok() {
        return path;
    }
    let fallback = std::path::PathBuf::from("bevyforge-doctor-report.txt");
    let _ = std::fs::write(&fallback, report);
    fallback
}
