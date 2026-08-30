//! Vector icon set painted with egui primitives — no icon fonts, no raster
//! assets, tintable at runtime and crisp at any DPI.
//!
//! Every icon is drawn in a 16x16 design space and scaled to the requested
//! rect. Strokes use round caps for a consistent, professional look that
//! matches the dark theme.

use egui::{Color32, Painter, Pos2, Rect, Stroke, Vec2};

/// The icon catalogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Icon {
    // --- transform modes ---
    Translate,
    Rotate,
    Scale,
    // --- viewport ---
    Grid,
    Outline,
    Frame,
    Camera,
    Perspective,
    // --- transport ---
    Play,
    Pause,
    Stop,
    SkipBack,
    SkipForward,
    Rewind,
    FastForward,
    Record,
    Keyframe,
    // --- edit ---
    Undo,
    Redo,
    Trash,
    Duplicate,
    Plus,
    Close,
    Check,
    Reload,
    // --- files ---
    Save,
    Folder,
    File,
    Scene,
    Script,
    Image,
    Config,
    Home,
    // --- objects ---
    Cube,
    Sphere,
    Light,
    Player,
    Group,
    Env,
    Sound,
    Material,
    // --- UI ---
    Eye,
    EyeOff,
    Lock,
    Unlock,
    Search,
    Console,
    Terminal,
    Graph,
    Layers,
    Wrench,
    Bevy,
    Warn,
    Error,
    Info,
    Monitor,
    PlayCircle,
}

