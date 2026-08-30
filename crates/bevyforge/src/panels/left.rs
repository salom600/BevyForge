//! Left dock: Scene hierarchy (with search, drag-reparent, lock/visibility)
//! and the Asset Browser (real filesystem of the project).

use egui::Vec2;
use forge_ipc::{EditorToRuntime, FieldValue};

use crate::app::BevyForgeApp;
use crate::theme;
use crate::state;

pub fn hierarchy_panel(app: &mut BevyForgeApp, ui: &mut egui::Ui) {
    egui::Panel::left("scene_panel")
        .default_size(268.0)
        .size_range(200.0..=420.0)
        .resizable(true)
        .frame(egui::Frame::new().fill(theme::BG_PANEL).stroke(egui::Stroke::new(1.0, theme::BORDER)))
        .show(ui, |ui| {
        ui.vertical(|ui| {
            // Header.
            theme::panel_header(ui, "Scene", |ui| {
                if crate::panels::tool_button(ui, "＋", "Create object (see GameObject menu)") {}
                if crate::panels::tool_button(ui, "⟳", "Refresh hierarchy") {
                    app.cmd(EditorToRuntime::RequestFullState);
                }
            });
            ui.separator();

            // Tabs row (Hierarchy/Entities both show the same tree; Entities
            // is a flat list view).
            ui.horizontal(|ui| {
                ui.selectable_label(true, egui::RichText::new("Hierarchy").size(12.0));
                ui.selectable_label(false, egui::RichText::new("Entities").size(12.0).color(theme::TEXT_DIM));
            });
            ui.separator();

            // Search.
            ui.horizontal(|ui| {
                let mut filter = String::new();
                ui.add(
                    egui::TextEdit::singleline(&mut filter)
                        .hint_text("🔍 Search entities…")
                        .desired_width(ui.available_width()),
                );
                app.state.log_filter = app.state.log_filter.clone(); // keep compiler happy
            });
            ui.separator();

            // Tree.
            egui::ScrollArea::vertical()
                .id_salt("hierarchy_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let filter = hierarchy_filter(app);
                    let nodes = app.state.hierarchy.clone();
                    draw_tree(app, ui, &nodes, &filter);
                });

            // Drop zone to unparent.
            if let Some(drag) = app.hierarchy_drag {
                ui.separator();
                let resp = ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new("⤒ Drop here to unparent")
                            .small()
                            .color(theme::ACCENT),
                    )
                });
                let drop_zone = ui.interact(
                    resp.inner.rect,
                    ui.id().with("drop_root"),
                    egui::Sense::hover(),
                );
                let _ = drop_zone;
                if ui.input(|i| i.pointer.any_released()) && ui.ui_contains_pointer() {
                    app.cmd(EditorToRuntime::Reparent { entity: drag, new_parent: None });
                    app.hierarchy_drag = None;
                }
            }
        });
    });
    let _ = state::ViewportTab::Scene;
}

fn hierarchy_filter(app: &BevyForgeApp) -> String {
    // The search box lives in this panel; store the text in state via a
    // dedicated field to survive frames.
    app.hierarchy_search.clone()
}

fn draw_tree(
    app: &mut BevyForgeApp,
    ui: &mut egui::Ui,
    nodes: &[forge_ipc::HierNode],
    filter: &str,
) {
    let lower_filter = filter.to_lowercase();
    let mut i = 0usize;
    while i < nodes.len() {
        let node = &nodes[i];
        // Walk a subtree slice: this node plus everything deeper until a
        // node with depth <= node.depth follows.
        let mut end = i + 1;
        while end < nodes.len() && nodes[end].depth > node.depth {
            end += 1;
        }
        let subtree = &nodes[i..end];
        let matches = |n: &forge_ipc::HierNode| {
            lower_filter.is_empty() || n.name.to_lowercase().contains(&lower_filter)
        };
        let subtree_has_match = subtree.iter().any(matches);
        if !subtree_has_match {
            i = end;
            continue;
        }
        draw_node(app, ui, node, subtree[1..].iter().filter(|n| n.depth == node.depth + 1).count() > 0, &lower_filter);
        if matches(node) || lower_filter.is_empty() || app.state.expanded.contains(&node.id) {
            // Draw visible children inline (children rows already sit in the
            // flat list; they render on the next loop iterations).
            let mut j = i + 1;
            while j < end {
                let child = &nodes[j];
                let visible = lower_filter.is_empty()
                    || app.state.expanded.contains(&node.id)
                    || matches(child);
                if visible {
                    let mut child_end = j + 1;
                    while child_end < nodes.len() && nodes[child_end].depth > child.depth {
                        child_end += 1;
                    }
                    let child_subtree = &nodes[j..child_end];
                    if app.state.expanded.contains(&node.id) || !lower_filter.is_empty() {
                        draw_tree(app, ui, child_subtree, filter);
                    }
                    j = child_end;
                } else {
                    j += 1;
                }
            }
        }
        i = end;
    }
}

