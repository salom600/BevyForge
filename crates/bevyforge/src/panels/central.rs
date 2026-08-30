//! Central area: viewport (frame stream + tab strip + toolbar) and the bottom
//! dock (Timeline / Console / Output) plus the Rust Compiler side-by-side and
//! the script editor tabs.

use egui::{Context, Vec2};
use forge_ipc::{AnimTrack, ComponentField, EditorToRuntime, FieldValue};

use crate::app::{BevyForgeApp, KeyframeDrag};
use crate::state::{DockTab, ViewportTab};
use crate::theme;

pub fn central(app: &mut BevyForgeApp, ui: &mut egui::Ui) {
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(theme::BG_APP))
        .show(ui, |ui| {
            // Explicit two-band layout: viewport on top, dock at the bottom.
            let full = ui.max_rect();
            let dock_height = if any_dock_visible(app) {
                (full.height() * 0.36).clamp(150.0, 360.0)
            } else {
                0.0
            };

            let viewport_rect = egui::Rect::from_min_max(
                full.min,
                egui::pos2(full.max.x, full.max.y - dock_height),
            );
            draw_viewport(app, ui, viewport_rect);

            if dock_height > 0.0 {
                let dock_rect = egui::Rect::from_min_max(
                    egui::pos2(full.min.x, full.max.y - dock_height),
                    full.max,
                );
                draw_bottom_dock(app, ui, dock_rect);
            }
        });

    draw_image_preview_popup(app, ui.ctx());
}

fn any_dock_visible(app: &BevyForgeApp) -> bool {
    app.state.show_timeline || app.state.show_console || app.state.show_compiler
}

// ---------------------------------------------------------------------------
// Viewport
// ---------------------------------------------------------------------------

