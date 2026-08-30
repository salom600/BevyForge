//! Viewport transform gizmos: translate / rotate / scale manipulators drawn
//! as a screen-space overlay on top of the streamed render frame.
//!
//! The overlay is painted by the editor using the runtime's view-projection
//! matrix (`RuntimeToEditor::CameraInfo`), so the handles stay pixel-aligned
//! with the scene regardless of camera or viewport size. Dragging sends
//! *relative* commands (`MoveEntity`, `RotateEntityWorld`, `ScaleEntityBy`)
//! which the runtime applies to the live transform; the exact pre/post pair
//! for undo comes back via `GestureDone`.
//!
//! Everything pure (matrix inverse, unprojection, closest-point-on-axis,
//! ray/plane intersection) lives in the first half of this file and is unit
//! tested; the second half is egui painting + interaction.

// ---------------------------------------------------------------------------
// Minimal column-major 4x4 matrix (matches glam's `to_cols_array` layout)
// ---------------------------------------------------------------------------

/// Column-major 4x4: `m[col * 4 + row]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat4([f32; 16]);

impl Mat4 {
    pub fn from_cols_array(flat: [f32; 16]) -> Self {
        Self(flat)
    }

    pub fn identity() -> Self {
        Self([
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ])
    }

    /// Multiply by a 4-vector.
    pub fn mul_vec4(&self, v: [f32; 4]) -> [f32; 4] {
        let m = &self.0;
        let mut out = [0.0f32; 4];
        for r in 0..4 {
            out[r] = m[r] * v[0] + m[4 + r] * v[1] + m[8 + r] * v[2] + m[12 + r] * v[3];
        }
        out
    }

    /// Full general inverse (Gauss-Jordan with partial pivoting).
    pub fn inverse(&self) -> Option<Self> {
        let mut a = self.0;
        let mut inv = Mat4::identity().0;
        for col in 0..4 {
            // Find pivot row (largest |value| in this column at/below diag).
            let mut pivot = col;
            let mut best = a[col * 4 + col].abs();
            for r in (col + 1)..4 {
                let v = a[col * 4 + r].abs();
                if v > best {
                    best = v;
                    pivot = r;
                }
            }
            if best < 1e-10 {
                return None;
            }
            if pivot != col {
                for k in 0..4 {
                    a.swap(k * 4 + col, k * 4 + pivot);
                    inv.swap(k * 4 + col, k * 4 + pivot);
                }
            }
            let diag = a[col * 4 + col];
            for k in 0..4 {
                a[k * 4 + col] /= diag;
                inv[k * 4 + col] /= diag;
            }
            for r in 0..4 {
                if r == col {
                    continue;
                }
                let factor = a[col * 4 + r];
                if factor == 0.0 {
                    continue;
                }
                for k in 0..4 {
                    a[k * 4 + r] -= factor * a[k * 4 + col];
                    inv[k * 4 + r] -= factor * inv[k * 4 + col];
                }
            }
        }
        Some(Mat4(inv))
    }
}

// ---------------------------------------------------------------------------
// Pure geometry
// ---------------------------------------------------------------------------

/// Project a world point to NDC; `None` when behind the camera (w <= 0).
pub fn world_to_ndc(vp: &Mat4, p: [f32; 3]) -> Option<[f32; 3]> {
    let c = vp.mul_vec4([p[0], p[1], p[2], 1.0]);
    if c[3] <= 1e-6 {
        return None;
    }
    Some([c[0] / c[3], c[1] / c[3], c[2] / c[3]])
}

/// Build the view ray through an NDC coordinate using the inverse VP.
/// Returns `(origin, direction)` with a normalised direction.
pub fn ndc_ray(vp_inv: &Mat4, ndc_x: f32, ndc_y: f32) -> Option<([f32; 3], [f32; 3])> {
    let near = vp_inv.mul_vec4([ndc_x, ndc_y, -1.0, 1.0]);
    let far = vp_inv.mul_vec4([ndc_x, ndc_y, 1.0, 1.0]);
    if near[3].abs() < 1e-9 || far[3].abs() < 1e-9 {
        return None;
    }
    let a = [near[0] / near[3], near[1] / near[3], near[2] / near[3]];
    let b = [far[0] / far[3], far[1] / far[3], far[2] / far[3]];
    let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if len < 1e-9 {
        return None;
    }
    Some((a, [d[0] / len, d[1] / len, d[2] / len]))
}

/// Parameter `t` of the closest point on the line `(ao, ad)` to the ray
/// `(ro, rd)`. Both directions need not be normalised. Returns the distance
/// along `ad` *measured in units of `|ad|`*.
pub fn ray_line_param(ro: [f32; 3], rd: [f32; 3], ao: [f32; 3], ad: [f32; 3]) -> Option<f32> {
    let w = [ro[0] - ao[0], ro[1] - ao[1], ro[2] - ao[2]];
    let a = dot(rd, rd);
    let b = dot(rd, ad);
    let c = dot(ad, ad);
    let d = dot(rd, w);
    let e = dot(ad, w);
    let denom = a * c - b * b;
    if denom.abs() < 1e-9 {
        return None;
    }
    Some((a * e - b * d) / denom)
}

/// Intersect a ray with the plane through `point` with normal `n`.
pub fn ray_plane(ro: [f32; 3], rd: [f32; 3], point: [f32; 3], n: [f32; 3]) -> Option<[f32; 3]> {
    let denom = dot(rd, n);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = dot([point[0] - ro[0], point[1] - ro[1], point[2] - ro[2]], n) / denom;
    if t < 0.0 {
        return None;
    }
    Some([ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t])
}