/// Paint `icon` inside `rect` in `color`.
pub fn paint(painter: &Painter, icon: Icon, rect: Rect, color: Color32) {
    let s = rect.width().min(rect.height()) / 16.0;
    let origin = egui::pos2(
        rect.min.x + (rect.width() - 16.0 * s) * 0.5,
        rect.min.y + (rect.height() - 16.0 * s) * 0.5,
    );
    let p = |x: f32, y: f32| Pos2::new(origin.x + x * s, origin.y + y * s);
    let stroke = Stroke::new((1.5 * s).max(0.8), color);
    let l = |a: (f32, f32), b: (f32, f32)| painter.line_segment([p(a.0, a.1), p(b.0, b.1)], stroke);
    let c = |center: (f32, f32), r: f32| {
        painter.circle_stroke(p(center.0, center.1), r * s, stroke);
    };
    let cf = |center: (f32, f32), r: f32, col: Color32| {
        painter.circle_filled(p(center.0, center.1), r * s, col);
    };
    let poly = |pts: &[(f32, f32)]| {
        painter.add(egui::Shape::line(
            pts.iter().map(|(x, y)| p(*x, *y)).collect(),
            stroke,
        ));
    };
    let filled = |pts: Vec<Pos2>, col: Color32| {
        painter.add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
    };

    match icon {
        // Move cross: four arrows from centre.
        Icon::Translate => {
            l((8.0, 3.0), (8.0, 13.0));
            l((3.0, 8.0), (13.0, 8.0));
            filled(vec![p(8.0, 1.2), p(6.4, 3.4), p(9.6, 3.4)], color);
            filled(vec![p(8.0, 14.8), p(6.4, 12.6), p(9.6, 12.6)], color);
            filled(vec![p(1.2, 8.0), p(3.4, 6.4), p(3.4, 9.6)], color);
            filled(vec![p(14.8, 8.0), p(12.6, 6.4), p(12.6, 9.6)], color);
        }
        // Circular arrow around centre dot.
        Icon::Rotate => {
            arc(painter, p(8.0, 8.0), 5.5 * s, -40.0_f32.to_radians(), 260.0_f32.to_radians(), stroke);
            filled(
                vec![
                    p(12.4, 2.6),
                    p(13.6, 6.2),
                    p(10.0, 5.2),
                ],
                color,
            );
            cf((8.0, 8.0), 1.4, color);
        }
        // Corner bracket + diagonal arrow (scale).
        Icon::Scale => {
            poly(&[(2.0, 5.0), (2.0, 2.0), (5.0, 2.0)]);
            poly(&[(11.0, 14.0), (14.0, 14.0), (14.0, 11.0)]);
            l((3.0, 3.0), (7.0, 7.0));
            l((9.0, 9.0), (13.0, 13.0));
            filled(vec![p(13.6, 8.6), p(13.6, 13.6), p(8.6, 13.6)], color);
        }
        Icon::Grid => {
            l((2.0, 5.5), (14.0, 5.5));
            l((2.0, 10.5), (14.0, 10.5));
            l((5.5, 2.0), (5.5, 14.0));
            l((10.5, 2.0), (10.5, 14.0));
        }
        Icon::Outline => {
            let r = Rect::from_min_max(p(2.5, 2.5), p(13.5, 13.5));
            painter.rect_stroke(r, 1.0 * s, stroke, egui::StrokeKind::Inside);
        }
        Icon::Frame => {
            poly(&[(2.0, 5.0), (2.0, 2.0), (5.0, 2.0)]);
            poly(&[(11.0, 2.0), (14.0, 2.0), (14.0, 5.0)]);
            poly(&[(14.0, 11.0), (14.0, 14.0), (11.0, 14.0)]);
            poly(&[(5.0, 14.0), (2.0, 14.0), (2.0, 11.0)]);
            cf((8.0, 8.0), 1.6, color);
        }
        Icon::Camera => {
            let body = Rect::from_min_max(p(2.0, 5.0), p(11.0, 12.0));
            painter.rect_stroke(body, 1.2 * s, stroke, egui::StrokeKind::Inside);
            poly(&[(11.5, 8.0), (14.0, 5.5), (14.0, 11.5), (11.5, 9.5)]);
            cf((6.5, 8.5), 1.8, color);
        }
        Icon::Perspective => {
            poly(&[(2.0, 13.0), (5.5, 4.0), (12.5, 4.0), (14.0, 13.0)]);
            l((2.0, 13.0), (14.0, 13.0));
            l((5.5, 4.0), (14.0, 13.0));
        }
        Icon::Play => filled(vec![p(5.0, 3.0), p(13.0, 8.0), p(5.0, 13.0)], color),
        Icon::PlayCircle => {
            c((8.0, 8.0), 6.2);
            filled(vec![p(6.4, 5.2), p(11.0, 8.0), p(6.4, 10.8)], color);
        }
        Icon::Pause => {
            let r1 = Rect::from_min_max(p(4.0, 3.0), p(7.0, 13.0));
            let r2 = Rect::from_min_max(p(9.0, 3.0), p(12.0, 13.0));
            painter.rect_filled(r1, 1.0 * s, color);
            painter.rect_filled(r2, 1.0 * s, color);
        }
        Icon::Stop => {
            let r = Rect::from_min_max(p(3.5, 3.5), p(12.5, 12.5));
            painter.rect_filled(r, 1.2 * s, color);
        }
        Icon::SkipBack => {
            filled(vec![p(12.5, 3.0), p(5.0, 8.0), p(12.5, 13.0)], color);
            let r = Rect::from_min_max(p(3.2, 3.0), p(5.2, 13.0));
            painter.rect_filled(r, 0.8 * s, color);
        }
        Icon::SkipForward => {
            filled(vec![p(3.5, 3.0), p(11.0, 8.0), p(3.5, 13.0)], color);
            let r = Rect::from_min_max(p(10.8, 3.0), p(12.8, 13.0));
            painter.rect_filled(r, 0.8 * s, color);
        }
        Icon::Rewind => {
            filled(vec![p(7.5, 4.0), p(1.5, 8.0), p(7.5, 12.0)], color);
            filled(vec![p(14.5, 4.0), p(8.5, 8.0), p(14.5, 12.0)], color);
        }
        Icon::FastForward => {
            filled(vec![p(1.5, 4.0), p(7.5, 8.0), p(1.5, 12.0)], color);
            filled(vec![p(8.5, 4.0), p(14.5, 8.0), p(8.5, 12.0)], color);
        }
        Icon::Record => cf((8.0, 8.0), 4.2, color),
        Icon::Keyframe => {
            let pts = [(8.0, 2.2), (13.8, 8.0), (8.0, 13.8), (2.2, 8.0)];
            painter.add(egui::Shape::convex_polygon(
                pts.iter().map(|(x, y)| p(*x, *y)).collect(),
                Color32::TRANSPARENT,
                stroke,
            ));
        }
        Icon::Undo => {
            arc(painter, p(8.5, 8.5), 5.0 * s, 30.0_f32.to_radians(), 250.0_f32.to_radians(), stroke);
            filled(vec![p(2.0, 3.4), p(6.4, 4.4), p(3.0, 7.6)], color);
        }
        Icon::Redo => {
            arc(painter, p(7.5, 8.5), 5.0 * s, 260.0_f32.to_radians(), 210.0_f32.to_radians() + 360.0_f32.to_radians(), stroke);
            filled(vec![p(14.0, 3.4), p(9.6, 4.4), p(13.0, 7.6)], color);
        }
        Icon::Trash => {
            l((3.0, 4.5), (13.0, 4.5));
            l((6.0, 4.5), (6.0, 2.8));
            l((10.0, 4.5), (10.0, 2.8));
            l((6.5, 2.8), (9.5, 2.8));
            poly(&[(4.2, 4.5), (5.0, 13.5), (11.0, 13.5), (11.8, 4.5)]);
            l((6.8, 6.8), (6.8, 11.2));
            l((9.2, 6.8), (9.2, 11.2));
        }
        Icon::Duplicate => {
            poly(&[(5.5, 5.5), (13.5, 5.5), (13.5, 13.5), (5.5, 13.5)]);
            poly(&[(10.5, 5.5), (10.5, 2.5), (2.5, 2.5), (2.5, 10.5), (5.5, 10.5)]);
        }
        Icon::Plus => {
            l((8.0, 2.5), (8.0, 13.5));
            l((2.5, 8.0), (13.5, 8.0));
        }
        Icon::Close => {
            l((3.5, 3.5), (12.5, 12.5));
            l((12.5, 3.5), (3.5, 12.5));
        }
        Icon::Check => {
            poly(&[(2.8, 8.6), (6.6, 12.2), (13.2, 4.0)]);
        }
        Icon::Reload => {
            arc(painter, p(8.0, 8.0), 5.4 * s, 0.0, 300.0_f32.to_radians(), stroke);
            filled(vec![p(12.2, 1.6), p(14.4, 5.4), p(9.8, 5.0)], color);
        }
        Icon::Save => {
            poly(&[(2.5, 2.5), (11.5, 2.5), (13.5, 4.5), (13.5, 13.5), (2.5, 13.5)]);
            poly(&[(5.0, 2.5), (11.0, 2.5), (11.0, 6.0), (5.0, 6.0)]);
            let r = Rect::from_min_max(p(4.5, 8.5), p(11.5, 13.5));
            painter.rect_stroke(r, 0.6 * s, stroke, egui::StrokeKind::Inside);
        }
        Icon::Folder => {
            poly(&[(1.8, 13.0), (1.8, 3.5), (6.0, 3.5), (7.8, 5.5), (14.2, 5.5), (14.2, 13.0)]);
        }
        Icon::File => {
            poly(&[(3.5, 1.8), (10.0, 1.8), (12.5, 4.3), (12.5, 14.2), (3.5, 14.2)]);
            poly(&[(10.0, 1.8), (10.0, 4.3), (12.5, 4.3)]);
        }
        Icon::Scene => {
            poly(&[(8.0, 1.8), (14.0, 5.0), (14.0, 11.0), (8.0, 14.2), (2.0, 11.0), (2.0, 5.0)]);
            l((2.0, 5.0), (8.0, 8.2));
            l((8.0, 8.2), (14.0, 5.0));
            l((8.0, 8.2), (8.0, 14.2));
        }
        Icon::Script => {
            poly(&[(3.5, 1.8), (10.0, 1.8), (12.5, 4.3), (12.5, 14.2), (3.5, 14.2)]);
            l((6.0, 7.0), (4.5, 8.5));
            l((4.5, 8.5), (6.0, 10.0));
            l((9.5, 7.0), (11.0, 8.5));
            l((11.0, 8.5), (9.5, 10.0));
            l((8.3, 6.0), (7.2, 11.0));
        }
        Icon::Image => {
            let r = Rect::from_min_max(p(2.0, 3.0), p(14.0, 13.0));
            painter.rect_stroke(r, 1.0 * s, stroke, egui::StrokeKind::Inside);
            cf((5.6, 6.2), 1.3, color);
            poly(&[(3.0, 12.0), (7.0, 8.0), (10.0, 11.0), (12.0, 9.0), (14.0, 11.5), (14.0, 13.0), (2.6, 13.0)]);
        }
        Icon::Config => {
            l((3.0, 4.0), (13.0, 4.0));
            l((3.0, 8.0), (13.0, 8.0));
            l((3.0, 12.0), (13.0, 12.0));
            cf((6.0, 4.0), 1.6, color);
            cf((10.0, 8.0), 1.6, color);
            cf((5.0, 12.0), 1.6, color);
        }
        Icon::Home => {
            poly(&[(2.0, 8.0), (8.0, 2.2), (14.0, 8.0)]);
            poly(&[(4.0, 7.2), (4.0, 13.5), (12.0, 13.5), (12.0, 7.2)]);
        }
        Icon::Cube => {
            poly(&[(8.0, 1.8), (14.0, 5.0), (14.0, 11.0), (8.0, 14.2), (2.0, 11.0), (2.0, 5.0)]);
            l((2.0, 5.0), (8.0, 8.2));
            l((8.0, 8.2), (14.0, 5.0));
            l((8.0, 8.2), (8.0, 14.2));
        }
        Icon::Sphere => {
            c((8.0, 8.0), 6.0);
            arc(painter, p(8.0, 8.0), 6.0 * s, -65.0_f32.to_radians(), 65.0_f32.to_radians(), stroke);
            arc(painter, p(8.0, 8.0), 6.0 * s, 115.0_f32.to_radians(), 245.0_f32.to_radians(), stroke);
        }
        Icon::Light => {
            c((8.0, 6.5), 3.4);
            l((8.0, 0.8), (8.0, 1.9));
            l((8.0, 11.1), (8.0, 12.2));
            l((2.0, 6.5), (3.5, 6.5));
            l((12.5, 6.5), (14.0, 6.5));
            l((3.6, 2.1), (4.8, 3.3));
            l((12.4, 2.1), (11.2, 3.3));
            l((3.6, 10.9), (4.8, 9.7));
            l((12.4, 10.9), (11.2, 9.7));
            poly(&[(6.4, 13.2), (9.6, 13.2), (9.2, 14.8), (6.8, 14.8)]);
        }
        Icon::Player => {
            cf((8.0, 4.2), 2.2, color);
            poly(&[(8.0, 6.6), (11.4, 9.0), (10.2, 14.2), (5.8, 14.2), (4.6, 9.0)]);
        }
        Icon::Group => {
            let r1 = Rect::from_min_max(p(2.0, 2.0), p(7.0, 7.0));
            let r2 = Rect::from_min_max(p(9.0, 9.0), p(14.0, 14.0));
            painter.rect_stroke(r1, 1.0 * s, stroke, egui::StrokeKind::Inside);
            painter.rect_stroke(r2, 1.0 * s, stroke, egui::StrokeKind::Inside);
            l((4.5, 7.0), (4.5, 11.5));
            l((4.5, 11.5), (9.0, 11.5));
        }
        Icon::Env => {
            c((8.0, 8.0), 6.0);
            arc(painter, p(8.0, 8.0), 6.0 * s, -90.0_f32.to_radians(), 90.0_f32.to_radians(), stroke);
            arc(painter, p(8.0, 8.0), 6.0 * s, 90.0_f32.to_radians(), 270.0_f32.to_radians(), stroke);
            l((2.0, 8.0), (14.0, 8.0));
        }
        Icon::Sound => {
            poly(&[(2.5, 6.0), (5.5, 6.0), (9.0, 3.0), (9.0, 13.0), (5.5, 10.0), (2.5, 10.0)]);
            arc(painter, p(9.5, 8.0), 3.4 * s, -55.0_f32.to_radians(), 55.0_f32.to_radians(), stroke);
            arc(painter, p(9.5, 8.0), 5.6 * s, -50.0_f32.to_radians(), 50.0_f32.to_radians(), stroke);
        }
        Icon::Material => {
            let r = Rect::from_min_max(p(2.5, 2.5), p(13.5, 13.5));
            painter.rect_stroke(r, 1.0 * s, stroke, egui::StrokeKind::Inside);
            poly(&[(2.5, 11.0), (6.5, 7.0), (9.5, 10.0), (12.0, 7.5), (13.5, 9.0)]);
        }
        Icon::Eye => {
            arc(painter, p(8.0, 8.0), 5.2 * s, 25.0_f32.to_radians(), 130.0_f32.to_radians(), stroke);
            arc(painter, p(8.0, 8.0), 5.2 * s, 205.0_f32.to_radians(), 310.0_f32.to_radians(), stroke);
            cf((8.0, 8.0), 1.9, color);
        }
        Icon::EyeOff => {
            l((3.0, 3.0), (13.0, 13.0));
            arc(painter, p(8.0, 8.0), 5.2 * s, 40.0_f32.to_radians(), 110.0_f32.to_radians(), stroke);
            arc(painter, p(8.0, 8.0), 5.2 * s, 215.0_f32.to_radians(), 285.0_f32.to_radians(), stroke);
        }
        Icon::Lock => {
            let body = Rect::from_min_max(p(4.0, 7.5), p(12.0, 13.5));
            painter.rect_stroke(body, 1.0 * s, stroke, egui::StrokeKind::Inside);
            arc(painter, p(8.0, 7.5), 2.6 * s, std::f32::consts::PI, std::f32::consts::TAU, stroke);
            cf((8.0, 10.5), 1.2, color);
        }
        Icon::Unlock => {
            let body = Rect::from_min_max(p(4.0, 7.5), p(12.0, 13.5));
            painter.rect_stroke(body, 1.0 * s, stroke, egui::StrokeKind::Inside);
            arc(painter, p(10.5, 7.5), 2.6 * s, std::f32::consts::PI, std::f32::consts::TAU, stroke);
            cf((8.0, 10.5), 1.2, color);
        }
        Icon::Search => {
            c((7.0, 7.0), 4.2);
            l((10.2, 10.2), (13.8, 13.8));
        }
        Icon::Console => {
            let r = Rect::from_min_max(p(1.8, 2.8), p(14.2, 13.2));
            painter.rect_stroke(r, 1.0 * s, stroke, egui::StrokeKind::Inside);
            poly(&[(4.2, 6.0), (6.4, 8.0), (4.2, 10.0)]);
            l((8.0, 10.5), (11.5, 10.5));
        }
        Icon::Terminal => {
            let r = Rect::from_min_max(p(1.8, 2.8), p(14.2, 13.2));
            painter.rect_stroke(r, 1.0 * s, stroke, egui::StrokeKind::Inside);
            l((2.0, 5.0), (14.0, 5.0));
            cf((4.0, 3.9), 0.7, color);
            cf((6.4, 3.9), 0.7, color);
            poly(&[(4.2, 7.5), (6.4, 9.3), (4.2, 11.1)]);
            l((8.0, 11.3), (11.5, 11.3));
        }
        Icon::Graph => {
            l((2.0, 2.0), (2.0, 14.0));
            l((2.0, 14.0), (14.0, 14.0));
            l((3.5, 11.0), (6.0, 7.5));
            l((6.0, 7.5), (8.5, 9.5));
            l((8.5, 9.5), (12.5, 3.8));
        }
        Icon::Layers => {
            poly(&[(8.0, 2.0), (14.0, 5.4), (8.0, 8.8), (2.0, 5.4)]);
            poly(&[(3.4, 8.4), (8.0, 11.0), (12.6, 8.4)]);
            poly(&[(3.4, 11.4), (8.0, 14.0), (12.6, 11.4)]);
        }
        Icon::Wrench => {
            arc(painter, p(5.2, 5.2), 2.8 * s, 30.0_f32.to_radians(), 250.0_f32.to_radians(), stroke);
            l((7.2, 7.2), (13.2, 13.2));
            l((11.6, 13.4), (13.4, 11.6));
        }
        Icon::Bevy => {
            // Stylised "F" anvil-ish mark: box + beak.
            poly(&[(2.5, 5.5), (13.5, 5.5), (13.5, 9.0), (9.0, 9.0), (9.0, 13.5), (5.0, 13.5), (5.0, 9.0), (2.5, 9.0)]);
            l((2.5, 5.5), (5.0, 3.0));
            l((13.5, 5.5), (11.0, 3.0));
        }
        Icon::Warn => {
            poly(&[(8.0, 2.0), (14.5, 13.5), (1.5, 13.5)]);
            l((8.0, 6.0), (8.0, 9.6));
            cf((8.0, 11.6), 0.9, color);
        }
        Icon::Error => {
            c((8.0, 8.0), 6.0);
            l((5.6, 5.6), (10.4, 10.4));
            l((10.4, 5.6), (5.6, 10.4));
        }
        Icon::Info => {
            c((8.0, 8.0), 6.0);
            cf((8.0, 5.0), 0.95, color);
            l((8.0, 7.4), (8.0, 11.2));
        }
        Icon::Monitor => {
            let r = Rect::from_min_max(p(2.0, 3.0), p(14.0, 11.0));
            painter.rect_stroke(r, 1.0 * s, stroke, egui::StrokeKind::Inside);
            l((8.0, 11.0), (8.0, 13.5));
            l((5.0, 13.5), (11.0, 13.5));
        }
    }
}

