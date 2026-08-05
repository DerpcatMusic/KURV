use crate::tokens::{LIGHT_TOKENS, WidgetColors, WidgetTokens, lerp_color, with_alpha};
use egui::{Color32, Mesh, Pos2, Rect, Response, Shape, Stroke, Ui, Vec2};

/// Dark cap radius as a fraction of the caller's `body_size` parameter.
const CAP_RADIUS_FRAC: f32 = 0.34;
const RING_WIDTH_FRAC: f32 = 0.062;
const ARC_GAP_FRAC: f32 = 0.112;
const ARC_TRACK_WIDTH_FRAC: f32 = 0.021;
const ARC_VALUE_WIDTH_FRAC: f32 = 0.033;
const LAYOUT_MARGIN_FRAC: f32 = 0.054;
const GLOW_EXPAND_FRAC: f32 = 0.074;
const GLOW_DROP_FRAC: f32 = 0.034;
const GLOW_MID_DROP_FRAC: f32 = 0.018;
const GLOW_BLUR_FRAC: f32 = 0.07;
const RING_OUTER_SHADOW_EXPAND_FRAC: f32 = 0.034;
const RING_OUTER_SHADOW_BLUR_FRAC: f32 = 0.052;
const RING_OUTER_SHADOW_ALPHA: u8 = 38;
const CAP_SHADOW_EXPAND_FRAC: f32 = 0.035;
const CAP_SHADOW_BLUR_FRAC: f32 = 0.036;
const CAP_SHADOW_ALPHA: u8 = 44;
const GLOW_OUTER_ALPHA: u8 = 22;
const GLOW_MID_ALPHA: u8 = 28;
const GLOW_INNER_ALPHA: u8 = 24;
const CAP_GRADIENT_STRENGTH: f32 = 0.28;
const CAP_RIM_WIDTH_FRAC: f32 = 0.024;
const CAP_RIM_ALPHA: u8 = 34;
const INDICATOR_INNER_FRAC: f32 = 0.60;
const INDICATOR_OUTER_FRAC: f32 = 0.78;
const INDICATOR_WIDTH_FRAC: f32 = 0.09;
const ARC_MARKER_RADIUS_FRAC: f32 = 0.022;
const ARC_VALUE_MARKER_RADIUS_FRAC: f32 = 0.045;
const ARC_VALUE_MARKER_STROKE_FRAC: f32 = 0.012;
const ARC_VALUE_MARKER_SHADOW_ALPHA: u8 = 42;
const ARC_VALUE_MARKER_SHADOW_BLUR_FRAC: f32 = 0.018;
const DIGITAL_SCREEN_WIDTH_FRAC: f32 = 1.16;
const DIGITAL_SCREEN_WIDE_WIDTH_FRAC: f32 = 1.58;
const DIGITAL_SCREEN_HEIGHT_FRAC: f32 = 0.58;
const DIGITAL_SCREEN_Y_FRAC: f32 = 0.02;
const DIGITAL_CELL_GAP_FRAC: f32 = 0.055;
const DIGITAL_SEGMENT_THICKNESS_FRAC: f32 = 0.155;
const DIGITAL_SEGMENT_INSET_FRAC: f32 = 0.035;
const DIGITAL_SEGMENT_ACTIVE_ALPHA: u8 = 246;
const DIGITAL_SEGMENT_GLOW_ALPHA: u8 = 28;
const DIGITAL_DECIMAL_RADIUS_FRAC: f32 = 0.115;
const FIELD_EDGE_INSET_FRAC: f32 = 0.031;
const READOUT_CLEARANCE_FRAC: f32 = 0.055;
const CAP_MESH_SEGMENTS: usize = 96;
const ARC_SEGMENTS: usize = 72;

#[inline]
fn body_metric(body_size: f32, frac: f32) -> f32 {
    body_size * frac
}

