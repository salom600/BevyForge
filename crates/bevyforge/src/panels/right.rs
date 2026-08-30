//! Right dock: the Inspector (reflection-equivalent typed component editing)
//! and the Environment & Lighting panel.

use egui::Vec2;
use forge_ipc::{
    ComponentData, ComponentField, ComponentKind, EditorToRuntime, FieldValue, MeshPrimitive,
};

use crate::app::BevyForgeApp;
use crate::theme;

pub fn inspector_panel(app: &mut BevyForgeApp, ui: &mut egui::Ui) {
    egui::Panel::right("inspector_panel")
        .default_size(340.0)
        .size_range(260.0..=520.0)
        .resizable(true)
        .frame(egui::Frame::new().fill(theme::BG_PANEL).stroke(egui::Stroke::new(1.0, theme::BORDER)))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                theme::panel_header(ui, "Inspector", |ui| {
                    if crate::panels::tool_button(ui, "⤢", "Frame selection (F)") {
                        if let Some(sel) = app.state.selected {
                            let _ = sel;
                        }
                    }
                });
                ui.separator();

                if app.state.selected.is_none() {
                    ui.centered_and_justified(|ui| {
                        ui.add_space(24.0);
                        ui.label(
                            egui::RichText::new("No entity selected")
                                .color(theme::TEXT_DIM),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(
                                "Click an entity in the viewport or hierarchy.\nCreate objects from the GameObject menu.",
                            )
                            .small()
                            .color(theme::TEXT_DIM),
                        );
                    });
                    return;
                }

                let selected_bits = app.state.selected.unwrap();

                // Entity header: name + Active checkbox.
                ui.horizontal(|ui| {
                    let mut name = app.state.selected_name.clone();
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut name)
                            .desired_width(ui.available_width() - 110.0)
                            .font(egui::TextStyle::Heading),
                    );
                    if resp.lost_focus() && name != app.state.selected_name && !name.is_empty() {
                        app.cmd(EditorToRuntime::RenameEntity {
                            entity: selected_bits,
                            name: name.clone(),
                        });
                        app.state.selected_name = name;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Visibility acts as the design's "Active" checkbox.
                        let visible = app
                            .state
                            .components
                            .iter()
                            .find(|c| c.kind == ComponentKind::Visibility)
                            .and_then(|c| c.rows.first())
                            .map(|(_, r)| matches!(r.value, FieldValue::Bool(true)))
                            .unwrap_or(true);
                        let mut active = visible;
                        if ui.checkbox(&mut active, "Active").changed() {
                            app.cmd(EditorToRuntime::SetField {
                                entity: selected_bits,
                                component: ComponentKind::Visibility,
                                field: ComponentField::EntityVisible,
                                value: FieldValue::Bool(active),
                            });
                        }
                    });
                });
                ui.separator();

                // Entity meta rows.
                egui::Grid::new("entity_meta").num_columns(2).show(ui, |ui| {
                    ui.label(egui::RichText::new("Entity ID").small().color(theme::TEXT_DIM));
                    ui.label(
                        egui::RichText::new(format!("0x{:04X}_{:04X}", (selected_bits >> 16) as u16, selected_bits as u16))
                            .monospace()
                            .small(),
                    );
                    ui.end_row();
                    let parent = app
                        .state
                        .hierarchy
                        .iter()
                        .find(|n| n.id == selected_bits)
                        .map(|n| if n.depth == 0 { "World".to_string() } else { format!("depth {}", n.depth) })
                        .unwrap_or_else(|| "—".into());
                    ui.label(egui::RichText::new("Location").small().color(theme::TEXT_DIM));
                    ui.label(egui::RichText::new(parent).small());
                    ui.end_row();
                });
                ui.separator();

                // Components.
                egui::ScrollArea::vertical()
                    .id_salt("inspector_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("COMPONENTS").small().strong().color(theme::TEXT_DIM));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                egui::ComboBox::from_id_salt("add_component")
                                    .selected_text(egui::RichText::new("＋ Add Component").color(theme::ACCENT))
                                    .show_ui(ui, |ui| {
                                        for kind in ComponentKind::ADDABLE {
                                            if ui.selectable_label(true, kind.label()).clicked() {
                                                app.cmd(EditorToRuntime::AddComponent {
                                                    entity: selected_bits,
                                                    component: *kind,
                                                });
                                            }
                                        }
                                    });
                            });
                        });
                        ui.separator();

                        let components = app.state.components.clone();
                        for comp in &components {
                            draw_component(app, ui, selected_bits, comp);
                        }
                    });
            });
        });
}

