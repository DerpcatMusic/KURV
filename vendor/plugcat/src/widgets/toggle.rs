use crate::tokens::{LIGHT_TOKENS, WidgetTokens, lerp_color, with_alpha};
use egui::{Color32, Mesh, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2};
use std::hash::Hash;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PowerToggleSize {
    Compact,
    Regular,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PowerToggleOrientation {
    Vertical,
    Horizontal,
}

/// CSS reference proportions (`width = 150px`).
const VERTICAL_HEIGHT_PER_WIDTH: f32 = 1.30;
const HORIZONTAL_HEIGHT_PER_WIDTH: f32 = 0.42;
const PADDING_FRAC: f32 = 20.0 / 150.0;
const PIVOT_Z_FRAC: f32 = 20.0 / 150.0;
const PERSPECTIVE_FRAC: f32 = 700.0 / 150.0;
const EXTRUSION_FRAC: f32 = 50.0 / 150.0;
const ROCKER_ANGLE_DEG: f32 = 25.0;
const ANIM_TIME_SECS: f32 = 0.24;
const GRADIENT_ROWS: usize = 18;

#[derive(Clone, Copy)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
}

#[derive(Clone, Copy)]
enum FaceKind {
    Front,
    Back,
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Clone, Copy)]
struct TogglePaintState {
    anim_t: f32,
    rocker_angle: f32,
    hovered: bool,
    orientation: PowerToggleOrientation,
}

#[derive(Clone, Copy)]
struct RockerBuildParams {
    width: f32,
    height: f32,
    depth: f32,
    pivot_z: f32,
    perspective: f32,
    screen_origin: Pos2,
    angle: f32,
    orientation: PowerToggleOrientation,
    lamp: Color32,
    white: Color32,
    shadow: Color32,
}

#[derive(Clone, Copy)]
struct RockerFace {
    kind: FaceKind,
    avg_z: f32,
    corners: [Pos2; 4],
    top: Color32,
    middle: Color32,
    bottom: Color32,
}

pub fn power_toggle(ui: &mut Ui, active: &mut bool, size: PowerToggleSize) -> Response {
    power_toggle_with_tokens(ui, active, size, &LIGHT_TOKENS)
}

pub fn power_toggle_with_tokens(
    ui: &mut Ui,
    active: &mut bool,
    size: PowerToggleSize,
    tokens: &WidgetTokens,
) -> Response {
    let width = match size {
        PowerToggleSize::Compact => tokens.spacing.lg * 4.5,
        PowerToggleSize::Regular => tokens.spacing.lg * 5.7,
    };
    let frame_size = Vec2::new(width, width * VERTICAL_HEIGHT_PER_WIDTH);
    ui.push_id(size, |ui| {
        let (rect, response) = ui.allocate_exact_size(frame_size, Sense::click());
        paint_power_toggle_response(
            ui,
            "power-toggle",
            rect,
            active,
            response,
            PowerToggleOrientation::Vertical,
            tokens,
        )
    })
    .inner
}

pub fn power_toggle_rect_with_tokens(
    ui: &mut Ui,
    id_salt: impl Hash + std::fmt::Debug,
    rect: Rect,
    active: &mut bool,
    tokens: &WidgetTokens,
) -> Response {
    power_toggle_rect_oriented_with_tokens(
        ui,
        id_salt,
        rect,
        active,
        PowerToggleOrientation::Vertical,
        tokens,
    )
}