/// Total square size to allocate for a knob whose body diameter is `body_size`.
pub fn tactile_knob_layout_size(body_size: f32) -> f32 {
    let cap_radius = body_metric(body_size, CAP_RADIUS_FRAC);
    let ring_outer = cap_radius + body_metric(body_size, RING_WIDTH_FRAC);
    let arc_radius = ring_outer + body_metric(body_size, ARC_GAP_FRAC);
    let arc_stroke = body_metric(body_size, ARC_VALUE_WIDTH_FRAC);
    let extent = arc_radius
        + arc_stroke * 0.5
        + body_metric(body_size, GLOW_EXPAND_FRAC)
        + body_metric(body_size, GLOW_BLUR_FRAC) * 0.5
        + body_metric(body_size, LAYOUT_MARGIN_FRAC);
    extent * 2.0
}

/// Horizontal inset from a field edge when right-aligning a knob slot.
pub fn tactile_knob_field_inset(knob_layout: f32) -> f32 {
    knob_layout * FIELD_EDGE_INSET_FRAC
}

/// Gap between the dB readout text anchor and the knob layout edge.
pub fn tactile_knob_readout_clearance(knob_layout: f32) -> f32 {
    knob_layout * READOUT_CLEARANCE_FRAC
}

/// 270° audio-style rotary control. Body lighting is fixed; only the indicator rotates.
///
/// Set `bipolar` to show the top arc marker and reset to center on double-click.
pub fn tactile_knob(ui: &mut Ui, value01: &mut f32, body_size: f32, bipolar: bool) -> Response {
    tactile_knob_with_tokens(ui, value01, body_size, bipolar, &LIGHT_TOKENS)
}

pub fn tactile_knob_with_tokens(
    ui: &mut Ui,
    value01: &mut f32,
    body_size: f32,
    bipolar: bool,
    tokens: &WidgetTokens,
) -> Response {
    tactile_knob_impl(ui, value01, body_size, bipolar, tokens, None)
}

pub fn tactile_knob_display(
    ui: &mut Ui,
    value01: &mut f32,
    body_size: f32,
    bipolar: bool,
    display_text: &str,
) -> Response {
    tactile_knob_display_with_tokens(ui, value01, body_size, bipolar, display_text, &LIGHT_TOKENS)
}

pub fn tactile_knob_display_with_tokens(
    ui: &mut Ui,
    value01: &mut f32,
    body_size: f32,
    bipolar: bool,
    display_text: &str,
    tokens: &WidgetTokens,
) -> Response {
    tactile_knob_impl(
        ui,
        value01,
        body_size,
        bipolar,
        tokens,
        Some(DisplayText {
            text: display_text,
            cells: 2,
            width_frac: DIGITAL_SCREEN_WIDTH_FRAC,
        }),
    )
}

pub fn tactile_knob_display_wide_with_tokens(
    ui: &mut Ui,
    value01: &mut f32,
    body_size: f32,
    bipolar: bool,
    display_text: &str,
    tokens: &WidgetTokens,
) -> Response {
    tactile_knob_impl(
        ui,
        value01,
        body_size,
        bipolar,
        tokens,
        Some(DisplayText {
            text: display_text,
            cells: 4,
            width_frac: DIGITAL_SCREEN_WIDE_WIDTH_FRAC,
        }),
    )
}

#[derive(Clone, Copy)]
struct DisplayText<'a> {
    text: &'a str,
    cells: usize,
    width_frac: f32,
}