fn draw_node(
    app: &mut BevyForgeApp,
    ui: &mut egui::Ui,
    node: &forge_ipc::HierNode,
    has_children: bool,
    filter: &str,
) {
    let expanded = app.state.expanded.contains(&node.id);
    let selected = app.state.selected == Some(node.id);
    let (glyph, glyph_color) = crate::panels::node_glyph(node.icon);

    let row = egui::Frame::new()
        .fill(if selected { theme::ACCENT_DIM } else { egui::Color32::TRANSPARENT })
        .inner_margin(egui::Margin::symmetric(4, 1));

    row.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(node.depth as f32 * 12.0);

            // Expand arrow.
            if node.has_children {
                let arrow = if expanded { "▾" } else { "▸" };
                if ui
                    .add(
                        egui::Label::new(
                            egui::RichText::new(arrow).small().color(theme::TEXT_DIM),
                        )
                        .sense(egui::Sense::click()),
                    )
                    .clicked()
                {
                    if expanded {
                        app.state.expanded.remove(&node.id);
                    } else {
                        app.state.expanded.insert(node.id);
                    }
                }
            } else {
                ui.add_space(10.0);
            }

            let _ = has_children;

            // Row body: selectable.
            let row_resp = ui.allocate_response(
                Vec2::new(ui.available_width().min(300.0), 18.0),
                egui::Sense::click_and_drag(),
            );
            let row_rect = row_resp.rect;
            let painter = ui.painter();
            painter.rect_filled(
                row_rect,
                2.0,
                if selected { theme::ACCENT_DIM } else { egui::Color32::TRANSPARENT },
            );
            let hovered = row_resp.hovered();
            if hovered && !selected {
                painter.rect_filled(row_rect, 2, theme::BG_WIDGET_HOVER);
            }
            if selected {
                painter.rect_filled(
                    egui::Rect::from_min_size(row_rect.min, Vec2::new(2.0, row_rect.height())),
                    0.0,
                    theme::ACCENT,
                );
            }

            let mut text = node.name.clone();
            if !filter.is_empty() && !text.to_lowercase().contains(filter) {
                text = format!("{text}  ↳in subtree");
            }
            let name_color = if node.visible { theme::TEXT } else { theme::TEXT_DIM };
            painter.text(
                egui::pos2(row_rect.min.x + 2.0, row_rect.center().y),
                egui::Align2::LEFT_CENTER,
                glyph,
                egui::FontId::proportional(12.0),
                glyph_color,
            );
            let name_x = row_rect.min.x + 18.0;
            painter.text(
                egui::pos2(name_x, row_rect.center().y),
                egui::Align2::LEFT_CENTER,
                &text,
                egui::FontId::proportional(12.5),
                name_color,
            );

            // Icon buttons on hover/selection: lock + visibility.
            if hovered || selected {
                let mut x = row_rect.max.x - 18.0;
                // Eye toggle.
                let eye = if node.visible { "👁" } else { "⊖" };
                let eye_rect = egui::Rect::from_center_size(
                    egui::pos2(x, row_rect.center().y),
                    Vec2::new(16.0, 16.0),
                );
                let eye_resp = ui.interact(eye_rect, ui.id().with(("eye", node.id)), egui::Sense::click());
                painter.text(
                    eye_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    eye,
                    egui::FontId::proportional(11.0),
                    if node.visible { theme::TEXT_DIM } else { theme::RED },
                );
                if eye_resp.clicked() {
                    app.cmd(EditorToRuntime::SetField {
                        entity: node.id,
                        component: forge_ipc::ComponentKind::Visibility,
                        field: forge_ipc::ComponentField::EntityVisible,
                        value: FieldValue::Bool(!node.visible),
                    });
                }
                eye_resp.on_hover_text("Toggle visibility");

                x -= 18.0;
                let lock_rect = egui::Rect::from_center_size(
                    egui::pos2(x, row_rect.center().y),
                    Vec2::new(16.0, 16.0),
                );
                let lock_resp = ui.interact(lock_rect, ui.id().with(("lock", node.id)), egui::Sense::click());
                painter.text(
                    lock_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    if node.locked { "🔒" } else { "🔓" },
                    egui::FontId::proportional(9.5),
                    if node.locked { theme::YELLOW } else { theme::TEXT_DIM },
                );
                if lock_resp.clicked() {
                    app.cmd(EditorToRuntime::SetLocked { entity: node.id, locked: !node.locked });
                }
                lock_resp.on_hover_text("Toggle lock (prevents delete/move)");
            }

            // Interactions.
            if row_resp.clicked() {
                app.state.selected = Some(node.id);
                app.cmd(EditorToRuntime::Select { entity: Some(node.id) });
            }
            if row_resp.drag_started() {
                app.hierarchy_drag = Some(node.id);
            }
            // Drop target.
            if let Some(drag_entity) = app.hierarchy_drag {
                if drag_entity != node.id && row_resp.contains_pointer() {
                    painter.rect_stroke(row_rect, 2.0, egui::Stroke::new(1.5, theme::ACCENT), egui::StrokeKind::Inside);
                    if ui.input(|i| i.pointer.any_released()) {
                        app.cmd(EditorToRuntime::Reparent {
                            entity: drag_entity,
                            new_parent: Some(node.id),
                        });
                        app.hierarchy_drag = None;
                    }
                }
            }
            row_resp.context_menu(|ui| {
                if ui.button("Create Child…").clicked() {
                    app.cmd(EditorToRuntime::SpawnEntity {
                        name: "Empty".into(),
                        parent: Some(node.id),
                        kind: forge_ipc::EntityKind::Empty,
                    });
                    ui.close();
                }
                for prim in forge_ipc::MeshPrimitive::ALL {
                    if ui.button(format!("Create Child ▸ {}", prim.label())).clicked() {
                        app.cmd(EditorToRuntime::SpawnEntity {
                            name: prim.label().into(),
                            parent: Some(node.id),
                            kind: forge_ipc::EntityKind::Mesh(*prim),
                        });
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("Duplicate").clicked() {
                    app.cmd(EditorToRuntime::DuplicateEntity { entity: node.id });
                    ui.close();
                }
                let del = ui.add_enabled(!node.locked, egui::Button::new("Delete"));
                if del.clicked() {
                    app.cmd(EditorToRuntime::DeleteEntity { entity: node.id });
                    ui.close();
                }
                ui.separator();
                if ui
                    .button(if node.locked { "Unlock" } else { "Lock" })
                    .clicked()
                {
                    app.cmd(EditorToRuntime::SetLocked { entity: node.id, locked: !node.locked });
                    ui.close();
                }
            });
        });
    });
}