fn draw_viewport(app: &mut BevyForgeApp, ui: &mut egui::Ui, rect: egui::Rect) {
    // Tab strip.
    let tab_h = 24.0;
    let tab_rect = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width(), tab_h));
    ui.scope_builder(egui::UiBuilder::new().max_rect(tab_rect), |ui| {
        ui.horizontal(|ui| {
            ui.painter().rect_filled(tab_rect, 0, theme::BG_HEADER);
            tab_button(ui, "Scene", ViewportTab::Scene, app);
            tab_button(ui, "Game", ViewportTab::Game, app);
            // Script tabs.
            let scripts = app.state.scripts.clone();
            for (i, doc) in scripts.iter().enumerate() {
                let label = if doc.dirty {
                    format!("● {}", doc.name)
                } else {
                    doc.name.clone()
                };
                let selected = app.state.active_script == Some(i);
                if ui
                    .selectable_label(
                        selected,
                        egui::RichText::new(label).size(12.0),
                    )
                    .clicked()
                {
                    app.state.active_script = Some(i);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(active) = app.state.active_script {
                    if crate::panels::tool_button(ui, "✕", "Close script") {
                        app.state.scripts.remove(active);
                        app.state.active_script = if app.state.scripts.is_empty() {
                            None
                        } else {
                            Some(active.saturating_sub(1).min(app.state.scripts.len() - 1))
                        };
                    }
                }
                if ui
                    .selectable_label(
                        false,
                        egui::RichText::new("＋ Script").size(11.5).color(theme::TEXT_DIM),
                    )
                    .on_hover_text("Open a script from assets/scripts")
                    .clicked()
                {
                    if let Some(project) = app.project.clone() {
                        let dir = project.assets_dir().join("scripts");
                        if let Ok(entries) = std::fs::read_dir(&dir) {
                            let first = entries
                                .flatten()
                                .find(|e| e.path().extension().map(|x| x == "rs").unwrap_or(false));
                            if let Some(f) = first {
                                app.open_script(&f.path().to_string_lossy());
                            } else {
                                app.state.push_toast(
                                    forge_ipc::LogLevel::Info,
                                    format!("no scripts yet — assets/scripts is empty"),
                                );
                            }
                        }
                    }
                }
            });
        });
    });

    let viewport_content = egui::Rect::from_min_size(
        egui::pos2(rect.min.x, rect.min.y + tab_h),
        Vec2::new(rect.width(), rect.height() - tab_h),
    );

    // Script editor takes over when a script tab is active.
    if let Some(idx) = app.state.active_script {
        if app.state.scripts.get(idx).is_some() {
            draw_script_editor(app, ui, viewport_content, idx);
            return;
        }
    }

    // Toolbar.
    let tb_h = 30.0;
    let tb_rect = egui::Rect::from_min_size(
        egui::pos2(viewport_content.min.x, viewport_content.min.y),
        Vec2::new(viewport_content.width(), tb_h),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(tb_rect), |ui| {
        ui.painter().rect_filled(tb_rect, 0, theme::BG_HEADER);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            let persp = egui::Button::new(
                egui::RichText::new("◉ Perspective").size(11.5),
            );
            if ui.add(persp).clicked() {
                // Perspective is the only projection; reset the rig instead.
                app.state.camera_rig = ([0.0, 0.5, 0.0], 12.0, -35.0, 28.0);
                app.send_camera();
            }
            let grid = ui
                .selectable_label(app.state.show_grid, egui::RichText::new("▦ Grid").size(11.5))
                .on_hover_text("Toggle reference grid");
            if grid.clicked() {
                app.state.show_grid = !app.state.show_grid;
                let mut env = app.state.env.clone();
                env.show_grid = app.state.show_grid;
                app.state.env = env.clone();
                app.cmd(EditorToRuntime::SetEnvironment(env));
            }
            let outline = ui
                .selectable_label(
                    app.state.show_outline,
                    egui::RichText::new("◫ Outline").size(11.5),
                )
                .on_hover_text("Toggle selection outline");
            if outline.clicked() {
                app.state.show_outline = !app.state.show_outline;
                let mut env = app.state.env.clone();
                env.show_selection_outline = app.state.show_outline;
                app.state.env = env.clone();
                app.cmd(EditorToRuntime::SetEnvironment(env));
            }
            if ui
                .add(egui::Button::new(egui::RichText::new("⦿ Frame").size(11.5)))
                .on_hover_text("Frame selected entity (F)")
                .clicked()
            {
                frame_selected(app);
            }
            ui.separator();
            let scene_tab_is_game = app.state.viewport_tab == ViewportTab::Game;
            let cam_label = if scene_tab_is_game { "Scene Camera" } else { "Editor Camera" };
            ui.label(egui::RichText::new(format!("⌾ {cam_label}")).size(11.0).color(theme::TEXT_DIM));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some((w, h, _)) = app.state.frame {
                    ui.label(
                        egui::RichText::new(format!("{}×{}", w, h))
                            .size(10.5)
                            .color(theme::TEXT_DIM),
                    );
                }
            });
        });
    });

    let frame_rect = egui::Rect::from_min_size(
        egui::pos2(viewport_content.min.x, viewport_content.min.y + tb_h),
        Vec2::new(
            viewport_content.width(),
            viewport_content.height() - tb_h,
        ),
    );

    // The streamed frame.
    let (resp, image_rect) = draw_frame(app, ui, frame_rect);
    handle_viewport_input(app, ui, &resp, image_rect);
}

fn tab_button(ui: &mut egui::Ui, label: &str, tab: ViewportTab, app: &mut BevyForgeApp) {
    let selected = app.state.viewport_tab == tab;
    if ui.selectable_label(selected, egui::RichText::new(label).size(12.0)).clicked() {
        let prev = app.state.viewport_tab;
        app.state.viewport_tab = tab;
        app.state.active_script = None;
        if prev != tab {
            match tab {
                ViewportTab::Scene => {
                    app.cmd(EditorToRuntime::SetViewportCamera { entity: None });
                }
                ViewportTab::Game => {
                    // First camera-typed entity in the hierarchy.
                    let cam = app
                        .state
                        .hierarchy
                        .iter()
                        .find(|n| n.icon == forge_ipc::NodeIcon::Camera)
                        .map(|n| n.id);
                    match cam {
                        Some(id) => {
                            app.cmd(EditorToRuntime::SetViewportCamera { entity: Some(id) })
                        }
                        None => app.state.push_toast(
                            forge_ipc::LogLevel::Warn,
                            "No camera entity in the scene",
                        ),
                    }
                }
            }
        }
    }
}