fn tactile_knob_impl(
    ui: &mut Ui,
    value01: &mut f32,
    body_size: f32,
    bipolar: bool,
    tokens: &WidgetTokens,
    display_text: Option<DisplayText<'_>>,
) -> Response {
    let layout = tactile_knob_layout_size(body_size);
    let (rect, mut response) =
        ui.allocate_exact_size(Vec2::splat(layout), egui::Sense::click_and_drag());

    if response.double_clicked() {
        *value01 = if bipolar { 0.5 } else { 0.0 };
        response.mark_changed();
    }

    if response.dragged() {
        let delta = -response.drag_delta().y * 0.004;
        *value01 = (*value01 + delta).clamp(0.0, 1.0);
        response.mark_changed();
    }

    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

    let painter = ui.painter_at(rect);
    let center = rect.center();
    let cap_radius = body_metric(body_size, CAP_RADIUS_FRAC);
    let ring_outer = cap_radius + body_metric(body_size, RING_WIDTH_FRAC);
    let arc_radius = ring_outer + body_metric(body_size, ARC_GAP_FRAC);
    let arc_track_width = body_metric(body_size, ARC_TRACK_WIDTH_FRAC);
    let arc_value_width = body_metric(body_size, ARC_VALUE_WIDTH_FRAC);
    let value01 = value01.clamp(0.0, 1.0);
    let colors = tokens.colors;

    paint_assembly_glow(&painter, center, ring_outer, body_size, colors);

    let start = std::f32::consts::PI * 0.75;
    let sweep = std::f32::consts::PI * 1.5;
    let end = start + sweep;

    paint_arc_rounded(
        &painter,
        center,
        arc_radius,
        start,
        end,
        arc_track_width,
        colors.knob_arc_track,
    );
    if value01 > 0.0 {
        paint_arc_rounded(
            &painter,
            center,
            arc_radius,
            start,
            start + sweep * value01,
            arc_value_width,
            colors.knob_arc_value,
        );
    }

    if bipolar {
        let top_angle = -std::f32::consts::FRAC_PI_2;
        let marker_pos = center + Vec2::angled(top_angle) * arc_radius;
        let marker_radius = body_metric(body_size, ARC_MARKER_RADIUS_FRAC);
        painter.circle_filled(marker_pos, marker_radius, colors.knob_marker);
    }

    painter.circle_filled(center, ring_outer, colors.white);
    paint_ring_outer_shadow(&painter, center, ring_outer, body_size, colors);
    paint_cap(&painter, center, cap_radius, colors);
    if let Some(text) = display_text {
        paint_digital_display(&painter, center, cap_radius, text, colors);
        paint_arc_value_marker(
            &painter,
            center,
            arc_radius,
            start + sweep * value01,
            body_size,
            colors,
        );
    } else {
        paint_indicator(
            &painter,
            center,
            cap_radius,
            start + sweep * value01,
            colors,
        );
    }

    response
}

pub fn tactile_knob_db(
    ui: &mut Ui,
    db: &mut f32,
    min_db: f32,
    max_db: f32,
    body_size: f32,
) -> Response {
    tactile_knob_db_with_tokens(ui, db, min_db, max_db, body_size, &LIGHT_TOKENS)
}

pub fn tactile_knob_db_with_tokens(
    ui: &mut Ui,
    db: &mut f32,
    min_db: f32,
    max_db: f32,
    body_size: f32,
    tokens: &WidgetTokens,
) -> Response {
    let span = (max_db - min_db).max(f32::EPSILON);
    let mut value01 = ((*db - min_db) / span).clamp(0.0, 1.0);
    let response = tactile_knob_with_tokens(ui, &mut value01, body_size, true, tokens);
    if response.changed() {
        *db = min_db + value01 * span;
    }
    response
}

fn paint_assembly_glow(
    painter: &egui::Painter,
    center: Pos2,
    ring_outer: f32,
    body_size: f32,
    colors: WidgetColors,
) {
    let expand = body_metric(body_size, GLOW_EXPAND_FRAC);
    let drop = body_metric(body_size, GLOW_DROP_FRAC);
    let mid_drop = body_metric(body_size, GLOW_MID_DROP_FRAC);
    let blur = body_metric(body_size, GLOW_BLUR_FRAC);

    paint_blurred_disc(
        painter,
        center + Vec2::new(0.0, drop),
        ring_outer + expand,
        with_alpha(colors.shadow, GLOW_OUTER_ALPHA),
        blur,
    );
    paint_blurred_disc(
        painter,
        center + Vec2::new(0.0, mid_drop),
        ring_outer + expand * 0.7,
        with_alpha(colors.shadow, GLOW_MID_ALPHA),
        blur * 0.75,
    );
    painter.circle_filled(
        center,
        ring_outer + expand * 0.4,
        with_alpha(colors.shadow, GLOW_INNER_ALPHA),
    );
}

fn paint_ring_outer_shadow(
    painter: &egui::Painter,
    center: Pos2,
    ring_outer: f32,
    body_size: f32,
    colors: WidgetColors,
) {
    let expand = body_metric(body_size, RING_OUTER_SHADOW_EXPAND_FRAC);
    let blur = body_metric(body_size, RING_OUTER_SHADOW_BLUR_FRAC);
    paint_blurred_disc(
        painter,
        center,
        ring_outer + expand,
        with_alpha(colors.shadow, RING_OUTER_SHADOW_ALPHA),
        blur,
    );
}

