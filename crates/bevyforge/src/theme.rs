//! BevyForge dark theme — matches the design blueprint (500.png):
//! deep navy-charcoal surfaces, blue accents, Bevy-orange highlights,
//! RGB axis colours for transform fields.

use egui::{Color32, Style, Vec2};


pub const BG_APP: Color32 = Color32::from_rgb(0x0a, 0x0e, 0x14);
pub const BG_PANEL: Color32 = Color32::from_rgb(0x0e, 0x13, 0x1b);
pub const BG_HEADER: Color32 = Color32::from_rgb(0x12, 0x18, 0x22);
pub const BG_WIDGET: Color32 = Color32::from_rgb(0x16, 0x1e, 0x2a);
pub const BG_WIDGET_HOVER: Color32 = Color32::from_rgb(0x1c, 0x26, 0x35);
pub const BG_ACTIVE: Color32 = Color32::from_rgb(0x1f, 0x2d, 0x42);
pub const BORDER: Color32 = Color32::from_rgb(0x1d, 0x27, 0x33);
pub const TEXT: Color32 = Color32::from_rgb(0xc9, 0xd4, 0xe0);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x6b, 0x7a, 0x8d);
pub const ACCENT: Color32 = Color32::from_rgb(0x2f, 0x81, 0xf7);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x1c, 0x3a, 0x5e);
pub const ORANGE: Color32 = Color32::from_rgb(0xff, 0x8a, 0x3d);
pub const GREEN: Color32 = Color32::from_rgb(0x3f, 0xb9, 0x50);
pub const RED: Color32 = Color32::from_rgb(0xf8, 0x51, 0x49);
pub const YELLOW: Color32 = Color32::from_rgb(0xd2, 0x99, 0x22);
pub const AXIS_X: Color32 = Color32::from_rgb(0xf8, 0x51, 0x49);
pub const AXIS_Y: Color32 = Color32::from_rgb(0x3f, 0xb9, 0x50);
pub const AXIS_Z: Color32 = Color32::from_rgb(0x2f, 0x81, 0xf7);

/// Apply the BevyForge theme to a context.
pub fn install(ctx: &egui::Context) {
    let mut style = Style::default();
    style.visuals = egui::Visuals::dark();
    let v = &mut style.visuals;

    v.window_fill = BG_PANEL;
    v.panel_fill = BG_PANEL;
    v.extreme_bg_color = BG_APP;          // text edits, code editor bg
    v.faint_bg_color = BG_HEADER;         // alternating rows
    v.window_stroke = egui::Stroke::new(1.0, BORDER);
    v.widgets.noninteractive.bg_fill = BG_WIDGET;
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT_DIM);
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    v.widgets.inactive.bg_fill = BG_WIDGET;
    v.widgets.inactive.weak_bg_fill = BG_WIDGET;
    v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT);
    v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    v.widgets.hovered.bg_fill = BG_WIDGET_HOVER;
    v.widgets.hovered.weak_bg_fill = BG_WIDGET_HOVER;
    v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, TEXT);
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, ACCENT_DIM);
    v.widgets.active.bg_fill = BG_ACTIVE;
    v.widgets.active.weak_bg_fill = BG_ACTIVE;
    v.widgets.active.fg_stroke = egui::Stroke::new(1.0, Color32::WHITE);
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = egui::Stroke::new(1.0, ACCENT);
    v.override_text_color = Some(TEXT);
    v.button_frame = true;
    v.collapsing_header_frame = false;
    v.window_corner_radius = egui::CornerRadius::same(4);
    v.menu_corner_radius = egui::CornerRadius::same(4);
    v.resize_corner_size = 8.0;

    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::proportional(13.0),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::proportional(12.5),
    );
    style.text_styles.insert(
        egui::TextStyle::Small,
        egui::FontId::proportional(10.5),
    );
    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::proportional(16.0),
    );
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        egui::FontId::monospace(12.0),
    );
    style.spacing.item_spacing = Vec2::new(6.0, 4.0);
    style.spacing.button_padding = Vec2::new(7.0, 3.0);
    style.spacing.menu_spacing = 2.0;
    style.spacing.icon_width = 16.0;
    style.spacing.icon_width_inner = 14.0;
    style.spacing.indent = 14.0;
    style.spacing.menu_margin = egui::Margin::same(4);

    ctx.all_styles_mut(|s| *s = style.clone());
}

/// Panel header with a small vector icon before the title.
pub fn panel_header_iconed(
    ui: &mut egui::Ui,
    icon: Option<crate::icons::Icon>,
    title: &str,
    actions: impl FnOnce(&mut egui::Ui),
) {
    let header_rect = egui::Rect::from_min_size(
        ui.available_rect_before_wrap().min,
        Vec2::new(ui.available_width(), 24.0),
    );
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(header_rect),
        |ui| {
            ui.set_min_size(Vec2::new(ui.available_width(), 24.0));
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.visuals_mut().panel_fill = BG_HEADER;
                let rect = ui.available_rect_before_wrap();
                ui.painter().rect_filled(rect, 0, BG_HEADER);
                ui.add_space(6.0);
                if let Some(icon) = icon {
                    let icon_rect = ui.allocate_exact_size(Vec2::splat(13.0), egui::Sense::hover()).0;
                    crate::icons::paint(ui.painter(), icon, icon_rect, ACCENT);
                    ui.add_space(2.0);
                }
                ui.label(
                    egui::RichText::new(title.to_uppercase())
                        .small()
                        .color(TEXT_DIM)
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), actions);
            });
        },
    );
}

/// Small square toolbar button with tooltip; returns clicked.
pub fn tool_button(ui: &mut egui::Ui, glyph: &str, tooltip: &str) -> bool {
    ui.add_sized(Vec2::new(20.0, 18.0), egui::Button::new(egui::RichText::new(glyph).size(12.0)))
        .on_hover_text(tooltip)
        .clicked()
}
