//! Top menu bar (design row 1): logo, menus, transport controls, badges.
//! Also: status bar, toasts and keyboard shortcuts.

use egui::{Context, Vec2};
use forge_ipc::{ComponentKind, EditorToRuntime, LogLevel};

use crate::app::{BevyForgeApp, DialogPurpose};
use crate::panels;
use crate::state::ViewportTab;
use crate::theme;

pub fn top_menu_bar(app: &mut BevyForgeApp, ui: &mut egui::Ui) {
    egui::Panel::top("menu_bar")
        .exact_size(34.0)
        .resizable(false)
        .frame(
            egui::Frame::new()
                .fill(theme::BG_HEADER)
                .stroke(egui::Stroke::new(1.0, theme::BORDER))
                .inner_margin(egui::Margin::symmetric(8, 2)),
        )
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                // Logo block.
                ui.label(
                    egui::RichText::new("◆")
                        .size(16.0)
                        .color(theme::ACCENT),
                );
                ui.label(
                    egui::RichText::new("BevyForge")
                        .strong()
                        .size(14.0)
                        .color(theme::TEXT),
                );
                ui.label(
                    egui::RichText::new(concat!("0.1.0", " · ", "bevy 0.19"))
                        .small()
                        .color(theme::TEXT_DIM),
                );
                ui.separator();

                // Menus.
                menu_file(app, ui);
                menu_edit(app, ui);
                menu_game_object(app, ui);
                menu_component(app, ui);
                menu_window(app, ui);
                menu_help(app, ui);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Build config badge (informational).
                    ui.label(
                        egui::RichText::new("Rust 1.98")
                            .small()
                            .color(theme::TEXT_DIM),
                    );
                    ui.label(
                        egui::RichText::new(if cfg!(debug_assertions) { "DEBUG" } else { "RELEASE" })
                            .small()
                            .color(theme::ORANGE)
                            .strong(),
                    );

                    ui.separator();

                    // Transport controls.
                    let play_label = if app.state.playing { "Stop" } else { "Play" };
                    let play = ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(play_label).size(12.0).color(if app.state.playing { theme::YELLOW } else { theme::GREEN }),
                            )
                            ,
                        )
                        .on_hover_text(if app.state.playing { "Stop play mode (restore snapshot)" } else { "Enter play mode (runs gameplay systems)" });
                    if play.clicked() {
                        app.toggle_play();
                    }
                    let stop = ui
                        .add(
                            egui::Button::new(egui::RichText::new("Stop").size(11.0).color(theme::RED))
                                ,
                        )
                        .on_hover_text("Stop play mode");
                    if stop.clicked() && app.state.playing {
                        app.toggle_play();
                    }
                    let pause = ui
                        .add(
                            egui::Button::new(egui::RichText::new("Anim").size(11.0).color(theme::TEXT_DIM))
                                ,
                        )
                        .on_hover_text("Pause/Resume animation playback");
                    if pause.clicked() {
                        let target = !app.state.anim.playing;
                        app.cmd(EditorToRuntime::SetAnimPlaying(target));
                    }
                });
            });
        });
}