pub fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub fn normalize(v: [f32; 3]) -> Option<[f32; 3]> {
    let l = dot(v, v).sqrt();
    if l < 1e-9 {
        None
    } else {
        Some([v[0] / l, v[1] / l, v[2] / l])
    }
}

/// Screen-space distance from point `p` to segment `a-b` (all in pixels).
pub fn seg_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ap = [p[0] - a[0], p[1] - a[1]];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if len2 < 1e-9 {
        0.0
    } else {
        ((ap[0] * ab[0] + ap[1] * ab[1]) / len2).clamp(0.0, 1.0)
    };
    let dx = ap[0] - ab[0] * t;
    let dy = ap[1] - ab[1] * t;
    (dx * dx + dy * dy).sqrt()
}

/// Wrap an angle to (-PI, PI].
pub fn wrap_angle(a: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let mut x = a % tau;
    if x <= -std::f32::consts::PI {
        x += tau;
    }
    if x > std::f32::consts::PI {
        x -= tau;
    }
    x
}

/// Orthonormal basis `(u, v)` perpendicular to `n`.
pub fn plane_basis(n: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let helper = if n[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let u = normalize(cross(n, helper)).unwrap_or([1.0, 0.0, 0.0]);
    let v = normalize(cross(n, u)).unwrap_or([0.0, 1.0, 0.0]);
    (u, v)
}

/// The world-space axis vector for an axis index (global axes).
pub fn axis_vec(i: usize) -> [f32; 3] {
    match i {
        0 => [1.0, 0.0, 0.0],
        1 => [0.0, 1.0, 0.0],
        _ => [0.0, 0.0, 1.0],
    }
}

// ---------------------------------------------------------------------------
// Overlay painting + interaction (egui + app state)
// ---------------------------------------------------------------------------

use crate::app::BevyForgeApp;
use crate::theme;
use egui::Color32;

/// Extract `(translation, euler_deg, scale)` from the mirrored inspector data.
pub fn transform_from_components(
    components: &[forge_ipc::ComponentData],
) -> Option<([f32; 3], [f32; 3], [f32; 3])> {
    let mut translation = None;
    let mut euler = None;
    let mut scale = None;
    for c in components {
        for (field, row) in &c.rows {
            let v3 = |v: &forge_ipc::FieldValue| match v {
                forge_ipc::FieldValue::Vec3(v) => Some(*v),
                _ => None,
            };
            match field {
                forge_ipc::ComponentField::Translation => translation = v3(&row.value),
                forge_ipc::ComponentField::RotationEulerDeg => euler = v3(&row.value),
                forge_ipc::ComponentField::Scale => scale = v3(&row.value),
                _ => {}
            }
        }
    }
    Some((translation?, euler.unwrap_or([0.0; 3]), scale.unwrap_or([1.0; 3])))
}

/// Project a world point to viewport-image screen space (egui points).
/// Image row 0 is the TOP (verified against the runtime capture pipeline).
pub fn screen_from_world(vp: &Mat4, ir: egui::Rect, p: [f32; 3]) -> Option<egui::Pos2> {
    let ndc = world_to_ndc(vp, p)?;
    Some(egui::pos2(
        ir.min.x + (ndc[0] + 1.0) * 0.5 * ir.width(),
        ir.min.y + (1.0 - (ndc[1] + 1.0) * 0.5) * ir.height(),
    ))
}

/// View ray through an egui point (viewport-image space).
fn pointer_ray(vp_inv: &Mat4, ir: egui::Rect, pos: egui::Pos2) -> Option<([f32; 3], [f32; 3])> {
    let ndc_x = 2.0 * (pos.x - ir.min.x) / ir.width() - 1.0;
    let ndc_y = 1.0 - 2.0 * (pos.y - ir.min.y) / ir.height();
    ndc_ray(vp_inv, ndc_x, ndc_y)
}

/// World units per screen pixel at `origin` (keeps gizmo size constant).
fn world_units_per_px(
    vp: &Mat4,
    ir: egui::Rect,
    origin: [f32; 3],
    eye: [f32; 3],
) -> Option<f32> {
    let view_dir = normalize([origin[0] - eye[0], origin[1] - eye[1], origin[2] - eye[2]])?;
    let helper = if view_dir[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    let perp = normalize(cross(view_dir, helper))?;
    let p0 = screen_from_world(vp, ir, origin)?;
    let p1 = screen_from_world(vp, ir, [
        origin[0] + perp[0],
        origin[1] + perp[1],
        origin[2] + perp[2],
    ])?;
    let d = ((p1.x - p0.x).powi(2) + (p1.y - p0.y).powi(2)).sqrt();
    if d < 1e-6 {
        None
    } else {
        Some(1.0 / d)
    }
}

fn axis_color(i: usize) -> Color32 {
    match i {
        0 => theme::AXIS_X,
        1 => theme::AXIS_Y,
        _ => theme::AXIS_Z,
    }
}

fn brighten(c: Color32) -> Color32 {
    Color32::from_rgb(
        c.r().saturating_add(60),
        c.g().saturating_add(60),
        c.b().saturating_add(60),
    )
}

/// Draw the gizmo overlay and process drag interaction.
/// Returns `true` when the gizmo consumed (or just finished consuming) the
/// pointer gesture, so the caller can suppress click-to-pick.
pub fn draw_and_handle(
    app: &mut BevyForgeApp,
    ui: &mut egui::Ui,
    image_rect: egui::Rect,
    resp: &egui::Response,
) -> bool {
    let Some(vp) = app.state.camera_vp else { return false };
    let eye = app.state.camera_eye;
    let Some(sel) = app.state.selected else { return false };
    let Some((translation, _euler, _scale)) = transform_from_components(&app.state.components)
    else {
        return false;
    };
    let locked = app
        .state
        .hierarchy
        .iter()
        .find(|n| n.id == sel)
        .map(|n| n.locked)
        .unwrap_or(false);

    let mut painter = ui.painter().clone();
    painter.set_clip_rect(image_rect.intersect(ui.clip_rect()));

    let Some(wupp) = world_units_per_px(&vp, image_rect, translation, eye) else {
        return false;
    };

    // During a translate drag, anchor on the accumulated position so the
    // gizmo doesn't lag behind the (asynchronously refreshed) mirror.
    let anchor = match &app.state.gizmo_drag {
        Some(d) if matches!(d.handle, GizmoHandle::TranslateAxis(_) | GizmoHandle::TranslatePlane(_, _) | GizmoHandle::TranslateView) => [
            d.start_translation[0] + d.accum_delta[0],
            d.start_translation[1] + d.accum_delta[1],
            d.start_translation[2] + d.accum_delta[2],
        ],
        _ => translation,
    };
    let Some(origin_px) = screen_from_world(&vp, image_rect, anchor) else {
        return false;
    };
    if !image_rect.expand(200.0).contains(origin_px) {
        return false;
    }

    let arrow_world = ARROW_LEN * wupp;
    let ring_world = RING_RADIUS * wupp;

    // ---- hit testing (before drawing so we can highlight) ----------------
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let hovered = if app.state.gizmo_drag.is_none() && !locked {
        pointer.and_then(|p| hit_test(app, &vp, image_rect, anchor, eye, origin_px, arrow_world, ring_world, p))
    } else {
        app.state.gizmo_drag.map(|d| d.handle)
    };

    // ---- painting --------------------------------------------------------
    match app.state.gizmo_mode {
        GizmoMode::Translate => paint_translate(&painter, &vp, image_rect, anchor, origin_px, arrow_world, hovered, locked),
        GizmoMode::Rotate => paint_rotate(&painter, &vp, image_rect, anchor, eye, origin_px, ring_world, hovered, locked),
        GizmoMode::Scale => paint_scale(&painter, &vp, image_rect, anchor, origin_px, arrow_world, hovered, locked),
    }

    // ---- interaction -----------------------------------------------------
    let mut consumed = false;
    if locked {
        return false;
    }
    if let Some(mut drag) = app.state.gizmo_drag.take() {
        consumed = true;
        if resp.drag_stopped() || ui.input(|i| !i.pointer.primary_down()) {
            app.cmd(forge_ipc::EditorToRuntime::EndGizmoGesture {
                entity: sel,
                label: drag.handle.undo_label(&app.state.selected_name),
            });
        } else if resp.dragged() {
            apply_drag(app, &mut drag, &vp, image_rect, anchor, eye, origin_px, wupp);
            app.state.gizmo_drag = Some(drag);
        } else {
            app.state.gizmo_drag = Some(drag);
        }
    } else if ui.input(|i| i.pointer.primary_pressed()) && resp.hovered() {
        // Arm the gesture on the PRESS frame (not egui's drag_started, which
        // only fires after the ~6 px threshold — by then the pointer has left
        // the thin handle geometry and the hit test would miss).
        if let Some(p) = pointer {
            if let Some(handle) = hit_test(app, &vp, image_rect, anchor, eye, origin_px, arrow_world, ring_world, p) {
                let Some((ro, rd)) = pointer_ray(&vp.inverse().unwrap_or(Mat4::identity()), image_rect, p)
                else {
                    return false;
                };
                let mut drag = DragState {
                    handle,
                    start_translation: translation,
                    accum_delta: [0.0; 3],
                    start_param: 0.0,
                    last_param: 0.0,
                    last_plane_point: anchor,
                    last_angle: 0.0,
                    accum_angle: 0.0,
                    sent_angle: 0.0,
                    circle_axis: [0.0, 1.0, 0.0],
                    accum_factor: [1.0; 3],
                    anchor_screen: origin_px,
                };
                // Initialise constraint values; abort the grab when the
                // pointer ray cannot see the constraint geometry.
                let ok = match handle {
                    GizmoHandle::TranslateAxis(i) => {
                        let a = axis_vec(i);
                        match ray_line_param(ro, rd, anchor, a) {
                            Some(t) => {
                                drag.last_param = t;
                                true
                            }
                            None => false,
                        }
                    }
                    GizmoHandle::TranslatePlane(a_i, b_i) => {
                        let Some(n) = normalize(cross(axis_vec(a_i), axis_vec(b_i))) else {
                            return false;
                        };
                        match ray_plane(ro, rd, anchor, n) {
                            Some(q) => {
                                drag.last_plane_point = q;
                                true
                            }
                            None => false,
                        }
                    }
                    GizmoHandle::TranslateView => {
                        let Some(n) = normalize([
                            anchor[0] - eye[0],
                            anchor[1] - eye[1],
                            anchor[2] - eye[2],
                        ]) else {
                            return false;
                        };
                        match ray_plane(ro, rd, anchor, n) {
                            Some(q) => {
                                drag.last_plane_point = q;
                                true
                            }
                            None => false,
                        }
                    }
                    GizmoHandle::RotateAxis(i) => {
                        let a = axis_vec(i);
                        let (u, v) = plane_basis(a);
                        match ray_plane(ro, rd, anchor, a) {
                            Some(q) => {
                                let rel = [q[0] - anchor[0], q[1] - anchor[1], q[2] - anchor[2]];
                                drag.last_angle = rel[1].atan2(rel[0]);
                                let _ = (u, v);
                                drag.circle_axis = a;
                                true
                            }
                            None => false,
                        }
                    }
                    GizmoHandle::ScaleAxis(i) => {
                        let a = axis_vec(i);
                        match ray_line_param(ro, rd, anchor, a) {
                            Some(t) if t.abs() > 1e-3 => {
                                drag.start_param = t;
                                drag.last_param = t;
                                true
                            }
                            Some(_) => true, // degenerate grab; fallback path
                            None => false,
                        }
                    }
                    GizmoHandle::ScaleUniform => {
                        drag.start_param = ((p.x - origin_px.x).powi(2)
                            + (p.y - origin_px.y).powi(2))
                            .sqrt()
                            .max(4.0);
                        true
                    }
                };
                if ok {
                    app.cmd(forge_ipc::EditorToRuntime::BeginGizmoGesture { entity: sel });
                    app.state.gizmo_drag = Some(drag);
                    consumed = true;
                }
            }
        }
    }
    consumed
}

/// Closest gizmo handle to the pointer, within the grab threshold.
#[allow(clippy::too_many_arguments)]
fn hit_test(
    app: &BevyForgeApp,
    vp: &Mat4,
    ir: egui::Rect,
    anchor: [f32; 3],
    eye: [f32; 3],
    origin_px: egui::Pos2,
    arrow_world: f32,
    ring_world: f32,
    p: egui::Pos2,
) -> Option<GizmoHandle> {
    let mut best: Option<(f32, GizmoHandle)> = None;
    let mut consider = |dist: f32, handle: GizmoHandle| {
        if dist < 8.0 && best.map(|(d, _)| dist < d).unwrap_or(true) {
            best = Some((dist, handle));
        }
    };
    let proj = |w: [f32; 3]| screen_from_world(vp, ir, w);

    match app.state.gizmo_mode {
        GizmoMode::Translate => {
            for i in 0..3 {
                let tip = proj([
                    anchor[0] + axis_vec(i)[0] * arrow_world,
                    anchor[1] + axis_vec(i)[1] * arrow_world,
                    anchor[2] + axis_vec(i)[2] * arrow_world,
                ]);
                if let Some(tip) = tip {
                    let d = seg_distance(
                        [p.x, p.y],
                        [origin_px.x, origin_px.y],
                        [tip.x, tip.y],
                    );
                    consider(d - 2.0, GizmoHandle::TranslateAxis(i));
                }
            }
            // Plane squares.
            for (a, b) in [(0, 1), (0, 2), (1, 2)] {
                let off_a = axis_vec(a)[a] * arrow_world * 0.36;
                let off_b = axis_vec(b)[b] * arrow_world * 0.36;
                let mut w = anchor;
                w[a] += off_a;
                w[b] += off_b;
                if let Some(c) = proj(w) {
                    let rect = egui::Rect::from_center_size(c, egui::vec2(11.0, 11.0));
                    if rect.expand(2.5).contains(p) {
                        let dx = (p.x - c.x).abs().max((p.y - c.y).abs());
                        consider(dx.max(0.1), GizmoHandle::TranslatePlane(a, b));
                    }
                }
            }
            let dc = ((p.x - origin_px.x).powi(2) + (p.y - origin_px.y).powi(2)).sqrt();
            consider(dc - 4.0, GizmoHandle::TranslateView);
        }
        GizmoMode::Rotate => {
            for i in 0..3 {
                let a = axis_vec(i);
                let (u, v) = plane_basis(a);
                // Facing test: hide the ring whose plane is near-parallel to
                // the view ray (can't be grabbed reliably anyway).
                let view = normalize([anchor[0] - eye[0], anchor[1] - eye[1], anchor[2] - eye[2]])?;
                if dot(view, a).abs() > 0.985 {
                    continue;
                }
                let mut min_d = f32::INFINITY;
                let mut prev: Option<egui::Pos2> = None;
                for k in 0..=48 {
                    let th = (k as f32 / 48.0) * std::f32::consts::TAU;
                    let w = [
                        anchor[0] + (u[0] * th.cos() + v[0] * th.sin()) * ring_world,
                        anchor[1] + (u[1] * th.cos() + v[1] * th.sin()) * ring_world,
                        anchor[2] + (u[2] * th.cos() + v[2] * th.sin()) * ring_world,
                    ];
                    if let Some(s) = proj(w) {
                        if let Some(prev) = prev {
                            let d = seg_distance([p.x, p.y], [prev.x, prev.y], [s.x, s.y]);
                            min_d = min_d.min(d);
                        }
                        prev = Some(s);
                    }
                }
                consider(min_d - 3.0, GizmoHandle::RotateAxis(i));
            }
        }
        GizmoMode::Scale => {
            for i in 0..3 {
                let tip = proj([
                    anchor[0] + axis_vec(i)[0] * arrow_world,
                    anchor[1] + axis_vec(i)[1] * arrow_world,
                    anchor[2] + axis_vec(i)[2] * arrow_world,
                ]);
                if let Some(tip) = tip {
                    let d = ((p.x - tip.x).powi(2) + (p.y - tip.y).powi(2)).sqrt();
                    consider(d - 3.0, GizmoHandle::ScaleAxis(i));
                    let d_line = seg_distance(
                        [p.x, p.y],
                        [origin_px.x, origin_px.y],
                        [tip.x, tip.y],
                    );
                    consider(d_line - 2.0, GizmoHandle::ScaleAxis(i));
                }
            }
            let dc = ((p.x - origin_px.x).powi(2) + (p.y - origin_px.y).powi(2)).sqrt();
            consider(dc - 4.0, GizmoHandle::ScaleUniform);
        }
    }
    let _ = eye;
    best.map(|(_, h)| h)
}

fn apply_drag(
    app: &mut BevyForgeApp,
    drag: &mut DragState,
    vp: &Mat4,
    ir: egui::Rect,
    anchor: [f32; 3],
    eye: [f32; 3],
    origin_px: egui::Pos2,
    wupp: f32,
) {
    let Some(sel) = app.state.selected else { return };
    let Some(pointer) = app.ui_ctx.as_ref().and_then(|ctx| {
        ctx.input(|i| i.pointer.interact_pos())
    }) else {
        return;
    };
    let vp_inv = match vp.inverse() {
        Some(m) => m,
        None => return,
    };
    let Some((ro, rd)) = pointer_ray(&vp_inv, ir, pointer) else {
        return;
    };
    let snap = app.ui_ctx.as_ref().map(|ctx| ctx.input(|i| i.modifiers.ctrl)).unwrap_or(false);

    match drag.handle {
        GizmoHandle::TranslateAxis(i) => {
            let Some(t) = ray_line_param(ro, rd, anchor, axis_vec(i)) else { return };
            let inc = t - drag.last_param;
            drag.last_param = t;
            let mut delta = [axis_vec(i)[0] * inc, axis_vec(i)[1] * inc, axis_vec(i)[2] * inc];
            if snap {
                // Quantise the absolute position on the dragged axis.
                let abs_i = drag.start_translation[i] + drag.accum_delta[i] + inc;
                let abs_q = (abs_i / 0.25).round() * 0.25;
                delta[i] = abs_q - (drag.start_translation[i] + drag.accum_delta[i]);
                for k in 0..3 {
                    if k != i {
                        delta[k] = 0.0;
                    }
                }
            }
            if delta.iter().all(|v| v.abs() < 1e-6) {
                return;
            }
            app.cmd(forge_ipc::EditorToRuntime::MoveEntity { entity: sel, delta });
            for k in 0..3 {
                drag.accum_delta[k] += delta[k];
            }
        }
        GizmoHandle::TranslatePlane(a, b) => {
            let n = match normalize(cross(axis_vec(a), axis_vec(b))) {
                Some(n) => n,
                None => return,
            };
            let Some(q) = ray_plane(ro, rd, anchor, n) else { return };
            let mut delta = [
                q[0] - drag.last_plane_point[0],
                q[1] - drag.last_plane_point[1],
                q[2] - drag.last_plane_point[2],
            ];
            // Zero the axis that isn't in the plane.
            for k in 0..3 {
                if k != a && k != b {
                    delta[k] = 0.0;
                }
            }
            if snap {
                for k in [a, b] {
                    let abs = drag.start_translation[k] + drag.accum_delta[k] + delta[k];
                    let abs_q = (abs / 0.25).round() * 0.25;
                    delta[k] = abs_q - (drag.start_translation[k] + drag.accum_delta[k]);
                }
            }
            drag.last_plane_point = q;
            if delta.iter().all(|v| v.abs() < 1e-6) {
                return;
            }
            app.cmd(forge_ipc::EditorToRuntime::MoveEntity { entity: sel, delta });
            for k in 0..3 {
                drag.accum_delta[k] += delta[k];
            }
        }
        GizmoHandle::TranslateView => {
            let n = match normalize([anchor[0] - eye[0], anchor[1] - eye[1], anchor[2] - eye[2]]) {
                Some(n) => n,
                None => return,
            };
            let Some(q) = ray_plane(ro, rd, anchor, n) else { return };
            let delta = [
                q[0] - drag.last_plane_point[0],
                q[1] - drag.last_plane_point[1],
                q[2] - drag.last_plane_point[2],
            ];
            drag.last_plane_point = q;
            if delta.iter().all(|v| v.abs() < 1e-6) {
                return;
            }
            app.cmd(forge_ipc::EditorToRuntime::MoveEntity { entity: sel, delta });
            for k in 0..3 {
                drag.accum_delta[k] += delta[k];
            }
        }
        GizmoHandle::RotateAxis(_i) => {
            let a = drag.circle_axis;
            let Some(q) = ray_plane(ro, rd, anchor, a) else { return };
            let (u, v) = plane_basis(a);
            let rel = [q[0] - anchor[0], q[1] - anchor[1], q[2] - anchor[2]];
            let angle = dot(rel, v).atan2(dot(rel, u));
            let delta = wrap_angle(angle - drag.last_angle);
            drag.last_angle = angle;
            drag.accum_angle = wrap_angle(drag.accum_angle + delta);
            // Optional 15-degree snapping on the TOTAL angle.
            let total = if snap {
                let step = 15.0_f32.to_radians();
                (drag.accum_angle / step).round() * step
            } else {
                drag.accum_angle
            };
            let send = wrap_angle(total - drag.sent_angle);
            if send.abs() < 1e-5 {
                return;
            }
            drag.sent_angle = wrap_angle(drag.sent_angle + send);
            app.cmd(forge_ipc::EditorToRuntime::RotateEntityWorld {
                entity: sel,
                axis: a,
                angle_deg: send.to_degrees(),
            });
        }
        GizmoHandle::ScaleAxis(i) => {
            let Some(t) = ray_line_param(ro, rd, anchor, axis_vec(i)) else { return };
            drag.last_param = t;
            // Scale by the ratio of the grab-point distance; fall back to a
            // screen-space method when the grab started at the origin.
            let desired = if drag.start_param.abs() > 1e-3 {
                (t / drag.start_param).clamp(0.01, 100.0)
            } else {
                let dir_px = screen_dir_of_axis(vp, ir, anchor, i, origin_px);
                let motion = ((pointer.x - drag.anchor_screen.x) * dir_px.0
                    + (pointer.y - drag.anchor_screen.y) * dir_px.1)
                    * 0.01;
                (1.0 + motion).clamp(0.01, 100.0)
            };
            if snap {
                let s = 0.05;
                let desired = (desired / s).round() * s;
                let desired = desired.max(0.05);
                let prev = drag.accum_factor[i];
                let factor = desired / prev.max(1e-6);
                if (factor - 1.0).abs() < 1e-6 {
                    return;
                }
                let mut f = [1.0; 3];
                f[i] = factor;
                drag.accum_factor[i] = desired;
                app.cmd(forge_ipc::EditorToRuntime::ScaleEntityBy { entity: sel, factor: f });
            } else {
                let prev = drag.accum_factor[i];
                let factor = desired / prev.max(1e-6);
                if (factor - 1.0).abs() < 1e-6 {
                    return;
                }
                drag.accum_factor[i] = desired;
                let mut f = [1.0; 3];
                f[i] = factor;
                app.cmd(forge_ipc::EditorToRuntime::ScaleEntityBy { entity: sel, factor: f });
            }
        }
        GizmoHandle::ScaleUniform => {
            let d_now = ((pointer.x - origin_px.x).powi(2) + (pointer.y - origin_px.y).powi(2))
                .sqrt()
                .max(4.0);
            let desired = (d_now / drag.start_param).clamp(0.01, 100.0);
            let desired = if snap { (desired / 0.05).round() * 0.05 } else { desired };
            let prev = drag.accum_factor[0];
            let factor = desired / prev.max(1e-6);
            if (factor - 1.0).abs() < 1e-6 {
                return;
            }
            drag.accum_factor = [desired; 3];
            app.cmd(forge_ipc::EditorToRuntime::ScaleEntityBy {
                entity: sel,
                factor: [factor; 3],
            });
        }
    }
    let _ = wupp;
}

/// Screen-space direction of a world axis (for the degenerate-scale fallback).
fn screen_dir_of_axis(
    vp: &Mat4,
    ir: egui::Rect,
    anchor: [f32; 3],
    i: usize,
    origin_px: egui::Pos2,
) -> (f32, f32) {
    let tip = screen_from_world(
        vp,
        ir,
        [
            anchor[0] + axis_vec(i)[0],
            anchor[1] + axis_vec(i)[1],
            anchor[2] + axis_vec(i)[2],
        ],
    );
    match tip {
        Some(t) => {
            let dx = t.x - origin_px.x;
            let dy = t.y - origin_px.y;
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1e-5 {
                (1.0, 0.0)
            } else {
                (dx / len, dy / len)
            }
        }
        None => (1.0, 0.0),
    }
}

// ---------------------------------------------------------------------------
// Painters
// ---------------------------------------------------------------------------

fn stroke_for(hovered: bool, color: Color32) -> egui::Stroke {
    if hovered {
        egui::Stroke::new(3.0, brighten(color))
    } else {
        egui::Stroke::new(2.0, color)
    }
}

fn paint_translate(
    painter: &egui::Painter,
    vp: &Mat4,
    ir: egui::Rect,
    anchor: [f32; 3],
    origin_px: egui::Pos2,
    arrow_world: f32,
    hovered: Option<GizmoHandle>,
    locked: bool,
) {
    let dim = |c: Color32| if locked { dim_color(c) } else { c };
    // Axis arrows.
    for i in 0..3 {
        let hov = matches!(hovered, Some(GizmoHandle::TranslateAxis(j)) if j == i);
        let tip = screen_from_world(
            vp,
            ir,
            [
                anchor[0] + axis_vec(i)[0] * arrow_world,
                anchor[1] + axis_vec(i)[1] * arrow_world,
                anchor[2] + axis_vec(i)[2] * arrow_world,
            ],
        );
        let Some(tip) = tip else { continue };
        let color = dim(axis_color(i));
        painter.line_segment([origin_px, tip], stroke_for(hov, color));
        // Arrow head.
        let dir = egui::vec2(tip.x - origin_px.x, tip.y - origin_px.y);
        let len = dir.length();
        if len > 1.0 {
            let dir = dir / len;
            let n = egui::vec2(-dir.y, dir.x);
            let head = 9.0;
            let p1 = tip - dir * head + n * head * 0.45;
            let p2 = tip - dir * head - n * head * 0.45;
            painter.add(egui::Shape::convex_polygon(
                vec![tip, p1, p2],
                color,
                egui::Stroke::NONE,
            ));
        }
        // Axis letter chip.
        let label = ["X", "Y", "Z"][i];
        painter.text(
            tip + dir * 12.0,
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(10.5),
            color,
        );
    }
    // Plane squares (XY, XZ, YZ).
    for (a, b) in [(0, 1), (0, 2), (1, 2)] {
        let hov = matches!(hovered, Some(GizmoHandle::TranslatePlane(x, y)) if (x, y) == (a, b));
        let mut w = anchor;
        w[a] += axis_vec(a)[a] * arrow_world * 0.36;
        w[b] += axis_vec(b)[b] * arrow_world * 0.36;
        let Some(c) = screen_from_world(vp, ir, w) else { continue };
        let rect = egui::Rect::from_center_size(c, egui::vec2(11.0, 11.0));
        let base = axis_color(a).gamma_multiply(0.45);
        painter.rect_filled(rect, 2.0, if hov { brighten(base) } else { dim(base) });
        painter.rect_stroke(rect, 2.0, stroke_for(hov, dim(axis_color(a))), egui::StrokeKind::Inside);
    }
    // Centre free-move dot.
    let hov = matches!(hovered, Some(GizmoHandle::TranslateView));
    let white = dim(Color32::from_rgb(0xf0, 0xf3, 0xf8));
    painter.circle_filled(origin_px, if hov { 6.0 } else { 4.5 }, if hov { white } else { white.gamma_multiply(0.85) });
}

#[allow(clippy::too_many_arguments)]
fn paint_rotate(
    painter: &egui::Painter,
    vp: &Mat4,
    ir: egui::Rect,
    anchor: [f32; 3],
    eye: [f32; 3],
    origin_px: egui::Pos2,
    ring_world: f32,
    hovered: Option<GizmoHandle>,
    locked: bool,
) {
    let dim = |c: Color32| if locked { dim_color(c) } else { c };
    for i in 0..3 {
        let a = axis_vec(i);
        let (u, v) = plane_basis(a);
        let Some(view) = normalize([anchor[0] - eye[0], anchor[1] - eye[1], anchor[2] - eye[2]])
        else {
            continue;
        };
        let facing = dot(view, a).abs();
        if facing > 0.985 {
            continue; // edge-on ring: not grabbable
        }
        let hov = matches!(hovered, Some(GizmoHandle::RotateAxis(j)) if j == i);
        let color = dim(axis_color(i));
        let alpha = if facing > 0.85 { 0.55 } else { 1.0 };
        let stroke = if hov {
            stroke_for(true, color)
        } else {
            egui::Stroke::new(2.0, color.gamma_multiply(alpha))
        };
        let mut points = Vec::with_capacity(50);
        for k in 0..=48 {
            let th = (k as f32 / 48.0) * std::f32::consts::TAU;
            let w = [
                anchor[0] + (u[0] * th.cos() + v[0] * th.sin()) * ring_world,
                anchor[1] + (u[1] * th.cos() + v[1] * th.sin()) * ring_world,
                anchor[2] + (u[2] * th.cos() + v[2] * th.sin()) * ring_world,
            ];
            if let Some(s) = screen_from_world(vp, ir, w) {
                points.push(s);
            }
        }
        painter.add(egui::Shape::line(points, stroke));
    }
    // Centre pivot dot.
    let white = dim(Color32::from_rgb(0xf0, 0xf3, 0xf8));
    painter.circle_filled(origin_px, 3.0, white.gamma_multiply(0.8));
}

fn paint_scale(
    painter: &egui::Painter,
    vp: &Mat4,
    ir: egui::Rect,
    anchor: [f32; 3],
    origin_px: egui::Pos2,
    arrow_world: f32,
    hovered: Option<GizmoHandle>,
    locked: bool,
) {
    let dim = |c: Color32| if locked { dim_color(c) } else { c };
    for i in 0..3 {
        let hov = matches!(hovered, Some(GizmoHandle::ScaleAxis(j)) if j == i);
        let tip = screen_from_world(
            vp,
            ir,
            [
                anchor[0] + axis_vec(i)[0] * arrow_world,
                anchor[1] + axis_vec(i)[1] * arrow_world,
                anchor[2] + axis_vec(i)[2] * arrow_world,
            ],
        );
        let Some(tip) = tip else { continue };
        let color = dim(axis_color(i));
        painter.line_segment([origin_px, tip], stroke_for(hov, color));
        let rect = egui::Rect::from_center_size(tip, egui::vec2(8.0, 8.0));
        painter.rect_filled(rect, 1.5, if hov { brighten(color) } else { color });
    }
    // Uniform centre cube.
    let hov = matches!(hovered, Some(GizmoHandle::ScaleUniform));
    let white = dim(Color32::from_rgb(0xf0, 0xf3, 0xf8));
    let rect = egui::Rect::from_center_size(origin_px, egui::vec2(if hov { 12.0 } else { 9.0 }, if hov { 12.0 } else { 9.0 }));
    painter.rect_filled(rect, 2.0, white.gamma_multiply(0.9));
}

fn dim_color(c: Color32) -> Color32 {
    Color32::from_rgba_premultiplied(c.r() / 2, c.g() / 2, c.b() / 2, 90)
}


// ---------------------------------------------------------------------------
// Gizmo model
// ---------------------------------------------------------------------------

/// Active manipulator mode (toolbar buttons / W-E-R shortcuts).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GizmoMode {
    #[default]
    Translate,
    Rotate,
    Scale,
}

impl GizmoMode {
    pub fn label(self) -> &'static str {
        match self {
            GizmoMode::Translate => "Translate",
            GizmoMode::Rotate => "Rotate",
            GizmoMode::Scale => "Scale",
        }
    }
}