pub fn power_toggle_rect_oriented_with_tokens(
    ui: &mut Ui,
    id_salt: impl Hash + std::fmt::Debug,
    rect: Rect,
    active: &mut bool,
    orientation: PowerToggleOrientation,
    tokens: &WidgetTokens,
) -> Response {
    ui.push_id(id_salt, |ui| {
        let response = ui
            .interact(rect, ui.id().with("power-toggle-hit"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        paint_power_toggle_response(
            ui,
            "power-toggle",
            rect,
            active,
            response,
            orientation,
            tokens,
        )
    })
    .inner
}

fn paint_power_toggle_response(
    ui: &mut Ui,
    animation_id: impl Hash + std::fmt::Debug,
    rect: Rect,
    active: &mut bool,
    mut response: Response,
    orientation: PowerToggleOrientation,
    tokens: &WidgetTokens,
) -> Response {
    if response.clicked() {
        *active = !*active;
        response.mark_changed();
    }

    let t = ui.ctx().animate_bool_with_time_and_easing(
        ui.id().with(animation_id),
        *active,
        ANIM_TIME_SECS,
        ease_rocker,
    );
    let rocker_angle = -((t * 2.0 - 1.0) * ROCKER_ANGLE_DEG.to_radians());
    let fitted = fit_power_toggle_rect(rect, orientation);

    let painter = ui.painter_at(rect);
    paint_power_toggle(
        &painter,
        fitted,
        TogglePaintState {
            anim_t: t,
            rocker_angle,
            hovered: response.hovered(),
            orientation,
        },
        tokens,
    );
    response
}

fn fit_power_toggle_rect(rect: Rect, orientation: PowerToggleOrientation) -> Rect {
    let aspect = height_per_width(orientation);
    let width = rect.width().min(rect.height() / aspect).max(1.0);
    let size = Vec2::new(width, width * aspect);
    Rect::from_center_size(rect.center(), size)
}

fn height_per_width(orientation: PowerToggleOrientation) -> f32 {
    match orientation {
        PowerToggleOrientation::Vertical => VERTICAL_HEIGHT_PER_WIDTH,
        PowerToggleOrientation::Horizontal => HORIZONTAL_HEIGHT_PER_WIDTH,
    }
}

fn paint_power_toggle(
    painter: &egui::Painter,
    rect: Rect,
    state: TogglePaintState,
    tokens: &WidgetTokens,
) {
    let colors = tokens.colors;
    let lamp = lamp_color(tokens, state.anim_t);
    let frame_rounding = tokens.radius.panel;
    let reference_width = reference_length(rect, state.orientation);

    paint_outer_switch_glow(painter, rect, lamp, state.anim_t, reference_width);
    paint_bezel_shell(painter, rect, frame_rounding, reference_width, tokens);

    let plate = rect.shrink(reference_width * PADDING_FRAC + tokens.stroke.control);
    paint_plate_recess(painter, plate, frame_rounding, reference_width, tokens);

    let rocker_rect = plate.shrink2(Vec2::new(plate.width() * 0.045, plate.height() * 0.035));
    let rocker_center = rocker_rect.center();
    let rocker_w = rocker_rect.width();
    let rocker_h = rocker_rect.height();
    let pivot_z = reference_width * PIVOT_Z_FRAC;
    let perspective = reference_width * PERSPECTIVE_FRAC;
    let depth = reference_width * EXTRUSION_FRAC;

    let faces = build_rocker_faces(RockerBuildParams {
        width: rocker_w,
        height: rocker_h,
        depth,
        pivot_z,
        perspective,
        screen_origin: rocker_center,
        angle: state.rocker_angle,
        orientation: state.orientation,
        lamp,
        white: colors.white,
        shadow: colors.shadow,
    });

    let front_corners = faces
        .iter()
        .find(|face| matches!(face.kind, FaceKind::Front))
        .map(|face| face.corners);

    paint_bottom_cast_shadow(painter, &faces, state.anim_t, reference_width);

    let mut sorted = faces;
    sorted.sort_by(|a, b| {
        a.avg_z
            .partial_cmp(&b.avg_z)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for face in &sorted {
        if !matches!(face.kind, FaceKind::Back) {
            paint_smooth_vertical_gradient_quad(
                painter,
                face.corners,
                face.top,
                face.middle,
                face.bottom,
            );
        }
    }

    if let Some(front) = front_corners {
        paint_rocker_pose_glow(
            painter,
            front,
            lamp,
            state.anim_t,
            state.hovered,
            reference_width,
            state.orientation,
        );
        paint_radial_lamp(painter, front, lamp, state.anim_t, reference_width);
        paint_dot_texture(painter, front, lamp, reference_width, tokens);
        paint_power_symbol(
            painter,
            front,
            state.anim_t,
            reference_width,
            state.orientation,
            tokens,
        );
        paint_protrusion_shine(
            painter,
            front,
            state.anim_t,
            colors.white,
            state.orientation,
        );
        paint_recess_black_mask(painter, front, state.anim_t, state.orientation);

        if state.hovered {
            painter.add(Shape::closed_line(
                front.to_vec(),
                Stroke::new(tokens.stroke.control, with_alpha(colors.white, 88)),
            ));
        }
    }
}

fn reference_length(rect: Rect, orientation: PowerToggleOrientation) -> f32 {
    match orientation {
        PowerToggleOrientation::Vertical => rect.width(),
        PowerToggleOrientation::Horizontal => rect.height(),
    }
}

fn paint_outer_switch_glow(
    painter: &egui::Painter,
    rect: Rect,
    lamp: Color32,
    anim_t: f32,
    reference_width: f32,
) {
    if anim_t < 0.08 {
        return;
    }
    let strength = anim_t.clamp(0.0, 1.0);
    // CSS `box-shadow: 0 -10px 20px` at 150px reference width.
    let upward = reference_width * (10.0 / 150.0);
    let blur = reference_width * (20.0 / 150.0);
    let glow_rect = rect
        .expand(reference_width * 0.05)
        .translate(Vec2::new(0.0, -upward));
    let alpha = (strength * 100.0) as u8;
    painter.add(
        egui::epaint::RectShape::filled(glow_rect, 6.0, with_alpha(lamp, alpha))
            .with_blur_width(blur),
    );
    painter.add(
        egui::epaint::RectShape::filled(
            glow_rect.translate(Vec2::new(0.0, -upward * 0.45)),
            4.0,
            with_alpha(lamp, (alpha * 2 / 3).max(32)),
        )
        .with_blur_width(blur * 1.35),
    );
}

fn paint_bezel_shell(
    painter: &egui::Painter,
    rect: Rect,
    rounding: u8,
    reference_width: f32,
    tokens: &WidgetTokens,
) {
    let colors = tokens.colors;
    let rounding = rounding as f32;
    let shell = lerp_color(colors.surface_low, colors.surface_dark, 0.42);

    painter.add(Shape::Rect(
        egui::epaint::Shadow {
            offset: [0, 0],
            blur: (reference_width * 0.067).round().max(6.0) as u8,
            spread: 2,
            color: with_alpha(Color32::BLACK, 51),
        }
        .as_shape(rect, rounding as u8),
    ));
    painter.add(Shape::Rect(
        egui::epaint::Shadow {
            offset: [0, 0],
            blur: 2,
            spread: 2,
            color: Color32::BLACK,
        }
        .as_shape(rect, rounding as u8),
    ));

    painter.rect_filled(rect, rounding, shell);
    painter.rect_stroke(
        rect.shrink(0.5),
        rounding,
        Stroke::new(tokens.stroke.control, with_alpha(colors.text, 190)),
        egui::StrokeKind::Inside,
    );

    let inner = rect.shrink(reference_width * 0.050);
    painter.rect_filled(inner, rounding * 0.75, Color32::BLACK);
    painter.rect_stroke(
        inner.shrink(0.5),
        rounding * 0.72,
        Stroke::new(1.0_f32, with_alpha(Color32::BLACK, 220)),
        egui::StrokeKind::Inside,
    );

    painter.add(
        egui::epaint::RectShape::filled(
            Rect::from_min_max(
                inner.left_top(),
                Pos2::new(inner.right(), inner.top() + inner.height() * 0.06),
            ),
            rounding * 0.5,
            with_alpha(colors.white, 72),
        )
        .with_blur_width(reference_width * 0.022),
    );

    painter.add(
        egui::epaint::RectShape::filled(inner, rounding * 0.7, with_alpha(Color32::BLACK, 118))
            .with_blur_width(reference_width * 0.022),
    );
}

fn paint_plate_recess(
    painter: &egui::Painter,
    plate: Rect,
    rounding: u8,
    reference_width: f32,
    tokens: &WidgetTokens,
) {
    let colors = tokens.colors;
    let recess = lerp_color(colors.surface_dark, Color32::BLACK, 0.82);
    painter.rect_filled(plate, rounding as f32, recess);
    painter.add(
        egui::epaint::RectShape::filled(
            Rect::from_min_max(
                plate.left_top(),
                Pos2::new(plate.right(), plate.top() + plate.height() * 0.08),
            ),
            rounding as f32 * 0.6,
            with_alpha(colors.white, 36),
        )
        .with_blur_width(reference_width * 0.016),
    );
    painter.add(
        egui::epaint::RectShape::filled(
            Rect::from_min_max(
                Pos2::new(plate.left(), plate.bottom() - plate.height() * 0.22),
                plate.right_bottom(),
            ),
            rounding as f32 * 0.5,
            with_alpha(Color32::BLACK, 140),
        )
        .with_blur_width(reference_width * 0.028),
    );
}

fn build_rocker_faces(params: RockerBuildParams) -> [RockerFace; 6] {
    let RockerBuildParams {
        width,
        height,
        depth,
        pivot_z,
        perspective,
        screen_origin,
        angle,
        orientation,
        lamp,
        white,
        shadow,
    } = params;
    let hw = width * 0.5;
    let hh = height * 0.5;

    let local = [
        Vec3 {
            x: -hw,
            y: -hh,
            z: 0.0,
        },
        Vec3 {
            x: hw,
            y: -hh,
            z: 0.0,
        },
        Vec3 {
            x: hw,
            y: hh,
            z: 0.0,
        },
        Vec3 {
            x: -hw,
            y: hh,
            z: 0.0,
        },
        Vec3 {
            x: -hw,
            y: -hh,
            z: -depth,
        },
        Vec3 {
            x: hw,
            y: -hh,
            z: -depth,
        },
        Vec3 {
            x: hw,
            y: hh,
            z: -depth,
        },
        Vec3 {
            x: -hw,
            y: hh,
            z: -depth,
        },
    ];

    let mut projected = [Pos2::ZERO; 8];
    let mut transformed_z = [0.0_f32; 8];
    for (index, point) in local.iter().enumerate() {
        let transformed = transform_rocker_point(*point, pivot_z, angle, orientation);
        transformed_z[index] = transformed.z;
        projected[index] = project_point(transformed, perspective, screen_origin);
    }

    let face_specs: [(usize, usize, usize, usize, FaceKind); 6] = [
        (0, 1, 2, 3, FaceKind::Front),
        (5, 4, 7, 6, FaceKind::Back),
        (0, 4, 5, 1, FaceKind::Top),
        (3, 2, 6, 7, FaceKind::Bottom),
        (0, 3, 7, 4, FaceKind::Left),
        (1, 5, 6, 2, FaceKind::Right),
    ];

    let mut faces = [RockerFace {
        kind: FaceKind::Front,
        avg_z: 0.0,
        corners: [Pos2::ZERO; 4],
        top: Color32::BLACK,
        middle: Color32::BLACK,
        bottom: Color32::BLACK,
    }; 6];

    for (face_index, (i0, i1, i2, i3, kind)) in face_specs.into_iter().enumerate() {
        let indices = [i0, i1, i2, i3];
        let corners = [projected[i0], projected[i1], projected[i2], projected[i3]];
        let avg_z = indices.iter().map(|&i| transformed_z[i]).sum::<f32>() / 4.0;
        let (top, middle, bottom) = face_colors(kind, lamp, white, shadow);
        faces[face_index] = RockerFace {
            kind,
            avg_z,
            corners,
            top,
            middle,
            bottom,
        };
    }

    faces
}

fn face_colors(
    kind: FaceKind,
    lamp: Color32,
    white: Color32,
    shadow: Color32,
) -> (Color32, Color32, Color32) {
    match kind {
        FaceKind::Front => (
            lerp_color(lamp, white, 0.26),
            darken(lamp, 0.06),
            darken(lamp, 0.28),
        ),
        FaceKind::Top => (
            lerp_color(white, lamp, 0.14),
            lerp_color(lamp, white, 0.30),
            darken(lamp, 0.12),
        ),
        FaceKind::Bottom => (darken(lamp, 0.30), darken(lamp, 0.42), darken(lamp, 0.58)),
        FaceKind::Back => (
            darken(lamp, 0.45),
            darken(shadow, 0.25),
            darken(shadow, 0.40),
        ),
        FaceKind::Left | FaceKind::Right => {
            (darken(lamp, 0.24), darken(lamp, 0.36), darken(lamp, 0.48))
        }
    }
}

fn transform_rocker_point(
    point: Vec3,
    pivot_z: f32,
    angle: f32,
    orientation: PowerToggleOrientation,
) -> Vec3 {
    let mut p = point;
    p.z += pivot_z;
    p = match orientation {
        PowerToggleOrientation::Vertical => rotate_x(p, angle),
        PowerToggleOrientation::Horizontal => rotate_y(p, angle),
    };
    p.z -= pivot_z;
    p
}

fn rotate_x(point: Vec3, angle: f32) -> Vec3 {
    let cos = angle.cos();
    let sin = angle.sin();
    Vec3 {
        x: point.x,
        y: point.y * cos + point.z * sin,
        z: -point.y * sin + point.z * cos,
    }
}

fn rotate_y(point: Vec3, angle: f32) -> Vec3 {
    let cos = angle.cos();
    let sin = angle.sin();
    Vec3 {
        x: point.x * cos - point.z * sin,
        y: point.y,
        z: point.x * sin + point.z * cos,
    }
}

fn project_point(point: Vec3, perspective: f32, origin: Pos2) -> Pos2 {
    let denom = (perspective - point.z).max(1.0);
    let scale = perspective / denom;
    Pos2::new(origin.x + point.x * scale, origin.y + point.y * scale)
}

fn paint_bottom_cast_shadow(
    painter: &egui::Painter,
    faces: &[RockerFace; 6],
    anim_t: f32,
    reference_width: f32,
) {
    let Some(bottom) = faces
        .iter()
        .find(|face| matches!(face.kind, FaceKind::Bottom))
    else {
        return;
    };

    let max_y = bottom
        .corners
        .iter()
        .map(|point| point.y)
        .fold(f32::NEG_INFINITY, f32::max);
    let min_x = bottom
        .corners
        .iter()
        .map(|point| point.x)
        .fold(f32::INFINITY, f32::min);
    let max_x = bottom
        .corners
        .iter()
        .map(|point| point.x)
        .fold(f32::NEG_INFINITY, f32::max);

    let shadow_alpha = ((1.0 - anim_t) * 130.0) as u8;
    let shadow_rect = Rect::from_min_max(
        Pos2::new(
            min_x - reference_width * 0.02,
            max_y + reference_width * 0.01,
        ),
        Pos2::new(
            max_x + reference_width * 0.02,
            max_y + reference_width * 0.28,
        ),
    );
    painter.add(
        egui::epaint::RectShape::filled(shadow_rect, 2.0, with_alpha(Color32::BLACK, shadow_alpha))
            .with_blur_width(reference_width * 0.09),
    );
    painter.add(
        egui::epaint::RectShape::filled(
            shadow_rect.translate(Vec2::new(0.0, reference_width * 0.07)),
            4.0,
            with_alpha(
                Color32::BLACK,
                (u16::from(shadow_alpha) * 2 / 5).max(24) as u8,
            ),
        )
        .with_blur_width(reference_width * 0.17),
    );
}

fn paint_rocker_pose_glow(
    painter: &egui::Painter,
    front: [Pos2; 4],
    lamp: Color32,
    anim_t: f32,
    hovered: bool,
    reference_width: f32,
    orientation: PowerToggleOrientation,
) {
    let pose_t = pose_anim_t(anim_t, orientation);
    let strength = if pose_t >= 0.5 {
        pose_t.clamp(0.0, 1.0)
    } else {
        (1.0 - pose_t).clamp(0.0, 1.0)
    };
    if strength < 0.12 {
        return;
    }
    let alpha = (strength * if hovered { 96.0 } else { 78.0 }) as u8;
    let glow_rect = match orientation {
        PowerToggleOrientation::Vertical => vertical_pose_glow_rect(front, pose_t, reference_width),
        PowerToggleOrientation::Horizontal => {
            horizontal_pose_glow_rect(front, pose_t, reference_width)
        }
    };
    painter.add(
        egui::epaint::RectShape::filled(glow_rect, 3.0, with_alpha(lamp, alpha))
            .with_blur_width(reference_width * 0.13),
    );
}

fn vertical_pose_glow_rect(front: [Pos2; 4], anim_t: f32, reference_width: f32) -> Rect {
    let protrudes_top = anim_t < 0.5;
    let edge_y = if protrudes_top {
        front
            .iter()
            .map(|point| point.y)
            .fold(f32::INFINITY, f32::min)
    } else {
        front
            .iter()
            .map(|point| point.y)
            .fold(f32::NEG_INFINITY, f32::max)
    };
    let min_x = front
        .iter()
        .map(|point| point.x)
        .fold(f32::INFINITY, f32::min);
    let max_x = front
        .iter()
        .map(|point| point.x)
        .fold(f32::NEG_INFINITY, f32::max);
    if protrudes_top {
        Rect::from_min_max(
            Pos2::new(
                min_x - reference_width * 0.08,
                edge_y - reference_width * 0.22,
            ),
            Pos2::new(
                max_x + reference_width * 0.08,
                edge_y + reference_width * 0.02,
            ),
        )
    } else {
        Rect::from_min_max(
            Pos2::new(
                min_x - reference_width * 0.08,
                edge_y - reference_width * 0.02,
            ),
            Pos2::new(
                max_x + reference_width * 0.08,
                edge_y + reference_width * 0.22,
            ),
        )
    }
}

fn horizontal_pose_glow_rect(front: [Pos2; 4], anim_t: f32, reference_width: f32) -> Rect {
    let protrudes_left = anim_t < 0.5;
    let edge_x = if protrudes_left {
        front
            .iter()
            .map(|point| point.x)
            .fold(f32::INFINITY, f32::min)
    } else {
        front
            .iter()
            .map(|point| point.x)
            .fold(f32::NEG_INFINITY, f32::max)
    };
    let min_y = front
        .iter()
        .map(|point| point.y)
        .fold(f32::INFINITY, f32::min);
    let max_y = front
        .iter()
        .map(|point| point.y)
        .fold(f32::NEG_INFINITY, f32::max);
    if protrudes_left {
        Rect::from_min_max(
            Pos2::new(
                edge_x - reference_width * 0.22,
                min_y - reference_width * 0.08,
            ),
            Pos2::new(
                edge_x + reference_width * 0.02,
                max_y + reference_width * 0.08,
            ),
        )
    } else {
        Rect::from_min_max(
            Pos2::new(
                edge_x - reference_width * 0.02,
                min_y - reference_width * 0.08,
            ),
            Pos2::new(
                edge_x + reference_width * 0.22,
                max_y + reference_width * 0.08,
            ),
        )
    }
}

fn paint_radial_lamp(
    painter: &egui::Painter,
    front: [Pos2; 4],
    lamp: Color32,
    anim_t: f32,
    reference_width: f32,
) {
    if anim_t < 0.05 {
        return;
    }
    let center = quad_center(front);
    let lamp_alpha = (anim_t * 200.0) as u8;
    let inner = lerp_color(lamp, Color32::WHITE, 0.28);
    for (radius_frac, color, alpha_scale) in [
        (0.50, inner, 0.42_f32),
        (0.36, lamp, 0.62),
        (0.22, inner, 0.32),
    ] {
        let radius = reference_width * radius_frac * 0.34;
        let rect = Rect::from_center_size(center, Vec2::splat(radius * 2.0));
        let alpha = (f32::from(lamp_alpha) * alpha_scale) as u8;
        painter.add(
            egui::epaint::RectShape::filled(rect, radius, with_alpha(color, alpha))
                .with_blur_width(radius * 0.9),
        );
    }
}

fn paint_dot_texture(
    painter: &egui::Painter,
    front: [Pos2; 4],
    lamp: Color32,
    reference_width: f32,
    tokens: &WidgetTokens,
) {
    let step = (reference_width * 0.055).max(tokens.spacing.xs);
    let radius = (step * 0.16).max(0.55);
    let bounds = quad_bounds(front);
    let mut y = bounds.top() + step * 0.5;
    while y < bounds.bottom() {
        let mut x = bounds.left() + step * 0.5;
        while x < bounds.right() {
            let point = Pos2::new(x, y);
            if point_in_quad(point, &front) {
                painter.circle_filled(
                    point,
                    radius,
                    with_alpha(lerp_color(lamp, Color32::BLACK, 0.22), 54),
                );
            }
            x += step;
        }
        y += step;
    }
}

fn paint_power_symbol(
    painter: &egui::Painter,
    front: [Pos2; 4],
    anim_t: f32,
    reference_width: f32,
    orientation: PowerToggleOrientation,
    tokens: &WidgetTokens,
) {
    let bounds = quad_bounds(front);
    let alpha = lerp_f32(188.0, 232.0, anim_t) as u8;
    let stroke = Stroke::new(
        (reference_width * 0.033).max(tokens.stroke.control),
        with_alpha(tokens.colors.white, alpha),
    );
    match orientation {
        PowerToggleOrientation::Vertical => {
            let center_x = bounds.center().x;
            painter.line_segment(
                [
                    Pos2::new(center_x, bounds.top() + bounds.height() * 0.18),
                    Pos2::new(center_x, bounds.top() + bounds.height() * 0.34),
                ],
                stroke,
            );
            painter.circle_stroke(
                Pos2::new(center_x, bounds.top() + bounds.height() * 0.75),
                reference_width * 0.102,
                stroke,
            );
        }
        PowerToggleOrientation::Horizontal => {
            let center_y = bounds.center().y;
            let line_x = bounds.left() + bounds.width() * 0.33;
            painter.line_segment(
                [
                    Pos2::new(line_x, center_y - bounds.height() * 0.18),
                    Pos2::new(line_x, center_y + bounds.height() * 0.18),
                ],
                stroke,
            );
            painter.circle_stroke(
                Pos2::new(bounds.left() + bounds.width() * 0.70, center_y),
                reference_width * 0.102,
                stroke,
            );
        }
    }
}

fn paint_protrusion_shine(
    painter: &egui::Painter,
    front: [Pos2; 4],
    anim_t: f32,
    white: Color32,
    orientation: PowerToggleOrientation,
) {
    let pose_t = pose_anim_t(anim_t, orientation);
    let protrudes_start = pose_t < 0.5;
    let (start_frac, end_frac, peak_alpha) = if protrudes_start {
        (0.0, 0.34, lerp_f32(200.0, 92.0, pose_t * 2.0))
    } else {
        (0.66, 1.0, lerp_f32(92.0, 200.0, (pose_t - 0.5) * 2.0))
    };
    paint_face_gradient_band(
        painter,
        front,
        start_frac,
        end_frac,
        with_alpha(white, peak_alpha as u8),
        Color32::TRANSPARENT,
        orientation,
    );
}

/// CSS-style black mask on the recessed rocker half (true mesh gradient, not a flat overlay).
fn paint_recess_black_mask(
    painter: &egui::Painter,
    front: [Pos2; 4],
    anim_t: f32,
    orientation: PowerToggleOrientation,
) {
    let pose_t = pose_anim_t(anim_t, orientation);
    let on_strength = pose_t.clamp(0.0, 1.0);
    let off_strength = 1.0 - on_strength;
    let mask_alpha = if matches!(orientation, PowerToggleOrientation::Horizontal) {
        92.0
    } else {
        118.0
    };
    if on_strength > 0.04 {
        paint_face_gradient_band(
            painter,
            front,
            0.0,
            0.58,
            with_alpha(Color32::BLACK, (on_strength * mask_alpha) as u8),
            Color32::TRANSPARENT,
            orientation,
        );
    }
    if off_strength > 0.04 {
        paint_face_gradient_band(
            painter,
            front,
            0.42,
            1.0,
            Color32::TRANSPARENT,
            with_alpha(Color32::BLACK, (off_strength * mask_alpha) as u8),
            orientation,
        );
    }
}

fn pose_anim_t(anim_t: f32, orientation: PowerToggleOrientation) -> f32 {
    match orientation {
        PowerToggleOrientation::Vertical => anim_t,
        PowerToggleOrientation::Horizontal => 1.0 - anim_t,
    }
}

fn paint_face_gradient_band(
    painter: &egui::Painter,
    front: [Pos2; 4],
    start_frac: f32,
    end_frac: f32,
    edge_color: Color32,
    inner_color: Color32,
    orientation: PowerToggleOrientation,
) {
    if edge_color.a() == 0 && inner_color.a() == 0 {
        return;
    }
    let rows = GRADIENT_ROWS;
    let mut mesh = Mesh::default();
    for row in 0..rows {
        let local_t0 = row as f32 / rows as f32;
        let local_t1 = (row + 1) as f32 / rows as f32;
        let t0 = lerp_f32(start_frac, end_frac, local_t0);
        let t1 = lerp_f32(start_frac, end_frac, local_t1);
        let (left0, right0, left1, right1) = match orientation {
            PowerToggleOrientation::Vertical => (
                lerp_pos(front[0], front[3], t0),
                lerp_pos(front[1], front[2], t0),
                lerp_pos(front[0], front[3], t1),
                lerp_pos(front[1], front[2], t1),
            ),
            PowerToggleOrientation::Horizontal => (
                lerp_pos(front[0], front[1], t0),
                lerp_pos(front[3], front[2], t0),
                lerp_pos(front[0], front[1], t1),
                lerp_pos(front[3], front[2], t1),
            ),
        };
        let c0 = lerp_color(edge_color, inner_color, local_t0);
        let c1 = lerp_color(edge_color, inner_color, local_t1);
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(left0, c0);
        mesh.colored_vertex(right0, c0);
        mesh.colored_vertex(right1, c1);
        mesh.colored_vertex(left1, c1);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    painter.add(Shape::mesh(mesh));
}

fn paint_smooth_vertical_gradient_quad(
    painter: &egui::Painter,
    corners: [Pos2; 4],
    top: Color32,
    middle: Color32,
    bottom: Color32,
) {
    let color_at = |t: f32| {
        if t < 0.5 {
            lerp_color(top, middle, t * 2.0)
        } else {
            lerp_color(middle, bottom, (t - 0.5) * 2.0)
        }
    };

    let mut mesh = Mesh::default();
    let rows = GRADIENT_ROWS;
    for row in 0..rows {
        let t0 = row as f32 / rows as f32;
        let t1 = (row + 1) as f32 / rows as f32;
        let left0 = lerp_pos(corners[0], corners[3], t0);
        let right0 = lerp_pos(corners[1], corners[2], t0);
        let left1 = lerp_pos(corners[0], corners[3], t1);
        let right1 = lerp_pos(corners[1], corners[2], t1);
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(left0, color_at(t0));
        mesh.colored_vertex(right0, color_at(t0));
        mesh.colored_vertex(right1, color_at(t1));
        mesh.colored_vertex(left1, color_at(t1));
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    painter.add(Shape::mesh(mesh));
}

fn lamp_color(tokens: &WidgetTokens, anim_t: f32) -> Color32 {
    let on = tokens.colors.accent;
    let off = lerp_color(tokens.colors.warning, Color32::BLACK, 0.06);
    lerp_color(off, on, anim_t.clamp(0.0, 1.0))
}

fn quad_center(points: [Pos2; 4]) -> Pos2 {
    Pos2::new(
        (points[0].x + points[1].x + points[2].x + points[3].x) * 0.25,
        (points[0].y + points[1].y + points[2].y + points[3].y) * 0.25,
    )
}

fn quad_bounds(points: [Pos2; 4]) -> Rect {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for point in points {
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    }
    Rect::from_min_max(Pos2::new(min_x, min_y), Pos2::new(max_x, max_y))
}

fn point_in_quad(point: Pos2, quad: &[Pos2; 4]) -> bool {
    let mut sign = 0.0_f32;
    for index in 0..4 {
        let a = quad[index];
        let b = quad[(index + 1) % 4];
        let cross = (b.x - a.x) * (point.y - a.y) - (b.y - a.y) * (point.x - a.x);
        if cross.abs() < f32::EPSILON {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if sign != cross.signum() {
            return false;
        }
    }
    true
}

fn lerp_pos(a: Pos2, b: Pos2, t: f32) -> Pos2 {
    a + (b - a) * t
}

fn lerp_f32(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t.clamp(0.0, 1.0)
}

fn darken(color: Color32, amount: f32) -> Color32 {
    lerp_color(color, Color32::BLACK, amount.clamp(0.0, 1.0))
}

/// Smooth snap without geometry jumps during pointer clicks.
fn ease_rocker(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - t).powi(4);
    eased * eased * (3.0 - 2.0 * eased)
}