fn draw_frame(
    app: &mut BevyForgeApp,
    ui: &mut egui::Ui,
    rect: egui::Rect,
) -> (egui::Response, Option<egui::Rect>) {
    let Some((w, h, image)) = &app.state.frame else {
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new(if app.state.connected {
                        "waiting for frames…"
                    } else {
                        "runtime offline"
                    })
                    .color(theme::TEXT_DIM),
                );
            });
        });
        let resp = ui.interact(rect, ui.id().with("viewport_wait"), egui::Sense::click_and_drag());
        return (resp, None);
    };
    let _ = (w, h);

    // Texture upload.
    let texture = match app.state.texture.clone() {
        Some(mut tex) => {
            tex.set(image.clone(), egui::TextureOptions::LINEAR);
            tex
        }
        None => {
            let tex = ui.ctx().load_texture("viewport", image.clone(), egui::TextureOptions::LINEAR);
            app.state.texture = Some(tex.clone());
            tex
        }
    };

    // Letterbox into rect.
    let iw = image.width() as f32;
    let ih = image.height() as f32;
    let scale = (rect.width() / iw).min(rect.height() / ih);
    let size = Vec2::new(iw * scale, ih * scale);
    let image_rect = egui::Rect::from_center_size(rect.center(), size);
    ui.painter().rect_filled(rect, 0, theme::BG_APP);
    ui.painter().image(
        texture.id(),
        image_rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );

    let resp = ui.interact(rect, ui.id().with("viewport"), egui::Sense::click_and_drag());
    (resp, Some(image_rect))
}

fn handle_viewport_input(
    app: &mut BevyForgeApp,
    ui: &mut egui::Ui,
    resp: &egui::Response,
    image_rect: Option<egui::Rect>,
) {
    let drag = resp.dragged();
    let button = ui.input(|i| i.pointer.secondary_down() || i.pointer.middle_down());

    if drag && button {
        let delta = resp.drag_delta();
        let secondary = ui.input(|i| i.pointer.secondary_down());
        let (target, dist, yaw, pitch) = app.state.camera_rig;
        if secondary {
            // Orbit.
            let yaw = yaw - delta.x * 0.45;
            let pitch = (pitch + delta.y * 0.35).clamp(-88.0, 88.0);
            app.state.camera_rig = (target, dist, yaw, pitch);
        } else {
            // Pan in camera plane.
            let yaw_r = yaw.to_radians();
            let pitch_r = pitch.to_radians();
            // Pan in the camera plane using plain scalar math.
            let pan_scale = dist * 0.0016;
            let right_x = -yaw_r.sin();
            let right_z = yaw_r.cos();
            let new_target = [
                target[0] + right_x * (-delta.x * pan_scale),
                target[1] + delta.y * pan_scale,
                target[2] + right_z * (-delta.x * pan_scale),
            ];
            let _ = pitch_r;
            app.state.camera_rig = (new_target, dist, yaw, pitch);
        }
        app.send_camera();
    }

    // Wheel zoom.
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            let (target, dist, yaw, pitch) = app.state.camera_rig;
            let dist = (dist * (1.0 + scroll * 0.001)).clamp(0.5, 500.0);
            app.state.camera_rig = (target, dist, yaw, pitch);
            app.send_camera();
        }
    }

    // Click select (pick) — only on clean clicks with left button.
    if resp.clicked()
        && !resp.dragged()
        && ui.input(|i| !i.pointer.secondary_down())
    {
        if let Some(ir) = image_rect {
            if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                if ir.contains(pos) {
                    let nx = ((pos.x - ir.min.x) / ir.width()).clamp(0.0, 1.0);
                    let ny = ((pos.y - ir.min.y) / ir.height()).clamp(0.0, 1.0);
                    app.cmd(EditorToRuntime::Pick { x: nx, y: ny });
                }
            }
        }
    }
}