/// Which gizmo sub-handle the pointer grabbed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GizmoHandle {
    /// Single axis translate (index 0/1/2).
    TranslateAxis(usize),
    /// Two-axis plane translate, e.g. `[0, 1]` = XY.
    TranslatePlane(usize, usize),
    /// Camera-facing free move (centre marker).
    TranslateView,
    /// Rotation ring around axis i.
    RotateAxis(usize),
    /// Per-axis scale (index).
    ScaleAxis(usize),
    /// Uniform scale (centre cube).
    ScaleUniform,
}

impl GizmoHandle {
    pub fn undo_label(self, name: &str) -> String {
        match self {
            GizmoHandle::TranslateAxis(_) | GizmoHandle::TranslatePlane(_, _) | GizmoHandle::TranslateView => {
                format!("Translate {name}")
            }
            GizmoHandle::RotateAxis(_) => format!("Rotate {name}"),
            GizmoHandle::ScaleAxis(_) | GizmoHandle::ScaleUniform => format!("Scale {name}"),
        }
    }
}

/// A gizmo drag in progress. `accum_*` hold the TOTAL applied change for the
/// gesture so the anchor position stays exact even while the mirrored state
/// lags a frame behind.
#[derive(Debug, Clone, Copy)]
pub struct DragState {
    pub handle: GizmoHandle,
    /// Pre-drag translation (world units).
    pub start_translation: [f32; 3],
    /// Translate: total delta applied so far.
    pub accum_delta: [f32; 3],
    /// Scale: ray parameter along the axis at grab time (fallback base).
    pub start_param: f32,
    /// Translate axis: last ray parameter. Scale: current ray parameter.
    pub last_param: f32,
    /// Translate plane/view: last intersection point on the constraint plane.
    pub last_plane_point: [f32; 3],
    /// Rotate: last angle (radians) measured in the circle plane.
    pub last_angle: f32,
    /// Rotate: total accumulated measured angle (radians).
    pub accum_angle: f32,
    /// Rotate: total angle actually sent to the runtime (radians).
    pub sent_angle: f32,
    /// Rotate: circle axis (world).
    pub circle_axis: [f32; 3],
    /// Scale: total per-axis factor applied so far.
    pub accum_factor: [f32; 3],
    /// Screen anchor used by the uniform-scale / degenerate fallbacks.
    pub anchor_screen: egui::Pos2,
}