fn menu_file(app: &mut BevyForgeApp, ui: &mut egui::Ui) {
    ui.menu_button("Project", |ui| {
        if ui.button("New Scene").clicked() {
            app.cmd(EditorToRuntime::NewScene);
            ui.close();
        }
        if ui.button("Open Scene…").clicked() {
            app.dialog = Some(DialogPurpose::OpenScene);
            app.file_dialog.pick_file();
            ui.close();
        }
        if ui.button("Save Scene").clicked() {
            save_scene(app);
            ui.close();
        }
        if ui.button("Save Scene As…").clicked() {
            app.dialog = Some(DialogPurpose::SaveSceneAs);
            app.file_dialog.save_file();
            ui.close();
        }
        ui.separator();
        if ui.button("Open Project…").clicked() {
            app.dialog = Some(DialogPurpose::OpenProject);
            app.file_dialog.pick_directory();
            ui.close();
        }
        if ui.button("New Project…").clicked() {
            app.dialog = Some(DialogPurpose::NewProject);
            app.file_dialog.pick_directory();
            ui.close();
        }
        ui.separator();
        if ui.button("Take Screenshot…").clicked() {
            let path = format!(
                "screenshot-{}.png",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            );
            app.cmd(EditorToRuntime::RequestScreenshot { path });
            ui.close();
        }
        ui.separator();
        if ui.button("Quit").clicked() {
            if let Some(net) = &app.net {
                net.shutdown();
            }
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    });
}

fn save_scene(app: &mut BevyForgeApp) {
    let path = match (&app.state.scene_path, &app.project) {
        (Some(p), _) => p.clone(),
        (None, Some(project)) => project.resolve_scene("").to_string_lossy().to_string(),
        (None, None) => "assets/scenes/main.scn.ron".to_string(),
    };
    app.save_scene_to(path);
}

fn menu_edit(app: &mut BevyForgeApp, ui: &mut egui::Ui) {
    ui.menu_button("Edit", |ui| {
        let undo_label = format!(
            "Undo {}",
            app.undo.top_label().unwrap_or("")
        );
        if ui
            .add_enabled(app.undo.can_undo(), egui::Button::new(undo_label))
            .clicked()
        {
            if let Some(entry) = app.undo.pop_undo() {
                for c in entry.undo {
                    app.cmd(c);
                }
            }
            ui.close();
        }
        if ui
            .add_enabled(app.undo.can_redo(), egui::Button::new("Redo"))
            .clicked()
        {
            if let Some(entry) = app.undo.pop_redo() {
                for c in entry.redo {
                    app.cmd(c);
                }
            }
            ui.close();
        }
        ui.separator();
        if ui.add_enabled(app.selected_entity().is_some(), egui::Button::new("Duplicate")).clicked() {
            app.duplicate_selected();
            ui.close();
        }
        if ui.add_enabled(app.selected_entity().is_some(), egui::Button::new("Delete")).clicked() {
            app.delete_selected();
            ui.close();
        }
    });
}

fn spawn_kind(app: &mut BevyForgeApp, ui: &mut egui::Ui, kind: forge_ipc::EntityKind) {
    let label = kind.label();
    if ui.button(label).clicked() {
        let name = match kind {
            forge_ipc::EntityKind::Mesh(p) => p.label().to_string(),
            forge_ipc::EntityKind::PlayerPrefab => "Player".to_string(),
            forge_ipc::EntityKind::Empty => "Empty".to_string(),
            other => other.label().to_string(),
        };
        let parent = app.selected_entity();
        app.cmd(EditorToRuntime::SpawnEntity { name, parent, kind });
        ui.close();
    }
}

fn menu_game_object(app: &mut BevyForgeApp, ui: &mut egui::Ui) {
    ui.menu_button("GameObject", |ui| {
        ui.set_min_width(160.0);
        ui.label(egui::RichText::new("3D Object").small().color(theme::TEXT_DIM));
        for prim in forge_ipc::MeshPrimitive::ALL {
            spawn_kind(app, ui, forge_ipc::EntityKind::Mesh(*prim));
        }
        ui.separator();
        ui.label(egui::RichText::new("Light").small().color(theme::TEXT_DIM));
        spawn_kind(app, ui, forge_ipc::EntityKind::DirectionalLight);
        spawn_kind(app, ui, forge_ipc::EntityKind::PointLight);
        spawn_kind(app, ui, forge_ipc::EntityKind::SpotLight);
        ui.separator();
        spawn_kind(app, ui, forge_ipc::EntityKind::Camera);
        spawn_kind(app, ui, forge_ipc::EntityKind::Empty);
        ui.separator();
        ui.label(egui::RichText::new("Prefab").small().color(theme::TEXT_DIM));
        spawn_kind(app, ui, forge_ipc::EntityKind::PlayerPrefab);
    });
}

fn add_component(app: &mut BevyForgeApp, ui: &mut egui::Ui, kind: ComponentKind) {
    if let Some(entity) = app.selected_entity() {
        app.cmd(EditorToRuntime::AddComponent { entity, component: kind });
    } else {
        app.state.push_toast(LogLevel::Warn, "Select an entity first");
    }
}

fn menu_component(app: &mut BevyForgeApp, ui: &mut egui::Ui) {
    ui.menu_button("Component", |ui| {
        ui.set_min_width(180.0);
        ui.label(
            egui::RichText::new("Add to selection:")
                .small()
                .color(theme::TEXT_DIM),
        );
        for kind in forge_ipc::ComponentKind::ADDABLE {
            if ui.button(kind.label()).clicked() {
                add_component(app, ui, *kind);
                ui.close();
            }
        }
    });
}

fn menu_window(app: &mut BevyForgeApp, ui: &mut egui::Ui) {
    ui.menu_button("Window", |ui| {
        toggle(ui, "Scene Hierarchy", &mut app.state.show_hierarchy);
        toggle(ui, "Asset Browser", &mut app.state.show_assets);
        toggle(ui, "Inspector", &mut app.state.show_inspector);
        toggle(ui, "Environment && Lighting", &mut app.state.show_environment);
        toggle(ui, "Timeline", &mut app.state.show_timeline);
        toggle(ui, "Console", &mut app.state.show_console);
        toggle(ui, "Rust Compiler", &mut app.state.show_compiler);
    });
}

fn toggle(ui: &mut egui::Ui, label: &str, value: &mut bool) {
    let mut v = *value;
    if ui.checkbox(&mut v, label).changed() {
        *value = v;
    }
}

fn menu_help(app: &mut BevyForgeApp, ui: &mut egui::Ui) {
    ui.menu_button("Help", |ui| {
        if ui.button("About BevyForge").clicked() {
            app.state.push_toast(
                LogLevel::Info,
                format!(
                    "BevyForge {} — standalone Bevy editor. Runtime: Bevy {} (pid {:?}).",
                    env!("CARGO_PKG_VERSION"),
                    if app.state.bevy_version.is_empty() { "not connected" } else { &app.state.bevy_version },
                    app.state.runtime_pid
                ),
            );
            ui.close();
        }
        if ui.button("Run cargo check").clicked() {
            app.check_requested = true;
            ui.close();
        }
    });
}

// ---------------------------------------------------------------------------
// Status bar
// ---------------------------------------------------------------------------

pub fn status_bar(app: &mut BevyForgeApp, ui: &mut egui::Ui) {
    egui::Panel::bottom("status_bar")
        .exact_size(24.0)
        .resizable(false)
        .frame(
            egui::Frame::new()
                .fill(theme::BG_HEADER)
                .stroke(egui::Stroke::new(1.0, theme::BORDER))
                .inner_margin(egui::Margin::symmetric(8, 1)),
        )
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                let (dot, label) = if app.state.connected {
                    (theme::GREEN, "Ready")
                } else {
                    (theme::RED, app.state.status_message.as_str())
                };
                ui.label(egui::RichText::new("●").color(dot).small());
                ui.label(egui::RichText::new(label).small().color(theme::TEXT_DIM));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let s = &app.state.stats;
                    let mono = |txt: String| egui::RichText::new(txt).small().monospace().color(theme::TEXT_DIM);
                    if !s.backend.is_empty() {
                        let short_gpu: String = s.backend.chars().take(22).collect();
                        ui.label(mono(format!("GPU: {short_gpu}")));
                        ui.separator();
                    }
                    if s.mem_mib > 0.0 {
                        ui.label(mono(format!("Memory: {:.1} MB", s.mem_mib)));
                        ui.separator();
                    }
                    ui.label(mono(format!("Frame: {:.2} ms", s.frame_ms)));
                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!("FPS: {:.0}", s.fps.min(9999.0)))
                            .small()
                            .monospace()
                            .color(if s.fps > 30.0 { theme::GREEN } else { theme::YELLOW }),
                    );
                    ui.separator();
                    ui.label(mono(format!("Systems: {}", s.system_count)));
                    ui.separator();
                    ui.label(mono(format!("Entities: {}", s.entity_count)));
                    let _ = crate::panels::clock_hms();
                });
            });
        });
}