fn frame_selected(app: &mut BevyForgeApp) {
    if let Some(t) = app.state.components.iter().find_map(|c| {
        c.rows.iter().find_map(|(f, r)| {
            if *f == ComponentField::Translation {
                match r.value {
                    FieldValue::Vec3(v) => Some(v),
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
}

// ---------------------------------------------------------------------------
// Script editor
// ---------------------------------------------------------------------------

fn draw_script_editor(app: &mut BevyForgeApp, ui: &mut egui::Ui, rect: egui::Rect, idx: usize) {
    // Snapshot the doc fields to avoid holding a mutable borrow across UI.
    let (path, dirty, text_snapshot) = match app.state.scripts.get(idx) {
        Some(doc) => (doc.path.clone(), doc.dirty, doc.text.clone()),
        None => return,
    };
    let header_h = 30.0;
    let header = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width(), header_h));
    ui.scope_builder(egui::UiBuilder::new().max_rect(header), |ui| {
        ui.painter().rect_filled(header, 0, theme::BG_HEADER);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.label(egui::RichText::new("⬢").color(theme::ORANGE));
            ui.label(
                egui::RichText::new(path)
                    .monospace()
                    .size(11.5)
                    .color(theme::TEXT_DIM),
            );
            if dirty {
                ui.label(egui::RichText::new("● unsaved").small().color(theme::YELLOW));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Save (Ctrl+S)").clicked() {
                    app.save_script(idx);
                }
                if ui.button("Check").clicked() {
                    app.check_requested = true;
                }
            });
        });
    });

    let body = egui::Rect::from_min_size(
        egui::pos2(rect.min.x, rect.min.y + header_h),
        Vec2::new(rect.width(), rect.height() - header_h),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
        let mut text = text_snapshot;
        let editor = egui::TextEdit::multiline(&mut text)
            .code_editor()
            .desired_width(ui.available_width())
            .font(egui::TextStyle::Monospace);
        let resp = ui.add(editor);
        if resp.changed() {
            if let Some(doc) = app.state.scripts.get_mut(idx) {
                doc.text = text;
                doc.dirty = true;
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Bottom dock: tabs + timeline + console + output + compiler
// ---------------------------------------------------------------------------

fn draw_bottom_dock(app: &mut BevyForgeApp, ui: &mut egui::Ui, rect: egui::Rect) {
    let tab_h = 24.0;
    let tab_rect = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width(), tab_h));
    ui.scope_builder(egui::UiBuilder::new().max_rect(tab_rect), |ui| {
        ui.painter().rect_filled(tab_rect, 0, theme::BG_HEADER);
        ui.horizontal(|ui| {
            dock_tab(ui, "Timeline", DockTab::Timeline, app, app.state.show_timeline);
            dock_tab(ui, "Console", DockTab::Console, app, app.state.show_console);
            dock_tab(ui, "Output", DockTab::Output, app, true);
        });
    });

    let content = egui::Rect::from_min_size(
        egui::pos2(rect.min.x, rect.min.y + tab_h),
        Vec2::new(rect.width(), rect.height() - tab_h),
    );

    // Right side: Rust Compiler panel (fixed share).
    let compiler_w = if app.state.show_compiler {
        (content.width() * 0.42).min(560.0)
    } else {
        0.0
    };
    let left_rect = egui::Rect::from_min_max(
        content.min,
        egui::pos2(content.max.x - compiler_w, content.max.y),
    );
    let compiler_rect = egui::Rect::from_min_max(
        egui::pos2(content.max.x - compiler_w, content.min.y),
        content.max,
    );

    if compiler_w > 0.0 {
        draw_compiler(app, ui, compiler_rect);
    }

    ui.scope_builder(egui::UiBuilder::new().max_rect(left_rect), |ui| {
        match app.state.dock_tab {
            DockTab::Timeline if app.state.show_timeline => draw_timeline(app, ui),
            DockTab::Console if app.state.show_console => draw_console(app, ui),
            _ => draw_output(app, ui),
        }
    });
}

fn dock_tab(ui: &mut egui::Ui, label: &str, tab: DockTab, app: &mut BevyForgeApp, enabled: bool) {
    let selected = app.state.dock_tab == tab;
    let mut text = egui::RichText::new(label).size(12.0);
    if !enabled {
        text = text.color(theme::TEXT_DIM).strikethrough();
    }
    if ui.add_enabled(enabled, egui::Button::selectable(selected, text)).clicked() {
        app.state.dock_tab = tab;
    }
}

// --- Timeline -------------------------------------------------------------

fn draw_timeline(app: &mut BevyForgeApp, ui: &mut egui::Ui) {
    ui.vertical(|ui| {
        // Transport row.
        ui.horizontal(|ui| {
            let play_label = if app.state.anim.playing { "Pause" } else { "Play" };
            if ui
                .add(egui::Button::new(
                    egui::RichText::new(play_label)
                        .color(if app.state.anim.playing { theme::YELLOW } else { theme::GREEN }),
                ))
                .clicked()
            {
                let target = !app.state.anim.playing;
                app.cmd(EditorToRuntime::SetAnimPlaying(target));
            }
            if ui.button("Rewind").on_hover_text("Stop (rewind to 0)").clicked() {
                app.cmd(EditorToRuntime::SetAnimTime(0.0));
                app.cmd(EditorToRuntime::SetAnimPlaying(false));
            }
            let mut looped = app.state.anim.looped;
            if ui.toggle_value(&mut looped, "Loop").changed() {
                app.cmd(EditorToRuntime::SetAnimLooped(looped));
            }
            let t = app.state.anim.time;
            ui.label(
                egui::RichText::new(format!(
                    "{:02}:{:05.2} / {:02}:{:05.2}",
                    (t / 60.0) as u32,
                    t % 60.0,
                    (app.state.anim.duration / 60.0) as u32,
                    app.state.anim.duration % 60.0
                ))
                .monospace()
                .color(theme::TEXT),
            );
            let mut dur = app.state.anim.duration;
            ui.add(
                egui::DragValue::new(&mut dur)
                    .speed(0.5)
                    .range(0.5..=600.0)
                    .suffix("s"),
            )
            .on_hover_text("Animation duration");
            if (dur - app.state.anim.duration).abs() > 0.01 {
                app.cmd(EditorToRuntime::SetAnimDuration(dur));
            }
            ui.separator();
            if ui
                .button("＋ Key from current")
                .on_hover_text("Add keyframes on all tracks of the selection at the playhead, capturing current transform values")
                .clicked()
            {
                add_keys_from_current(app);
            }
        });
        ui.separator();

        // Track rows.
        egui::ScrollArea::vertical()
            .id_salt("timeline_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if app.state.anim_tracks.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            egui::RichText::new(
                                "Select an entity and use “＋ Key from current” to start animating",
                            )
                            .color(theme::TEXT_DIM),
                        );
                    });
                }
                let tracks = app.state.anim_tracks.clone();
                for entry in tracks {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("▾ {}", entry.name))
                                .size(12.0)
                                .strong()
                                .color(theme::ORANGE),
                        );
                        if ui
                            .small_button("✕")
                            .on_hover_text("Clear all tracks for this entity")
                            .clicked()
                        {
                            app.cmd(EditorToRuntime::ClearTracks { entity: entry.entity });
                        }
                    });
                    for (track, keys) in &entry.tracks {
                        draw_track_row(app, ui, &entry.name, entry.entity, *track, keys);
                    }
                }
            });
    });
}