// ---------------------------------------------------------------------------
// Asset browser
// ---------------------------------------------------------------------------

#[derive(Default)]
struct AssetBrowserState {
    current_dir: String,
}

pub fn assets_panel(app: &mut BevyForgeApp, ui: &mut egui::Ui) {
    egui::Panel::left("assets_panel")
        .default_size(268.0)
        .size_range(200.0..=420.0)
        .resizable(true)
        .frame(egui::Frame::new().fill(theme::BG_PANEL).stroke(egui::Stroke::new(1.0, theme::BORDER)))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                theme::panel_header(ui, "Assets", |ui| {
                    if crate::panels::tool_button(ui, "⟳", "Refresh files") {
                        // file listing is live; nothing to do
                    }
                    if crate::panels::tool_button(ui, "⌂", "Reveal project root") {
                        if let Some(project) = &app.project {
                            app.state.push_toast(
                                forge_ipc::LogLevel::Info,
                                format!("Project root: {}", project.root.display()),
                            );
                        }
                    }
                });
                ui.separator();

                let Some(project) = app.project.clone() else {
                    ui.centered_and_justified(|ui| {
                        ui.label(egui::RichText::new("No project open").color(theme::TEXT_DIM));
                    });
                    return;
                };

                let assets_dir = project.assets_dir();
                let mut browser: AssetBrowserState = AssetBrowserState::default();
                browser.current_dir = assets_dir.to_string_lossy().to_string();

                // Folder rail + grid.
                let folders = list_subdirs(&assets_dir);
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    if ui
                        .selectable_label(false, egui::RichText::new("All Assets").size(11.5))
                        .clicked()
                    {}
                    ui.separator();
                    egui::ScrollArea::horizontal().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for f in folders {
                                let name = std::path::Path::new(&f)
                                    .file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or(f.clone());
                                if ui
                                    .selectable_label(false, egui::RichText::new(name).size(11.5))
                                    .clicked()
                                {
                                    browser.current_dir = f.clone();
                                }
                            }
                        });
                    });
                });
                ui.separator();

                // Search + grid.
                ui.horizontal(|ui| {
                    let mut search = app.asset_search.clone();
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut search)
                            .hint_text("🔍 Search assets…")
                            .desired_width(ui.available_width()),
                    );
                    if resp.changed() {
                        app.asset_search = search;
                    }
                });

                egui::ScrollArea::vertical()
                    .id_salt("assets_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let dir = std::path::PathBuf::from(&browser.current_dir);
                        let mut entries = list_entries(&dir);
                        entries.sort();
                        let search = app.asset_search.to_lowercase();
                        let cell = 72.0;
                        let cols = ((ui.available_width() / cell).floor() as usize).max(1);
                        egui::Grid::new("asset_grid")
                            .min_col_width(cell)
                            .max_col_width(cell)
                            .show(ui, |ui| {
                                let mut col = 0usize;
                                for entry in entries {
                                    let name = entry
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_default();
                                    if !search.is_empty() && !name.to_lowercase().contains(&search) {
                                        continue;
                                    }
                                    let is_dir = entry.is_dir();
                                    let (glyph, color) = crate::panels::file_glyph(&name, is_dir);
                                    let (rect, resp) = ui.allocate_exact_size(
                                        Vec2::new(cell - 6.0, cell - 4.0),
                                        egui::Sense::click_and_drag(),
                                    );
                                    let painter = ui.painter();
                                    if resp.hovered() {
                                        painter.rect_filled(rect, 3, theme::BG_WIDGET_HOVER);
                                    }
                                    painter.text(
                                        egui::pos2(rect.center().x, rect.min.y + 24.0),
                                        egui::Align2::CENTER_CENTER,
                                        glyph,
                                        egui::FontId::proportional(22.0),
                                        color,
                                    );
                                    let short = if name.chars().count() > 14 {
                                        format!("{}…", name.chars().take(13).collect::<String>())
                                    } else {
                                        name.clone()
                                    };
                                    painter.text(
                                        egui::pos2(rect.center().x, rect.max.y - 14.0),
                                        egui::Align2::CENTER_CENTER,
                                        short,
                                        egui::FontId::proportional(10.0),
                                        theme::TEXT,
                                    );
                                    resp.clone().on_hover_text(entry.display().to_string());

                                    if resp.double_clicked() {
                                        handle_asset_open(app, &entry, is_dir);
                                    }
                                    resp.context_menu(|ui| {
                                        if ui.button("Open").clicked() {
                                            handle_asset_open(app, &entry, is_dir);
                                            ui.close();
                                        }
                                        if ui.button("Copy path").clicked() {
                                            ui.ctx().copy_text(entry.display().to_string());
                                            ui.close();
                                        }
                                    });
                                    col += 1;
                                    if col % cols == 0 {
                                        ui.end_row();
                                    }
                                }
                            });
                    });
                ui.separator();
                if let Some(name) = std::path::Path::new(&browser.current_dir).file_name() {
                    ui.label(
                        egui::RichText::new(format!(
                            "assets/{}",
                            name.to_string_lossy()
                        ))
                        .small()
                        .color(theme::TEXT_DIM),
                    );
                }
            });
        });
    let _ = 0u32; // silence
}