// ---------------------------------------------------------------------------
// Toasts
// ---------------------------------------------------------------------------

pub fn draw_toasts(app: &mut BevyForgeApp, ctx: &Context) {
    let toasts = app.state.toasts.clone();
    if toasts.is_empty() {
        return;
    }
    egui::Area::new(egui::Id::new("toasts"))
        .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -32.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.set_max_width(420.0);
            for (time, level, message) in toasts.iter().rev() {
                let age = time.elapsed().as_secs_f32();
                let alpha = (1.0 - (age - 3.5) / 1.5).clamp(0.15, 1.0);
                let color = crate::panels::level_color(*level);
                egui::Frame::new()
                    .fill(theme::BG_WIDGET.gamma_multiply(alpha as f32 * 0.0 + 1.0))
                    .stroke(egui::Stroke::new(1.0, color))
                    .corner_radius(4)
                    .inner_margin(egui::Margin::symmetric(8, 5))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width().min(400.0));
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("●").small().color(color));
                            ui.label(
                                egui::RichText::new(message)
                                    .small()
                                    .color(theme::TEXT.gamma_multiply(alpha)),
                            );
                        });
                    });
                ui.add_space(4.0);
            }
        });
}

// ---------------------------------------------------------------------------
// Shortcuts
// ---------------------------------------------------------------------------