fn draw_track_row(
    app: &mut BevyForgeApp,
    ui: &mut egui::Ui,
    entity_name: &str,
    entity: u64,
    track: AnimTrack,
    keys: &[(f32, [f32; 3])],
) {
    let list_w = 150.0;
    ui.horizontal(|ui| {
        // Left label + key add.
        ui.allocate_ui(Vec2::new(list_w, 22.0), |ui| {
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.label(egui::RichText::new(track.label()).size(11.5).color(theme::TEXT_DIM));
                if ui
                    .small_button("＋")
                    .on_hover_text("Add keyframe at playhead (captures inspector value)")
                    .clicked()
                {
                    if let Some(value) = current_track_value(app, track) {
                        app.cmd(EditorToRuntime::AddKeyframe {
                            entity,
                            track,
                            time: app.state.anim.time,
                            value,
                        });
                    } else {
                        app.state.push_toast(
                            forge_ipc::LogLevel::Warn,
                            "Select the entity so its transform can be read",
                        );
                    }
                }
            });
        });
        ui.separator();

        // Ruler area.
        let ruler_width = ui.available_width() - 8.0;
        let duration = app.state.anim.duration.max(0.1);
        let (rect, resp) = ui.allocate_exact_size(
            Vec2::new(ruler_width, 22.0),
            egui::Sense::click_and_drag(),
        );
        let painter = ui.painter();
        painter.rect_filled(rect, 0, theme::BG_APP);
        // Time gridlines every 5s.
        let mut t = 0.0;
        while t <= duration {
            let x = rect.min.x + (t / duration) * rect.width();
            painter.line_segment(
                [egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)],
                egui::Stroke::new(1.0, theme::BORDER),
            );
            if t % 10.0 < 0.01 {
                painter.text(
                    egui::pos2(x + 2.0, rect.min.y + 1.0),
                    egui::Align2::LEFT_TOP,
                    format!("00:{:02}", t as u32),
                    egui::FontId::monospace(9.0),
                    theme::TEXT_DIM,
                );
            }
            t += 5.0;
        }

        // Playhead.
        let play_x = rect.min.x + (app.state.anim.time / duration) * rect.width();
        painter.line_segment(
            [egui::pos2(play_x, rect.min.y), egui::pos2(play_x, rect.max.y)],
            egui::Stroke::new(1.5, theme::ACCENT),
        );

        // Keyframes.
        let track_color = match track {
            AnimTrack::Translation => theme::AXIS_X,
            AnimTrack::Rotation => theme::AXIS_Y,
            AnimTrack::Scale => theme::AXIS_Z,
        };
        for (i, (time, _)) in keys.iter().enumerate() {
            let x = rect.min.x + (time / duration) * rect.width();
            let center = egui::pos2(x, rect.center().y);
            let dragging = matches!(
                &app.keyframe_drag,
                Some(d) if d.entity == entity && d.track == track && d.index == i
            );
            let size = if dragging { 6.0 } else { 5.0 };
            painter.rect_filled(
                egui::Rect::from_center_size(center, Vec2::splat(size * 2.0)),
                1.0,
                track_color,
            );
        }

        // Scrub on click/drag in empty space.
        if resp.dragged() || resp.is_pointer_button_down_on() {
            if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                if rect.contains(pos) {
                    if let Some(drag) = &app.keyframe_drag {
                        if drag.entity == entity && drag.track == track {
                            let new_time =
                                (((pos.x - rect.min.x) / rect.width()) * duration).clamp(0.0, duration);
                            app.cmd(EditorToRuntime::MoveKeyframe {
                                entity: drag.entity,
                                track: drag.track,
                                index: drag.index,
                                new_time,
                            });
                        }
                    } else {
                        let time = (((pos.x - rect.min.x) / rect.width()) * duration).clamp(0.0, duration);
                        app.cmd(EditorToRuntime::SetAnimTime(time));
                    }
                }
            }
        }
        // Start keyframe drag on diamond grab.
        if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
            if ui.input(|i| i.pointer.primary_down()) && rect.contains(pos) {
                for (i, (time, _)) in keys.iter().enumerate() {
                    let x = rect.min.x + (time / duration) * rect.width();
                    if (pos.x - x).abs() < 5.0 {
                        let _ = entity_name;
                        app.keyframe_drag = Some(KeyframeDrag { entity, track, index: i });
                    }
                }
            }
        }
        if ui.input(|i| i.pointer.any_released()) {
            app.keyframe_drag = None;
        }

        // Right-click a diamond deletes the keyframe.
        resp.context_menu(|ui| {
            if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                for (i, (time, _)) in keys.iter().enumerate() {
                    let x = rect.min.x + (time / duration) * rect.width();
                    if (pos.x - x).abs() < 5.0 {
                        ui.label(format!("Keyframe at {time:.2}s"));
                        if ui.button("Delete keyframe").clicked() {
                            app.cmd(EditorToRuntime::RemoveKeyframe { entity, track, index: i });
                            ui.close();
                        }
                    }
                }
            }
        });
    });
}