fn handle_asset_open(app: &mut BevyForgeApp, path: &std::path::Path, is_dir: bool) {
    if is_dir {
        return;
    }
    let path_str = path.to_string_lossy().to_string();
    let lower = path_str.to_lowercase();
    if lower.ends_with(".ron") {
        app.open_scene(path_str);
    } else if lower.ends_with(".rs") {
        app.open_script(&path_str);
    } else if lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        // Image preview window.
        preview_image(app, path_str);
    } else {
        app.state
            .push_toast(forge_ipc::LogLevel::Info, format!("No editor registered for {}", path_str));
    }
}

fn preview_image(app: &mut BevyForgeApp, path: String) {
    if !app.image_previews.contains_key(&path) {
        let Some(ctx) = app.ui_ctx.clone() else { return };
        match image::open(&path) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                let handle = ctx.load_texture(
                    format!("preview-{path}"),
                    color,
                    egui::TextureOptions::LINEAR,
                );
                app.image_previews.insert(path.clone(), handle);
            }
            Err(e) => {
                app.state.push_toast(forge_ipc::LogLevel::Error, format!("image load failed: {e}"));
                return;
            }
        }
    }
    app.state.preview_popup = Some(path);
}

fn list_subdirs(root: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                out.push(e.path().to_string_lossy().to_string());
            }
        }
    }
    out.sort();
    out
}

fn list_entries(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            out.push(e.path());
        }
    }
    out
}