fn draw_component(app: &mut BevyForgeApp, ui: &mut egui::Ui, entity: u64, comp: &ComponentData) {
    let header_id = egui::Id::new(("component", entity, comp.kind as u32));
    egui::containers::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), header_id, true)
        .show_header(ui, |ui| {
            let (glyph, color) = component_glyph(comp.kind);
            ui.label(egui::RichText::new(glyph).color(color));
            ui.label(egui::RichText::new(comp.kind.label()).strong().size(12.5));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("⋯").on_hover_text("Component actions").clicked() {}
                ui.menu_button("⋯", |ui| {
                    if ui.button("Remove Component").clicked() {
                        app.cmd(EditorToRuntime::RemoveComponent {
                            entity,
                            component: comp.kind,
                        });
                        ui.close();
                    }
                });
            });
        })
        .body(|ui| {
            egui::Grid::new(egui::Id::new(("rows", entity, comp.kind as u32)))
                .num_columns(2)
                .spacing([8.0, 3.0])
                .show(ui, |ui| {
                    for (field, row) in &comp.rows {
                        ui.label(
                            egui::RichText::new(&row.label)
                                .size(12.0)
                                .color(theme::TEXT_DIM),
                        );
                        draw_value_editor(app, ui, entity, comp.kind, *field, &row.value);
                        ui.end_row();
                    }
                });
            ui.add_space(4.0);
        });
}

fn component_glyph(kind: ComponentKind) -> (&'static str, egui::Color32) {
    use ComponentKind as C;
    match kind {
        C::Transform => ("◈", theme::ACCENT),
        C::Visibility => ("◉", theme::TEXT_DIM),
        C::Mesh => ("⬢", theme::TEXT_DIM),
        C::Material => ("◐", theme::ORANGE),
        C::Camera => ("◧", theme::ACCENT),
        C::DirectionalLight | C::PointLight | C::SpotLight => ("✳", theme::YELLOW),
        C::Rotator | C::Orbiter | C::LinearMover | C::PingPongMover => ("⚙", theme::GREEN),
        C::Player | C::CharacterController => ("☉", theme::ORANGE),
        C::Health => ("♥", theme::RED),
        C::Inventory => ("▦", theme::ACCENT),
    }
}