fn paint_cap(painter: &egui::Painter, center: Pos2, radius: f32, colors: WidgetColors) {
    paint_blurred_disc(
        painter,
        center + Vec2::new(0.0, radius * 0.035),
        radius + radius * CAP_SHADOW_EXPAND_FRAC,
        with_alpha(colors.shadow, CAP_SHADOW_ALPHA),
        radius * CAP_SHADOW_BLUR_FRAC,
    );

    let mut mesh = Mesh::default();
    let center_idx = mesh.vertices.len() as u32;
    mesh.colored_vertex(center, colors.knob_cap);

    for i in 0..=CAP_MESH_SEGMENTS {
        let angle = std::f32::consts::TAU * i as f32 / CAP_MESH_SEGMENTS as f32;
        let point = center + Vec2::angled(angle) * radius;
        let topness = ((center.y - point.y) / radius).clamp(0.0, 1.0);
        let color = lerp_color(
            colors.knob_cap,
            colors.knob_cap_highlight,
            topness * CAP_GRADIENT_STRENGTH,
        );
        mesh.colored_vertex(point, color);
    }

    for i in 0..CAP_MESH_SEGMENTS {
        mesh.add_triangle(
            center_idx,
            center_idx + i as u32 + 1,
            center_idx + i as u32 + 2,
        );
    }
    painter.add(Shape::mesh(mesh));

    let rim_width = radius * CAP_RIM_WIDTH_FRAC;
    painter.circle_stroke(
        center,
        radius + rim_width * 0.5,
        Stroke::new(rim_width, with_alpha(colors.shadow, CAP_RIM_ALPHA)),
    );
}

fn paint_blurred_disc(
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    color: Color32,
    blur_width: f32,
) {
    let rect = Rect::from_center_size(center, Vec2::splat(radius * 2.0));
    painter.add(egui::epaint::RectShape::filled(rect, radius, color).with_blur_width(blur_width));
}

fn paint_indicator(
    painter: &egui::Painter,
    center: Pos2,
    cap_radius: f32,
    angle: f32,
    colors: WidgetColors,
) {
    let dir = Vec2::angled(angle);
    let stroke_width = cap_radius * INDICATOR_WIDTH_FRAC;
    let inner = cap_radius * INDICATOR_INNER_FRAC;
    let outer = cap_radius * INDICATOR_OUTER_FRAC;
    let start = center + dir * inner;
    let end = center + dir * outer;
    let cap_r = stroke_width * 0.5;

    painter.line_segment([start, end], Stroke::new(stroke_width, colors.white));
    painter.circle_filled(start, cap_r, colors.white);
    painter.circle_filled(end, cap_r, colors.white);
}

fn paint_arc_value_marker(
    painter: &egui::Painter,
    center: Pos2,
    arc_radius: f32,
    angle: f32,
    body_size: f32,
    colors: WidgetColors,
) {
    let marker_radius = body_metric(body_size, ARC_VALUE_MARKER_RADIUS_FRAC);
    let marker_pos = center + Vec2::angled(angle) * arc_radius;
    let shadow_blur = body_metric(body_size, ARC_VALUE_MARKER_SHADOW_BLUR_FRAC);

    paint_blurred_disc(
        painter,
        marker_pos + Vec2::new(0.0, marker_radius * 0.16),
        marker_radius * 1.25,
        with_alpha(colors.shadow, ARC_VALUE_MARKER_SHADOW_ALPHA),
        shadow_blur,
    );
    paint_blurred_disc(
        painter,
        marker_pos,
        marker_radius * 1.48,
        with_alpha(colors.knob_arc_value, ARC_VALUE_MARKER_SHADOW_ALPHA),
        shadow_blur * 0.82,
    );
    painter.circle_filled(marker_pos, marker_radius, colors.knob_arc_value);
    painter.circle_stroke(
        marker_pos,
        marker_radius,
        Stroke::new(
            body_metric(body_size, ARC_VALUE_MARKER_STROKE_FRAC),
            with_alpha(colors.white, 180),
        ),
    );
}