/// Constant handle lengths (screen px) for a professional look.
pub const ARROW_LEN: f32 = 68.0;
pub const RING_RADIUS: f32 = 58.0;

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn mat4_inverse_roundtrip() {
        let m = Mat4::from_cols_array([
            2.0, 0.5, 0.1, 1.0, //
            0.0, 1.5, 0.0, -2.0, //
            0.3, 0.0, 1.2, 0.5, //
            0.0, 0.0, 0.0, 1.0,
        ]);
        let inv = m.inverse().expect("invertible");
        for c in 0..4 {
            for r in 0..4 {
                let mut sum = 0.0;
                for k in 0..4 {
                    sum += m.0[k * 4 + r] * inv.0[c * 4 + k];
                }
                assert!(approx(sum, if c == r { 1.0 } else { 0.0 }), "({c},{r})={sum}");
            }
        }
    }

    #[test]
    fn mat4_singular_has_no_inverse() {
        let m = Mat4::from_cols_array([1.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
        assert!(m.inverse().is_none());
    }

    /// A classic perspective view-projection: camera at origin looking down
    /// -Z. World +X must project to +X NDC, world +Y to +Y NDC.
    #[test]
    fn world_to_ndc_perspective() {
        let fov = 90.0_f32.to_radians();
        let aspect = 1.0;
        let proj = Mat4::from_cols_array([
            1.0 / (aspect * (fov * 0.5).tan()), 0.0, 0.0, 0.0, //
            0.0, 1.0 / (fov * 0.5).tan(), 0.0, 0.0, //
            0.0, 0.0, -1.01, -1.0, // bevy-style: negate z, w = -z
            0.0, 0.0, -2.02, 0.0, // far/near packing
        ]);
        // Simple affine "view" (identity): VP = proj.
        let ndc = world_to_ndc(&proj, [2.0, 1.0, -4.0]).expect("in front");
        // x_ndc = (2 / (4 * tan(45) * 1)) = 0.5, y_ndc = 1/4 = 0.25
        assert!(approx(ndc[0], 0.5), "x={}", ndc[0]);
        assert!(approx(ndc[1], 0.25), "y={}", ndc[1]);
        assert!(world_to_ndc(&proj, [0.0, 0.0, 4.0]).is_none(), "behind camera");
    }

    #[test]
    fn ndc_ray_recovers_direction() {
        // Identity VP is not a valid projection; build the same one as above.
        let fov = 90.0_f32.to_radians();
        let proj = Mat4::from_cols_array([
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, -1.01, -1.0, //
            0.0, 0.0, -2.02, 0.0,
        ]);
        let inv = proj.inverse().unwrap();
        let (origin, dir) = ndc_ray(&inv, 0.0, 0.0).unwrap();
        // NDC z = -1 sits at z = -2.02 / 2.01 ≈ -1.005 for this matrix.
        assert!(approx(origin[0], 0.0) && approx(origin[1], 0.0) && approx(origin[2], -1.005));
        // Centre ray looks straight down -Z.
        assert!(approx(dir[0], 0.0) && approx(dir[1], 0.0) && approx(dir[2], -1.0));
    }

    #[test]
    fn ray_line_param_basic() {
        // Ray from (0,1,0) pointing -Y; line = X axis. Closest at x=0.
        let t = ray_line_param([0.0, 1.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]).unwrap();
        assert!(approx(t, 0.0));
        // Offset ray: closest point at x = 3.
        let t = ray_line_param([3.0, 5.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]).unwrap();
        assert!(approx(t, 3.0));
    }

    #[test]
    fn ray_plane_basic() {
        let hit = ray_plane([0.0, 2.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        assert!(approx(hit[0], 0.0) && approx(hit[1], 0.0) && approx(hit[2], 0.0));
        // Parallel ray: no hit.
        assert!(ray_plane([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0]).is_none());
    }

    #[test]
    fn seg_distance_ends() {
        let d = seg_distance([0.0, -3.0], [0.0, 0.0], [0.0, 10.0]);
        assert!(approx(d, 3.0));
        let d = seg_distance([5.0, 10.0], [0.0, 0.0], [0.0, 10.0]);
        assert!(approx(d, 5.0));
    }

    #[test]
    fn wrap_angle_bounds() {
        assert!(approx(wrap_angle(3.5 * std::f32::consts::PI), -0.5 * std::f32::consts::PI));
        // Exact -PI is a boundary: both -PI and PI are acceptable.
        let wrapped = wrap_angle(-3.0 * std::f32::consts::PI);
        assert!(approx(wrapped.abs(), std::f32::consts::PI));
        assert!(approx(wrap_angle(0.25), 0.25));
    }

    #[test]
    fn plane_basis_orthonormal() {
        for n in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.577, 0.577, 0.577]] {
            let (u, v) = plane_basis(n);
            assert!(approx(dot(u, v), 0.0));
            assert!(approx(dot(u, n), 0.0));
            assert!(approx(dot(v, n), 0.0));
            assert!(approx(dot(u, u), 1.0) && approx(dot(v, v), 1.0));
        }
    }
}