fn draw_value_editor(
    app: &mut BevyForgeApp,
    ui: &mut egui::Ui,
    entity: u64,
    component: ComponentKind,
    field: ComponentField,
    value: &FieldValue,
) {
    let value = value.clone();
    let push = |app: &mut BevyForgeApp, old: FieldValue, new: FieldValue| {
        let undo = EditorToRuntime::SetField { entity, component, field, value: old };
        let redo = EditorToRuntime::SetField { entity, component, field, value: new };
        let label = format!("Edit {}", component.label());
        app.apply_with_undo(&label, vec![undo], vec![redo]);
    };

    match value {
        FieldValue::F32(v) => {
            let mut v = v;
            let resp = ui.add_sized(
                Vec2::new(90.0, 18.0),
                egui::DragValue::new(&mut v).speed(0.05),
            );
            if resp.changed() {
                push(app, value, FieldValue::F32(v));
            }
        }
        FieldValue::U32(v) => {
            let mut v = v;
            let resp = ui.add_sized(
                Vec2::new(90.0, 18.0),
                egui::DragValue::new(&mut v).speed(1.0),
            );
            if resp.changed() {
                push(app, value, FieldValue::U32(v));
            }
        }
        FieldValue::Bool(v) => {
            let mut v = v;
            if ui.checkbox(&mut v, "").changed() {
                push(app, value, FieldValue::Bool(v));
            }
        }
        FieldValue::Vec3(v) => {
            let mut v = v;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("X").small().color(theme::AXIS_X));
                let rx = ui.add_sized(Vec2::new(52.0, 18.0), egui::DragValue::new(&mut v[0]).speed(0.05));
                ui.label(egui::RichText::new("Y").small().color(theme::AXIS_Y));
                let ry = ui.add_sized(Vec2::new(52.0, 18.0), egui::DragValue::new(&mut v[1]).speed(0.05));
                ui.label(egui::RichText::new("Z").small().color(theme::AXIS_Z));
                let rz = ui.add_sized(Vec2::new(52.0, 18.0), egui::DragValue::new(&mut v[2]).speed(0.05));
                if rx.changed() || ry.changed() || rz.changed() {
                    push(app, value, FieldValue::Vec3(v));
                }
            });
        }
        FieldValue::Rgba(c) => {
            ui.horizontal(|ui| {
                let mut color = egui::Color32::from_rgba_unmultiplied(
                    (c[0].clamp(0.0, 1.0) * 255.0) as u8,
                    (c[1].clamp(0.0, 1.0) * 255.0) as u8,
                    (c[2].clamp(0.0, 1.0) * 255.0) as u8,
                    (c[3].clamp(0.0, 1.0) * 255.0) as u8,
                );
                let is_hdr = c.iter().any(|x| *x > 1.0);
                if is_hdr {
                    // HDR (emissive): numeric RGB editors, alpha fixed at 1.
                    let mut rgb = [c[0], c[1], c[2]];
                    let mut changed = false;
                    ui.label(egui::RichText::new("HDR").small().color(theme::ORANGE));
                    for (i, axis) in ["R", "G", "B"].iter().enumerate() {
                        ui.label(egui::RichText::new(*axis).small().color(theme::TEXT_DIM));
                        let resp = ui.add_sized(Vec2::new(40.0, 18.0), egui::DragValue::new(&mut rgb[i]).speed(0.02));
                        changed |= resp.changed();
                    }
                    if changed {
                        push(app, value, FieldValue::Rgba([rgb[0], rgb[1], rgb[2], 1.0]));
                    }
                } else {
                    if ui.color_edit_button_srgba(&mut color).changed() {
                        let [r, g, b, a] = color.to_array();
                        push(
                            app,
                            value,
                            FieldValue::Rgba([
                                r as f32 / 255.0,
                                g as f32 / 255.0,
                                b as f32 / 255.0,
                                a as f32 / 255.0,
                            ]),
                        );
                    }
                }
            });
        }
        FieldValue::Mesh(prim) => {
            egui::ComboBox::from_id_salt("mesh_primitive")
                .selected_text(prim.label())
                .show_ui(ui, |ui| {
                    for p in MeshPrimitive::ALL {
                        if ui.selectable_label(prim == *p, p.label()).clicked() {
                            push(app, value.clone(), FieldValue::Mesh(*p));
                        }
                    }
                });
        }
        FieldValue::Str(ref s) => {
            let original = s.clone();
            let mut edited = original.clone();
            let resp = ui.add(egui::TextEdit::singleline(&mut edited).desired_width(140.0));
            if resp.lost_focus() && edited != original {
                push(app, value, FieldValue::Str(edited));
            }
        }
    }
}


// ---------------------------------------------------------------------------
// Environment & Lighting panel
// ---------------------------------------------------------------------------