/// Stroke arc helper (centre in absolute coords, radius in pixels).
fn arc(painter: &Painter, center: Pos2, radius: f32, from: f32, to: f32, stroke: Stroke) {
    let steps = ((to - from).abs() / 0.22).ceil().max(6.0) as usize;
    let mut pts = Vec::with_capacity(steps + 1);
    for k in 0..=steps {
        let a = from + (to - from) * (k as f32 / steps as f32);
        pts.push(Pos2::new(center.x + a.cos() * radius, center.y + a.sin() * radius));
    }
    painter.add(egui::Shape::line(pts, stroke));
}

/// Size of the standard icon button square.
pub const ICON_BUTTON: f32 = 22.0;

/// Square icon button with hover background and tooltip.
/// Returns true on click.
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, tooltip: &str) -> bool {
    icon_button_sized(ui, icon, tooltip, ICON_BUTTON, if ui.is_enabled() { ui.visuals().widgets.inactive.fg_stroke.color } else { ui.visuals().widgets.noninteractive.fg_stroke.color })
}

/// Square icon button with an explicit tint (for coloured icons).
pub fn icon_button_colored(ui: &mut egui::Ui, icon: Icon, tooltip: &str, color: Color32) -> bool {
    icon_button_sized(ui, icon, tooltip, ICON_BUTTON, color)
}