fn current_track_value(app: &BevyForgeApp, track: AnimTrack) -> Option<[f32; 3]> {
    let field = match track {
        AnimTrack::Translation => ComponentField::Translation,
        AnimTrack::Rotation => ComponentField::RotationEulerDeg,
        AnimTrack::Scale => ComponentField::Scale,
    };
    app.state.components.iter().find_map(|c| {
        c.rows.iter().find_map(|(f, r)| {
            if *f == field {
                match r.value {
                    FieldValue::Vec3(v) => Some(v),
                    _ => None,
                }
            } else {
                None
            }
        })
    })
}

fn add_keys_from_current(app: &mut BevyForgeApp) {
    let Some(entity) = app.selected_entity() else {
        app.state.push_toast(forge_ipc::LogLevel::Warn, "Select an entity first");
        return;
    };
    let time = app.state.anim.time;
    for track in [AnimTrack::Translation, AnimTrack::Rotation, AnimTrack::Scale] {
        if let Some(value) = current_track_value(app, track) {
            app.cmd(EditorToRuntime::AddKeyframe { entity, track, time, value });
        }
    }
}

// --- Console ---------------------------------------------------------------

fn draw_console(app: &mut BevyForgeApp, ui: &mut egui::Ui) {
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            let mut filter = app.state.log_filter.clone();
            let resp = ui.add(
                egui::TextEdit::singleline(&mut filter)
                    .hint_text("🔍 Filter…")
                    .desired_width(220.0),
            );
            if resp.changed() {
                app.state.log_filter = filter;
            }
            let levels = [
                (forge_ipc::LogLevel::Debug, "All"),
                (forge_ipc::LogLevel::Info, "Info+"),
                (forge_ipc::LogLevel::Warn, "Warn+"),
                (forge_ipc::LogLevel::Error, "Error"),
            ];
            egui::ComboBox::from_id_salt("log_level")
                .selected_text(levels.iter().find(|(l, _)| *l == app.state.log_level_min).map(|(_, n)| *n).unwrap_or("All"))
                .show_ui(ui, |ui| {
                    for (level, name) in levels {
                        ui.selectable_value(&mut app.state.log_level_min, level, name);
                    }
                });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Clear").clicked() {
                    app.state.logs.clear();
                }
            });
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("console_scroll")
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                let filter = app.state.log_filter.to_lowercase();
                let min_level = app.state.log_level_min;
                let logs = &app.state.logs;
                for entry in logs.iter().rev().take(800).rev() {
                    // Enum order: Info < Warn < Error < Debug < Trace; the
                    // selector picks the *minimum severity* shown.
                    if (entry.level as i32) > (min_level as i32) {
                        continue;
                    }
                    if !filter.is_empty()
                        && !entry.message.to_lowercase().contains(&filter)
                        && !entry.target.to_lowercase().contains(&filter)
                    {
                        continue;
                    }
                    let color = crate::panels::level_color(entry.level);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("⏱ {}", entry.time))
                                .monospace()
                                .size(11.0)
                                .color(theme::TEXT_DIM),
                        );
                        ui.label(
                            egui::RichText::new(format!("[{:>5}]", entry.level.tag()))
                                .monospace()
                                .size(11.0)
                                .color(color),
                        );
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&entry.message)
                                    .monospace()
                                    .size(11.0)
                                    .color(theme::TEXT),
                            )
                            .truncate(),
                        );
                    });
                }
            });
    });
}