pub fn handle_shortcuts(app: &mut BevyForgeApp, ctx: &Context) {
    if !app.shortcuts_enabled {
        return;
    }
    if let Some(script_idx) = app.state.active_script {
        let dirty = app.state.scripts.get(script_idx).map(|d| d.dirty).unwrap_or(false);
        if dirty && ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
            app.save_script(script_idx);
            return;
        }
    }
    ctx.input(|i| {
        if i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::S) {
            // Save scene.
            let path = match (&app.state.scene_path, &app.project) {
                (Some(p), _) => p.clone(),
                (None, Some(project)) => project.resolve_scene("").to_string_lossy().to_string(),
                _ => "assets/scenes/main.scn.ron".to_string(),
            };
            app.save_scene_to(path);
        } else if i.modifiers.command && i.key_pressed(egui::Key::Z) && !i.modifiers.shift {
            if let Some(entry) = app.undo.pop_undo() {
                for c in entry.undo {
                    app.cmd(c);
                }
            }
        } else if (i.modifiers.command && i.key_pressed(egui::Key::Y))
            || (i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::Z))
        {
            if let Some(entry) = app.undo.pop_redo() {
                for c in entry.redo {
                    app.cmd(c);
                }
            }
        } else if i.modifiers.command && i.key_pressed(egui::Key::D) {
            app.duplicate_selected();
        } else if i.key_pressed(egui::Key::Delete) {
            app.delete_selected();
        } else if i.key_pressed(egui::Key::F) && !i.modifiers.command {
            if let Some(sel) = app.state.selected {
                // Frame selection: focus orbit target on the entity position
                // (from the inspector translation row when available).
                if let Some(t) = app.state.components.iter().find_map(|c| {
                    c.rows.iter().find_map(|(f, r)| {
                        if *f == forge_ipc::ComponentField::Translation {
                            match r.value {
                                forge_ipc::FieldValue::Vec3(v) => Some(v),
                                _ => None,
                            }
                        } else {
                            None
                        }
                    })
                }) {
                    app.state.camera_rig.0 = t;
                    app.send_camera();
                }
                let _ = sel;
            }
        }
    });
    let _ = ViewportTab::Scene;
}