fn paint_digital_display(
    painter: &egui::Painter,
    center: Pos2,
    cap_radius: f32,
    display_text: DisplayText<'_>,
    colors: WidgetColors,
) {
    let screen_size = Vec2::new(
        cap_radius * display_text.width_frac,
        cap_radius * DIGITAL_SCREEN_HEIGHT_FRAC,
    );
    let screen_center = center + Vec2::new(0.0, cap_radius * DIGITAL_SCREEN_Y_FRAC);
    let screen = Rect::from_center_size(screen_center, screen_size);

    let digits = display_digits(display_text.text, display_text.cells);
    let gap = screen.width() * DIGITAL_CELL_GAP_FRAC;
    let cells = digits.len().max(1) as f32;
    let cell_width = (screen.width() - gap * (cells - 1.0)) / cells;
    for (index, digit) in digits.iter().copied().enumerate() {
        let left = screen.left() + index as f32 * (cell_width + gap);
        let cell = Rect::from_min_size(
            Pos2::new(left, screen.top()),
            Vec2::new(cell_width, screen.height()),
        )
        .shrink2(Vec2::new(
            screen.width() * DIGITAL_SEGMENT_INSET_FRAC * 0.5,
            screen.height() * DIGITAL_SEGMENT_INSET_FRAC,
        ));
        paint_digit(painter, cell, digit, colors);
    }
}

#[derive(Clone, Copy)]
struct DisplayDigit {
    value: Option<u8>,
    minus: bool,
    decimal: bool,
}

fn display_digits(display_text: &str, cells: usize) -> Vec<DisplayDigit> {
    let cells = cells.clamp(1, 4);
    let mut digits = vec![
        DisplayDigit {
            value: None,
            minus: false,
            decimal: false,
        };
        cells
    ];
    let mut slot = 0;
    for byte in display_text.bytes() {
        if byte == b'.' {
            if slot > 0 {
                digits[slot - 1].decimal = true;
            }
            continue;
        }
        if byte == b'-' {
            if slot < digits.len() {
                digits[slot].minus = true;
                slot += 1;
            }
            continue;
        }
        if !byte.is_ascii_digit() {
            continue;
        }
        if slot >= digits.len() {
            break;
        }
        digits[slot].value = Some(byte - b'0');
        slot += 1;
    }
    digits
}

fn paint_digit(painter: &egui::Painter, rect: Rect, digit: DisplayDigit, colors: WidgetColors) {
    const SEGMENTS: [[bool; 7]; 10] = [
        [true, true, true, true, true, true, false],
        [false, true, true, false, false, false, false],
        [true, true, false, true, true, false, true],
        [true, true, true, true, false, false, true],
        [false, true, true, false, false, true, true],
        [true, false, true, true, false, true, true],
        [true, false, true, true, true, true, true],
        [true, true, true, false, false, false, false],
        [true, true, true, true, true, true, true],
        [true, true, true, true, false, true, true],
    ];

    if digit.minus {
        paint_digit_segment(painter, digit_segment_points(rect, 6), colors);
    } else if let Some(active) = digit.value.and_then(|digit| SEGMENTS.get(digit as usize)) {
        for index in 0..7 {
            if active[index] {
                paint_digit_segment(painter, digit_segment_points(rect, index), colors);
            }
        }
    }

    if digit.decimal {
        let radius = rect.width().min(rect.height()) * DIGITAL_DECIMAL_RADIUS_FRAC;
        let center = Pos2::new(rect.right() + radius * 1.15, rect.bottom() - radius * 1.2);
        let color = with_alpha(colors.text_on_dark, DIGITAL_SEGMENT_ACTIVE_ALPHA);
        painter.circle_filled(center, radius, color);
        painter.add(
            egui::epaint::RectShape::filled(
                Rect::from_center_size(center, Vec2::splat(radius * 3.0)),
                radius * 1.5,
                with_alpha(colors.text_on_dark, DIGITAL_SEGMENT_GLOW_ALPHA),
            )
            .with_blur_width(radius * 1.25),
        );
    }
}