// --- Output (runtime stdout + compiler raw) --------------------------------

fn draw_output(app: &mut BevyForgeApp, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical()
        .id_salt("output_scroll")
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            for line in app.state.compile_raw.iter().rev().take(1000).rev() {
                ui.label(
                    egui::RichText::new(line)
                        .monospace()
                        .size(11.0)
                        .color(theme::TEXT),
                );
            }
        });
}

// --- Rust Compiler panel ----------------------------------------------------

fn draw_compiler(app: &mut BevyForgeApp, ui: &mut egui::Ui, rect: egui::Rect) {
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.vertical(|ui| {
            // Header row.
            ui.horizontal(|ui| {
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(ui.available_rect_before_wrap().min, Vec2::new(ui.available_width(), 1.0)),
                    0.0,
                    theme::BG_HEADER,
                );
                ui.label(egui::RichText::new("⬢ Rust Compiler").strong().size(12.0));
                let err_badge = egui::RichText::new(format!("⛔ {}", app.state.compile_errors))
                    .color(if app.state.compile_errors > 0 { theme::RED } else { theme::TEXT_DIM })
                    .size(11.5);
                ui.label(err_badge);
                let warn_badge = egui::RichText::new(format!("⚠ {}", app.state.compile_warnings))
                    .color(if app.state.compile_warnings > 0 { theme::YELLOW } else { theme::TEXT_DIM })
                    .size(11.5);
                ui.label(warn_badge);
                if app.state.compile_running {
                    ui.label(egui::RichText::new("running…").small().color(theme::ACCENT));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Check").clicked() {
                        app.check_requested = true;
                    }
                    if ui.small_button("Clear").clicked() {
                        app.state.diagnostics.clear();
                        app.state.compile_errors = 0;
                        app.state.compile_warnings = 0;
                        app.state.compile_raw.clear();
                        app.state.selected_diagnostic = None;
                    }
                });
            });
            ui.separator();

            // Selected diagnostic snippet.
            if let Some(idx) = app.state.selected_diagnostic {
                if let Some(diag) = app.state.diagnostics.get(idx) {
                    egui::CollapsingHeader::new(egui::RichText::new(format!(
                        "{} {} — {}:{}",
                        if diag.level == forge_editor_core::DiagnosticLevel::Error { "⛔" } else { "⚠" },
                        diag.message,
                        diag.file,
                        diag.line
                    ))
                    .monospace()
                    .size(11.0))
                    .default_open(true)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("diag_snippet")
                            .max_height(120.0)
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new(&diag.rendered)
                                        .monospace()
                                        .size(10.5)
                                        .color(theme::TEXT),
                                );
                            });
                    });
                }
            }

            // Diagnostic list.
            egui::ScrollArea::vertical()
                .id_salt("diag_list")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if app.state.diagnostics.is_empty() {
                        ui.label(
                            egui::RichText::new(if app.state.compile_running {
                                "Compiling…"
                            } else {
                                "No diagnostics — run Check to validate scripts"
                            })
                            .small()
                            .color(theme::TEXT_DIM),
                        );
                    }
                    let diags = app.state.diagnostics.clone();
                    for (i, diag) in diags.iter().enumerate() {
                        let (icon, color) = match diag.level {
                            forge_editor_core::DiagnosticLevel::Error => ("⛔", theme::RED),
                            forge_editor_core::DiagnosticLevel::Warning => ("⚠", theme::YELLOW),
                            forge_editor_core::DiagnosticLevel::Note => ("ℹ", theme::ACCENT),
                            forge_editor_core::DiagnosticLevel::Help => ("?", theme::TEXT_DIM),
                        };
                        let label = egui::RichText::new(format!(
                            "{icon} {} — {}:{}",
                            diag.message, diag.file, diag.line
                        ))
                        .monospace()
                        .size(11.0)
                        .color(color);
                        if ui.selectable_label(false, label).clicked() {
                            app.state.selected_diagnostic = Some(i);
                            // Open the file in the script editor when it is a
                            // workspace-relative path.
                            open_diagnostic_file(app, diag);
                        }
                    }
                });
        });
    });
}