pub fn environment_panel(app: &mut BevyForgeApp, ui: &mut egui::Ui) {
    egui::Panel::right("environment_panel")
        .default_size(300.0)
        .resizable(true)
        .frame(egui::Frame::new().fill(theme::BG_PANEL).stroke(egui::Stroke::new(1.0, theme::BORDER)))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                theme::panel_header(ui, "Environment && Lighting", |ui| {
                    if crate::panels::tool_button(ui, "⟳", "Request settings from runtime") {
                        app.cmd(EditorToRuntime::RequestFullState);
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("env_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let env = app.state.env.clone();
                        let mut next = env.clone();
                        let mut changed = false;

                        ui.label(section("Ambient"));
                        egui::Grid::new("env_ambient").num_columns(2).show(ui, |ui| {
                            ui.label(small("Color"));
                            let mut c = to_egui(next.ambient_color);
                            if ui.color_edit_button_srgba(&mut c).changed() {
                                next.ambient_color = from_egui(c);
                                changed = true;
                            }
                            ui.end_row();
                            ui.label(small("Brightness (cd/m²)"));
                            if drag(ui, &mut next.ambient_brightness, 0.5, 0.0..=500.0) {
                                changed = true;
                            }
                            ui.end_row();
                        });

                        ui.separator();
                        ui.label(section("Sun (first Directional Light)"));
                        egui::Grid::new("env_sun").num_columns(2).show(ui, |ui| {
                            ui.label(small("Illuminance (lux)"));
                            if drag(ui, &mut next.sun_illuminance, 100.0, 0.0..=150_000.0) {
                                changed = true;
                            }
                            ui.end_row();
                            ui.label(small("Elevation (deg)"));
                            if drag(ui, &mut next.sun_elevation_deg, 0.5, -89.0..=89.0) {
                                changed = true;
                            }
                            ui.end_row();
                            ui.label(small("Azimuth (deg)"));
                            if drag(ui, &mut next.sun_azimuth_deg, 0.5, -360.0..=360.0) {
                                changed = true;
                            }
                            ui.end_row();
                            ui.label(small("Shadow maps"));
                            if ui.checkbox(&mut next.sun_shadows, "").changed() {
                                changed = true;
                            }
                            ui.end_row();
                        });

                        ui.separator();
                        ui.label(section("Camera / Tonemapping"));
                        egui::Grid::new("env_camera").num_columns(2).show(ui, |ui| {
                            ui.label(small("Tonemapping"));
                            let mut tm = next.tonemapping;
                            let mut tm_changed = false;
                            egui::ComboBox::from_id_salt("tonemap")
                                .selected_text(tm.label())
                                .show_ui(ui, |ui| {
                                    for k in forge_ipc::TonemappingKind::ALL {
                                        if ui.selectable_label(tm == *k, k.label()).clicked() {
                                            tm = *k;
                                            tm_changed = true;
                                        }
                                    }
                                });
                            if tm_changed {
                                next.tonemapping = tm;
                                changed = true;
                            }
                            ui.end_row();
                            ui.label(small("Exposure EV100"));
                            if drag(ui, &mut next.exposure_ev100, 0.05, -10.0..=10.0) {
                                changed = true;
                            }
                            ui.end_row();
                        });

                        ui.separator();
                        ui.label(section("Fog"));
                        if ui.checkbox(&mut next.fog_enabled, "Enabled").changed() {
                            changed = true;
                        }
                        egui::Grid::new("env_fog").num_columns(2).show(ui, |ui| {
                            ui.label(small("Color"));
                            let mut c = to_egui(next.fog_color);
                            if ui.color_edit_button_srgba(&mut c).changed() {
                                next.fog_color = from_egui(c);
                                changed = true;
                            }
                            ui.end_row();
                            ui.label(small("Start"));
                            if drag(ui, &mut next.fog_start, 0.5, 0.0..=1000.0) {
                                changed = true;
                            }
                            ui.end_row();
                            ui.label(small("End"));
                            if drag(ui, &mut next.fog_end, 0.5, 1.0..=2000.0) {
                                changed = true;
                            }
                            ui.end_row();
                        });

                        ui.separator();
                        ui.label(section("Viewport"));
                        egui::Grid::new("env_viewport").num_columns(2).show(ui, |ui| {
                            ui.label(small("Grid"));
                            if ui.checkbox(&mut next.show_grid, "").changed() {
                                app.state.show_grid = next.show_grid;
                                changed = true;
                            }
                            ui.end_row();
                            ui.label(small("Selection outline"));
                            if ui.checkbox(&mut next.show_selection_outline, "").changed() {
                                app.state.show_outline = next.show_selection_outline;
                                changed = true;
                            }
                            ui.end_row();
                            ui.label(small("Clear color"));
                            let mut c = to_egui(next.clear_color);
                            if ui.color_edit_button_srgba(&mut c).changed() {
                                next.clear_color = from_egui(c);
                                changed = true;
                            }
                            ui.end_row();
                        });

                        if changed {
                            app.state.env = next.clone();
                            app.cmd(EditorToRuntime::SetEnvironment(next));
                        }
                    });
            });
        });
}

fn to_egui(c: [f32; 4]) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(
        (c[0].clamp(0.0, 1.0) * 255.0) as u8,
        (c[1].clamp(0.0, 1.0) * 255.0) as u8,
        (c[2].clamp(0.0, 1.0) * 255.0) as u8,
        (c[3].clamp(0.0, 1.0) * 255.0) as u8,
    )
}

fn from_egui(c: egui::Color32) -> [f32; 4] {
    let [r, g, b, a] = c.to_array();
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0]
}

fn section(t: &str) -> egui::RichText {
    egui::RichText::new(t.to_uppercase()).small().strong().color(theme::TEXT_DIM)
}

fn small(t: &str) -> egui::RichText {
    egui::RichText::new(t.to_string()).size(12.0).color(theme::TEXT_DIM)
}

fn drag(ui: &mut egui::Ui, v: &mut f32, speed: f64, range: std::ops::RangeInclusive<f64>) -> bool {
    ui.add(egui::DragValue::new(v).speed(speed).range(range)).changed()
}