fn digit_segment_points(rect: Rect, index: usize) -> Vec<Pos2> {
    let thickness = rect.width().min(rect.height()) * DIGITAL_SEGMENT_THICKNESS_FRAC;
    let mid_y = rect.center().y;
    let edge = thickness * 0.18;
    match index {
        0 => horizontal_segment(
            rect.left() + thickness * 0.42,
            rect.right() - thickness * 0.42,
            rect.top() + edge,
            thickness,
        ),
        1 => vertical_segment(
            rect.right() - thickness - edge,
            rect.top() + thickness * 0.58,
            mid_y - thickness * 0.24,
            thickness,
        ),
        2 => vertical_segment(
            rect.right() - thickness - edge,
            mid_y + thickness * 0.24,
            rect.bottom() - thickness * 0.58,
            thickness,
        ),
        3 => horizontal_segment(
            rect.left() + thickness * 0.42,
            rect.right() - thickness * 0.42,
            rect.bottom() - thickness - edge,
            thickness,
        ),
        4 => vertical_segment(
            rect.left() + edge,
            mid_y + thickness * 0.24,
            rect.bottom() - thickness * 0.58,
            thickness,
        ),
        5 => vertical_segment(
            rect.left() + edge,
            rect.top() + thickness * 0.58,
            mid_y - thickness * 0.24,
            thickness,
        ),
        _ => horizontal_segment(
            rect.left() + thickness * 0.52,
            rect.right() - thickness * 0.52,
            mid_y - thickness * 0.5,
            thickness,
        ),
    }
}

fn horizontal_segment(left: f32, right: f32, top: f32, thickness: f32) -> Vec<Pos2> {
    let chamfer = thickness * 0.58;
    let mid_y = top + thickness * 0.5;
    vec![
        Pos2::new(left + chamfer, top),
        Pos2::new(right - chamfer, top),
        Pos2::new(right, mid_y),
        Pos2::new(right - chamfer, top + thickness),
        Pos2::new(left + chamfer, top + thickness),
        Pos2::new(left, mid_y),
    ]
}

fn vertical_segment(left: f32, top: f32, bottom: f32, thickness: f32) -> Vec<Pos2> {
    let chamfer = thickness * 0.58;
    vec![
        Pos2::new(left + chamfer, top),
        Pos2::new(left + thickness, top + chamfer),
        Pos2::new(left + thickness, bottom - chamfer),
        Pos2::new(left + chamfer, bottom),
        Pos2::new(left, bottom - chamfer),
        Pos2::new(left, top + chamfer),
    ]
}

fn paint_digit_segment(painter: &egui::Painter, points: Vec<Pos2>, colors: WidgetColors) {
    let bounds = Rect::from_points(&points);
    painter.add(Shape::convex_polygon(
        expand_points(&points, bounds.center(), 1.12),
        with_alpha(colors.white, DIGITAL_SEGMENT_GLOW_ALPHA),
        Stroke::NONE,
    ));
    painter.add(Shape::convex_polygon(
        points,
        with_alpha(colors.white, DIGITAL_SEGMENT_ACTIVE_ALPHA),
        Stroke::NONE,
    ));
}

fn expand_points(points: &[Pos2], center: Pos2, scale: f32) -> Vec<Pos2> {
    points
        .iter()
        .map(|point| center + (*point - center) * scale)
        .collect()
}

fn paint_arc_rounded(
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    start: f32,
    end: f32,
    width: f32,
    color: Color32,
) {
    if (end - start).abs() < 0.002 {
        return;
    }

    let mut points = Vec::with_capacity(ARC_SEGMENTS + 1);
    for i in 0..=ARC_SEGMENTS {
        let t = i as f32 / ARC_SEGMENTS as f32;
        let a = start + (end - start) * t;
        points.push(center + Vec2::angled(a) * radius);
    }
    painter.add(Shape::line(points, Stroke::new(width, color)));

    let cap_r = width * 0.5;
    let start_pos = center + Vec2::angled(start) * radius;
    let end_pos = center + Vec2::angled(end) * radius;
    painter.circle_filled(start_pos, cap_r, color);
    painter.circle_filled(end_pos, cap_r, color);
}