fn open_diagnostic_file(app: &mut BevyForgeApp, diag: &forge_editor_core::Diagnostic) {
    let file = &diag.file;
    // cargo reports paths like "crates/forge_scripts/src/lib.rs" relative to
    // the workspace manifest, absolute paths, or "scripts/…" for edited files.
    let candidates = [
        std::path::PathBuf::from(file),
        std::env::current_exe()
            .ok()
            .and_then(|p| p.ancestors().nth(3).map(|d| d.join(file)))
            .unwrap_or_default(),
    ];
    for candidate in candidates {
        if candidate.exists() {
            let path = candidate.to_string_lossy().to_string();
            app.open_script(&path);
            if let Some(idx) = app.state.active_script {
                if let Some(doc) = app.state.scripts.get_mut(idx) {
                    doc.error_line = Some((diag.line, diag.message.clone()));
                }
            }
            return;
        }
    }
}

// --- Image preview popup -----------------------------------------------------

fn draw_image_preview_popup(app: &mut BevyForgeApp, ctx: &Context) {
    let Some(path) = app.state.preview_popup.clone() else { return };
    let Some(texture) = app.image_previews.get(&path).cloned() else {
        app.state.preview_popup = None;
        return;
    };
    let mut open = true;
    egui::Window::new(format!("🖼 {}", std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()))
        .open(&mut open)
        .default_width(420.0)
        .show(ctx, |ui| {
            let size = texture.size_vec2();
            let avail = ui.available_size();
            let scale = (avail.x / size.x).min(avail.y / size.y).min(1.0);
            let size = size * scale;
            ui.image(egui::load::SizedTexture::new(texture.id(), size));
        });
    if !open {
        app.state.preview_popup = None;
    }
    let _ = Context::default;
}
