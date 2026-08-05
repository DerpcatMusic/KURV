use crate::{
    LIGHT_TOKENS,
    tokens::{WidgetTokens, lerp_color, with_alpha},
};
use egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontId, Painter, Pos2, Rect, Response, Sense,
    Stroke, Ui, Vec2,
};

#[derive(Clone, Copy, Debug)]
pub struct StereoDbMeterValues {
    pub meter_db: [f32; 2],
    pub peak_db: [f32; 2],
    pub lane_labels: [&'static str; 2],
}

impl Default for StereoDbMeterValues {
    fn default() -> Self {
        Self {
            meter_db: [-12.0, -14.5],
            peak_db: [-6.0, -8.0],
            lane_labels: ["L", "R"],
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MixDbMeterValues {
    pub mix01: f32,
    pub meter_db: [f32; 2],
    pub peak_db: [f32; 2],
    pub lane_labels: [&'static str; 2],
}

impl Default for MixDbMeterValues {
    fn default() -> Self {
        Self {
            mix01: 0.72,
            meter_db: [-12.0, -14.5],
            peak_db: [-6.0, -8.0],
            lane_labels: ["L", "R"],
        }
    }
}

impl From<MixDbMeterValues> for StereoDbMeterValues {
    fn from(values: MixDbMeterValues) -> Self {
        Self {
            meter_db: values.meter_db,
            peak_db: values.peak_db,
            lane_labels: values.lane_labels,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DbMeterRange {
    pub floor_db: f32,
    pub ceil_db: f32,
}

impl Default for DbMeterRange {
    fn default() -> Self {
        Self {
            floor_db: -60.0,
            ceil_db: 18.0,
        }
    }
}

pub fn mix_db_meter(ui: &mut Ui, values: &mut MixDbMeterValues) -> Response {
    mix_db_meter_with_tokens(
        ui,
        values,
        Vec2::new(96.0, 228.0),
        DbMeterRange::default(),
        &LIGHT_TOKENS,
    )
}

pub fn mix_db_meter_with_tokens(
    ui: &mut Ui,
    values: &mut MixDbMeterValues,
    size: Vec2,
    range: DbMeterRange,
    tokens: &WidgetTokens,
) -> Response {
    let size = Vec2::new(
        size.x
            .clamp(tokens.spacing.lg * 4.5, tokens.spacing.lg * 8.0),
        size.y
            .clamp(tokens.spacing.lg * 9.0, tokens.spacing.lg * 18.0),
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let response = response.on_hover_cursor(CursorIcon::ResizeHorizontal);

    if (response.dragged() || response.clicked())
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let mix_rect = mix_db_meter_mix_rect(rect, tokens);
        values.mix01 = ((pointer.x - mix_rect.left()) / mix_rect.width().max(1.0)).clamp(0.0, 1.0);
    }

    paint_mix_db_meter(ui.painter(), rect, *values, range, tokens);

    response
}

pub fn paint_mix_db_meter(
    painter: &Painter,
    rect: Rect,
    values: MixDbMeterValues,
    range: DbMeterRange,
    tokens: &WidgetTokens,
) {
    paint_stereo_db_meter(painter, rect, values.into(), range, tokens);
    paint_mix_bar(
        painter,
        mix_db_meter_mix_rect(rect, tokens),
        values.mix01,
        tokens,
    );
}

pub fn paint_stereo_db_meter(
    painter: &Painter,
    rect: Rect,
    values: StereoDbMeterValues,
    range: DbMeterRange,
    tokens: &WidgetTokens,
) {
    let range = normalized_range(range);
    let panel_rounding = CornerRadius::same(tokens.radius.tile);
    let meter = stereo_db_meter_track_rect(rect, tokens);
    let colors = tokens.colors;
    let footer_top = rect.bottom() - footer_band_height(tokens);

    painter.add(egui::Shape::Rect(
        egui::epaint::Shadow {
            offset: [0, 2],
            blur: if tokens.light_visuals { 5 } else { 7 },
            spread: 0,
            color: with_alpha(colors.shadow, if tokens.light_visuals { 44 } else { 104 }),
        }
        .as_shape(rect.shrink(tokens.stroke.control * 0.5), panel_rounding),
    ));
    let panel_fill = lerp_color(colors.surface_dark, colors.surface_low, 0.18);
    painter.rect_filled(rect, panel_rounding, panel_fill);
    painter.rect_stroke(
        rect.shrink(tokens.stroke.control * 0.5),
        panel_rounding,
        Stroke::new(tokens.stroke.control, with_alpha(colors.border, 224)),
        egui::StrokeKind::Inside,
    );

    painter.text(
        rect.center_top() + Vec2::new(0.0, tokens.spacing.sm),
        Align2::CENTER_TOP,
        "dB",
        FontId::monospace(tokens.spacing.md),
        with_alpha(colors.text_on_dark, 200),
    );

    paint_meter_scale(painter, meter, range, tokens);
    paint_meter_tracks(painter, meter, values, range, tokens);

    let readout_db = values
        .meter_db
        .iter()
        .copied()
        .fold(range.floor_db, f32::max)
        .clamp(range.floor_db, range.ceil_db);
    painter.text(
        Pos2::new(rect.center().x, footer_top + tokens.spacing.xs),
        Align2::CENTER_TOP,
        "dBFS",
        FontId::monospace(tokens.spacing.sm + 1.0),
        with_alpha(colors.text_on_dark, 160),
    );
    painter.text(
        Pos2::new(
            rect.center().x,
            footer_top + tokens.spacing.md + tokens.spacing.xs * 0.5,
        ),
        Align2::CENTER_TOP,
        format!("{readout_db:.1}"),
        FontId::monospace(tokens.spacing.sm),
        colors.text_on_dark,
    );
    let left_db = values.meter_db[0];
    let right_db = values.meter_db[1];
    painter.text(
        Pos2::new(
            rect.center().x,
            footer_top + tokens.spacing.lg + tokens.spacing.xs * 0.25,
        ),
        Align2::CENTER_TOP,
        format!(
            "{} {:.0}  {} {:.0}",
            values.lane_labels[0], left_db, values.lane_labels[1], right_db
        ),
        FontId::monospace(tokens.spacing.sm - 1.0),
        with_alpha(colors.text_on_dark, 168),
    );
}

fn footer_band_height(tokens: &WidgetTokens) -> f32 {
    tokens.spacing.lg * 2.35
}

fn paint_meter_tracks(
    painter: &Painter,
    meter: Rect,
    values: StereoDbMeterValues,
    range: DbMeterRange,
    tokens: &WidgetTokens,
) {
    painter.add(
        egui::epaint::RectShape::filled(
            meter.translate(Vec2::new(1.0, 1.5)),
            CornerRadius::same(tokens.radius.control),
            with_alpha(
                tokens.colors.shadow,
                if tokens.light_visuals { 64 } else { 144 },
            ),
        )
        .with_blur_width(4.0),
    );

    for lane in 0..2 {
        let track = lane_rect(meter, lane, tokens);
        paint_meter_lane(
            painter,
            track,
            values.meter_db[lane],
            values.peak_db[lane],
            range,
            tokens,
        );
        painter.text(
            Pos2::new(track.center().x, meter.bottom() + tokens.spacing.xs * 1.4),
            Align2::CENTER_TOP,
            values.lane_labels[lane],
            FontId::monospace(tokens.spacing.sm),
            with_alpha(tokens.colors.text_on_dark, 196),
        );
    }
}

fn paint_meter_lane(
    painter: &Painter,
    track: Rect,
    meter_db: f32,
    peak_db: f32,
    range: DbMeterRange,
    tokens: &WidgetTokens,
) {
    let rounding = CornerRadius::same(tokens.radius.control);
    let well = lerp_color(tokens.colors.surface_dark, tokens.colors.track, 0.08);
    painter.rect_filled(track, rounding, well);
    painter.rect_stroke(
        track.shrink(tokens.stroke.control * 0.5),
        rounding,
        Stroke::new(tokens.stroke.control, with_alpha(tokens.colors.border, 224)),
        egui::StrokeKind::Inside,
    );

    let fill = track.shrink2(Vec2::new(
        tokens.stroke.control + 1.5,
        tokens.stroke.control + 1.5,
    ));
    paint_meter_fill(painter, fill, meter_db, range, tokens);

    let peak_y = db_to_y(track, peak_db, range);
    let peak_color = if peak_db > 3.0 {
        tokens.colors.error
    } else if peak_db > -6.0 {
        tokens.colors.warning
    } else {
        with_alpha(tokens.colors.text_on_dark, 225)
    };
    painter.line_segment(
        [
            Pos2::new(track.left() - 0.5, peak_y),
            Pos2::new(track.right() + 0.5, peak_y),
        ],
        Stroke::new(tokens.stroke.control + 0.35, peak_color),
    );
}

fn paint_meter_scale(painter: &Painter, meter: Rect, range: DbMeterRange, tokens: &WidgetTokens) {
    let minor = minor_ticks(range);
    for db in minor {
        let y = db_to_y(meter, db, range);
        let major = is_major_tick(db);
        let tick = if major {
            tokens.spacing.xs + 1.0
        } else {
            tokens.spacing.xs * 0.5
        };
        let tick_alpha = if major { 128 } else { 62 };
        let stroke = Stroke::new(
            if major { 0.85_f32 } else { 0.55_f32 },
            with_alpha(tokens.colors.text, tick_alpha),
        );
        painter.line_segment(
            [
                Pos2::new(meter.left() - tick, y),
                Pos2::new(meter.left() - 1.0, y),
            ],
            stroke,
        );
        if major {
            let label = db_label(db);
            painter.text(
                Pos2::new(meter.left() - tokens.spacing.sm, y),
                Align2::RIGHT_CENTER,
                &label,
                FontId::monospace(tokens.spacing.sm - 1.0),
                with_alpha(tokens.colors.text, 154),
            );
        }
    }
}

pub fn stereo_db_meter_track_rect(rect: Rect, tokens: &WidgetTokens) -> Rect {
    Rect::from_min_max(
        Pos2::new(
            rect.center().x - tokens.spacing.md * 1.1,
            rect.top() + tokens.spacing.lg * 2.0,
        ),
        Pos2::new(
            rect.center().x + tokens.spacing.md * 1.1,
            rect.bottom() - footer_band_height(tokens) - tokens.spacing.xs,
        ),
    )
}

/// Back-compat alias for hosts that still reference the mix meter layout helper.
pub fn mix_db_meter_track_rect(rect: Rect, tokens: &WidgetTokens) -> Rect {
    stereo_db_meter_track_rect(rect, tokens)
}

pub fn mix_db_meter_mix_rect(rect: Rect, tokens: &WidgetTokens) -> Rect {
    let footer_top = rect.bottom() - footer_band_height(tokens);
    Rect::from_center_size(
        Pos2::new(rect.center().x, footer_top - tokens.spacing.sm * 0.75),
        Vec2::new(
            (rect.width() - tokens.spacing.md * 1.5).max(tokens.spacing.lg),
            6.0,
        ),
    )
}

fn paint_mix_bar(painter: &Painter, rect: Rect, mix01: f32, tokens: &WidgetTokens) {
    let mix01 = mix01.clamp(0.0, 1.0);
    let colors = tokens.colors;
    let rounding = CornerRadius::same((rect.height() * 0.32).round().clamp(1.0, 4.0) as u8);
    painter.add(
        egui::epaint::RectShape::filled(
            rect.translate(Vec2::new(0.0, 1.0)),
            rounding,
            with_alpha(colors.shadow, if tokens.light_visuals { 72 } else { 120 }),
        )
        .with_blur_width(3.0),
    );
    painter.rect_filled(
        rect,
        rounding,
        lerp_color(colors.surface_dark, colors.track, 0.12),
    );
    painter.rect_stroke(
        rect.shrink(tokens.stroke.control * 0.5),
        rounding,
        Stroke::new(tokens.stroke.control, with_alpha(colors.border, 210)),
        egui::StrokeKind::Inside,
    );

    let filled = Rect::from_min_max(
        rect.left_top(),
        Pos2::new(rect.left() + rect.width() * mix01, rect.bottom()),
    );
    if filled.width() > 0.5 {
        paint_vertical_gradient(
            painter,
            filled.shrink(tokens.stroke.control),
            with_alpha(colors.white, 210),
            with_alpha(colors.white, 118),
        );
    }

    let handle_center = Pos2::new(rect.left() + rect.width() * mix01, rect.center().y);
    let handle = Rect::from_center_size(
        handle_center,
        Vec2::new(tokens.spacing.sm * 0.9, rect.height() + tokens.spacing.xs),
    );
    painter.add(
        egui::epaint::RectShape::filled(
            handle.translate(Vec2::new(0.0, 1.0)),
            CornerRadius::same(tokens.radius.control),
            with_alpha(colors.shadow, if tokens.light_visuals { 78 } else { 130 }),
        )
        .with_blur_width(3.0),
    );
    painter.rect_filled(
        handle,
        CornerRadius::same(tokens.radius.control),
        colors.surface_high,
    );
    painter.line_segment(
        [
            handle.left_top() + Vec2::new(1.5, 1.0),
            handle.right_top() + Vec2::new(-1.5, 1.0),
        ],
        Stroke::new(tokens.stroke.control * 0.65, with_alpha(colors.white, 120)),
    );
}

fn lane_rect(meter: Rect, lane: usize, tokens: &WidgetTokens) -> Rect {
    let gap = tokens.spacing.xs * 0.9;
    let width = (meter.width() - gap) * 0.5;
    let left = meter.left() + lane as f32 * (width + gap);
    Rect::from_min_max(
        Pos2::new(left, meter.top()),
        Pos2::new(left + width, meter.bottom()),
    )
}

fn paint_meter_fill(
    painter: &Painter,
    track: Rect,
    meter_db: f32,
    range: DbMeterRange,
    tokens: &WidgetTokens,
) {
    let fill_top = db_to_y(track, meter_db, range);
    if fill_top >= track.bottom() - 0.5 {
        return;
    }

    let filled = Rect::from_min_max(Pos2::new(track.left(), fill_top), track.right_bottom());
    let clipped = painter.with_clip_rect(filled);
    let mut start_db = range.floor_db;
    for end_db in [-18.0, -6.0, 0.0, range.ceil_db] {
        if end_db <= range.floor_db || start_db >= range.ceil_db {
            continue;
        }
        let low = start_db.clamp(range.floor_db, range.ceil_db);
        let high = end_db.clamp(range.floor_db, range.ceil_db);
        if high > low + f32::EPSILON {
            let bottom = db_to_y(track, low, range)
                .min(filled.bottom())
                .max(filled.top());
            let top = db_to_y(track, high, range)
                .min(filled.bottom())
                .max(filled.top());
            if top < bottom - 0.5 {
                paint_vertical_gradient(
                    &clipped,
                    Rect::from_min_max(
                        Pos2::new(track.left(), top),
                        Pos2::new(track.right(), bottom),
                    ),
                    meter_color(high, tokens),
                    meter_color(low, tokens),
                );
            }
        }
        start_db = end_db;
    }

    let center_highlight = Rect::from_min_max(
        Pos2::new(filled.left() + filled.width() * 0.34, filled.top()),
        Pos2::new(filled.right(), filled.bottom()),
    );
    paint_vertical_gradient(
        &clipped,
        center_highlight,
        with_alpha(tokens.colors.white, 88),
        with_alpha(tokens.colors.white, 150),
    );
}

fn paint_vertical_gradient(painter: &Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::epaint::Mesh::default();
    let base = mesh.vertices.len() as u32;
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.add_triangle(base, base + 1, base + 2);
    mesh.add_triangle(base + 1, base + 3, base + 2);
    painter.add(mesh);
}

fn meter_color(db: f32, tokens: &WidgetTokens) -> Color32 {
    if db >= 0.0 {
        lerp_color(
            tokens.colors.warning,
            tokens.colors.error,
            (db / 6.0).clamp(0.0, 1.0),
        )
    } else if db >= -12.0 {
        lerp_color(
            tokens.colors.white,
            tokens.colors.warning,
            ((db + 12.0) / 12.0).clamp(0.0, 1.0),
        )
    } else {
        lerp_color(
            tokens.colors.surface_high,
            tokens.colors.white,
            ((db + 60.0) / 48.0).clamp(0.0, 1.0),
        )
    }
}

fn db_to_y(track: Rect, db: f32, range: DbMeterRange) -> f32 {
    let span = (range.ceil_db - range.floor_db).max(f32::EPSILON);
    track.bottom()
        - track.height() * ((db.clamp(range.floor_db, range.ceil_db) - range.floor_db) / span)
}

fn normalized_range(range: DbMeterRange) -> DbMeterRange {
    if range.ceil_db > range.floor_db + f32::EPSILON {
        range
    } else {
        DbMeterRange::default()
    }
}

fn minor_ticks(range: DbMeterRange) -> impl Iterator<Item = f32> {
    [
        -60.0, -54.0, -48.0, -42.0, -36.0, -30.0, -24.0, -20.0, -18.0, -16.0, -12.0, -8.0, -6.0,
        -4.0, 0.0, 4.0, 6.0, 8.0, 12.0, 16.0, 18.0, 20.0,
    ]
    .into_iter()
    .filter(move |db| *db >= range.floor_db && *db <= range.ceil_db)
}

fn is_major_tick(db: f32) -> bool {
    matches!(
        db.round() as i32,
        -60 | -48 | -36 | -24 | -18 | -12 | -6 | 0 | 6 | 12 | 18 | 20
    )
}

fn db_label(db: f32) -> String {
    if db > 0.0 {
        format!("+{db:.0}")
    } else {
        format!("{db:.0}")
    }
}