/// Toggleable icon button (pressed = accent tint).
pub fn icon_toggle(ui: &mut egui::Ui, icon: Icon, tooltip: &str, selected: bool) -> bool {
    let resp = icon_button_widget(ui, icon, ICON_BUTTON, if selected { crate::theme::ACCENT } else { ui.visuals().widgets.inactive.fg_stroke.color }, Some(selected));
    resp.on_hover_text(tooltip).clicked()
}

fn icon_button_sized(ui: &mut egui::Ui, icon: Icon, tooltip: &str, size: f32, color: Color32) -> bool {
    icon_button_widget(ui, icon, size, color, None).on_hover_text(tooltip).clicked()
}

fn icon_button_widget(ui: &mut egui::Ui, icon: Icon, size: f32, color: Color32, selected: Option<bool>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::click());
    let visuals = ui.visuals();
    let bg = if selected == Some(true) {
        crate::theme::ACCENT_DIM
    } else if resp.hovered() {
        visuals.widgets.hovered.bg_fill
    } else if resp.is_pointer_button_down_on() {
        visuals.widgets.active.bg_fill
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter();
    if bg != Color32::TRANSPARENT {
        painter.rect_filled(rect, 3.0, bg);
    }
    // Shrink the icon slightly inside the button.
    let pad = size * 0.14;
    let icon_rect = rect.shrink2(Vec2::splat(pad * 0.55));
    paint(painter, icon, icon_rect, if resp.hovered() && selected.is_none() { crate::theme::TEXT } else { color });
    resp
}

/// Small inline icon next to a label (e.g. panel titles, tree rows).
pub fn inline_icon(painter: &Painter, icon: Icon, center: Pos2, size: f32, color: Color32) {
    let rect = Rect::from_center_size(center, Vec2::splat(size));
    paint(painter, icon, rect, color);
}
