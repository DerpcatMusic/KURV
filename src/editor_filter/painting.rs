//! Paint-only helpers for the ordered filter card.

use std::sync::Arc;

use crate::editor_controls::fit_font_to_width;
use crate::filters::{
    FilterConfig, FilterMode, MAX_RATIO, MIN_RATIO, normalized_ratio, ratio_brickwall_bypassed,
};
use crate::{editor_theme, editor_widgets};

use super::{
    MAX_CUTOFF_HZ, MAX_RESPONSE_SEGMENTS, MIN_CUTOFF_HZ, MIN_RESPONSE_SEGMENTS, normalized_log,
};

pub(super) fn paint_header(
    ui: &egui::Ui,
    identity: egui::Rect,
    close_rect: egui::Rect,
    number: usize,
    drag_response: &egui::Response,
    close_response: &egui::Response,
    accent: egui::Color32,
) {
    let palette = editor_theme::semantic();
    let active = drag_response.dragged() || drag_response.hovered() || drag_response.has_focus();
    let label = format!("FILTER {number}");
    let label_rect = egui::Rect::from_min_max(
        egui::pos2(identity.left(), close_rect.bottom()),
        identity.right_bottom(),
    );
    editor_widgets::paint_vertical_label(
        ui,
        label_rect,
        &label,
        fit_font_to_width(
            ui.painter(),
            &label,
            editor_theme::font::title(),
            label_rect.height() * 0.90,
        ),
        if active { accent } else { palette.text },
    );

    let identity_hot = ui.rect_contains_pointer(identity)
        || drag_response.hovered()
        || drag_response.dragged()
        || drag_response.has_focus()
        || close_response.hovered();
    if identity_hot {
        let cross = close_rect.shrink(close_rect.width() * 0.34);
        let color = if close_response.hovered() || close_response.is_pointer_button_down_on() {
            palette.danger
        } else {
            palette.text_muted.gamma_multiply(0.72)
        };
        let stroke = egui::Stroke::new(editor_theme::shape::STROKE, color);
        ui.painter()
            .line_segment([cross.left_top(), cross.right_bottom()], stroke);
        ui.painter()
            .line_segment([cross.right_top(), cross.left_bottom()], stroke);
    }
}

pub(super) const MIN_RESPONSE_DB: f32 = -54.0;
pub(super) const MAX_RESPONSE_DB: f32 = 18.0;
const RESPONSE_OVERFLOW: f32 = 10.0;

#[derive(Clone)]
struct ResponsePointsCache {
    key: egui::Id,
    points: Arc<Vec<egui::Pos2>>,
    maximum_db: f32,
}

pub(super) fn paint_response_preview(
    ui: &egui::Ui,
    rect: egui::Rect,
    config: FilterConfig,
    accent: egui::Color32,
    response: &egui::Response,
    dsp_sample_rate: f32,
) {
    if !rect.is_positive() {
        return;
    }
    if config.mode == FilterMode::RatioBrickwall {
        paint_ratio_response_preview(ui, rect, config, accent, response);
        return;
    }
    let painter = ui.painter_at(rect);
    let palette = editor_theme::semantic();
    let count = ((rect.width() / editor_theme::space::XXS.max(1.0)).ceil() as usize)
        .clamp(MIN_RESPONSE_SEGMENTS, MAX_RESPONSE_SEGMENTS);
    let glow = egui::Stroke::new(
        editor_theme::shape::FOCUS_STROKE * 3.2,
        accent.gamma_multiply(if response.hovered() { 0.28 } else { 0.16 }),
    );
    let stroke = egui::Stroke::new(
        editor_theme::shape::FOCUS_STROKE,
        accent.gamma_multiply(if response.hovered() { 1.0 } else { 0.9 }),
    );
    let cache_key = (
        config.mode as u8,
        config.cutoff_hz.to_bits(),
        config.q.to_bits(),
        config.slope_db_oct.to_bits(),
        config.morph.to_bits(),
        config.shape.to_bits(),
        dsp_sample_rate.to_bits(),
        [
            rect.min.x.to_bits(),
            rect.min.y.to_bits(),
            rect.width().to_bits(),
            rect.height().to_bits(),
        ],
        count,
        painter.ctx().pixels_per_point().to_bits(),
    );
    let points_id = response.id.with("filter-response-points");
    let points_key = egui::Id::new(cache_key);
    let plot = ui
        .data(|store| store.get_temp::<ResponsePointsCache>(points_id))
        .filter(|cached| cached.key == points_key)
        .unwrap_or_else(|| {
            let (points, maximum_db) = response_points(rect, config, dsp_sample_rate, count);
            let points = Arc::new(points);
            let plot = ResponsePointsCache {
                key: points_key,
                points: Arc::clone(&points),
                maximum_db,
            };
            ui.data_mut(|store| {
                store.insert_temp(points_id, plot.clone());
            });
            plot
        });
    paint_frequency_grid(&painter, rect, palette.grid);
    let cutoff_x = frequency_to_x(rect, config.cutoff_hz);
    let zero_y = response_db_to_y_with_ceiling(rect, 0.0, plot.maximum_db);
    painter.line_segment(
        [
            egui::pos2(rect.left(), zero_y),
            egui::pos2(rect.right(), zero_y),
        ],
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            palette.grid.gamma_multiply(0.55),
        ),
    );
    painter.line_segment(
        [
            egui::pos2(cutoff_x, rect.top()),
            egui::pos2(cutoff_x, rect.bottom()),
        ],
        egui::Stroke::new(editor_theme::shape::STROKE, accent.gamma_multiply(0.22)),
    );
    paint_stage_ticks(&painter, rect, config, dsp_sample_rate, accent);
    paint_response_scope(&painter, rect, config.mode, plot.maximum_db, palette.grid);
    let glow_mesh = editor_widgets::cached_stroke_mesh(
        ui,
        response.id.with("filter-response-glow"),
        (cache_key, glow.color.to_array()),
        || plot.points.as_ref().clone(),
        glow,
    );
    painter.add(glow_mesh);
    let mesh = editor_widgets::cached_gradient_stroke_mesh(
        ui,
        response.id.with("filter-response-mesh"),
        (cache_key, stroke.color.to_array()),
        || plot.points.as_ref().clone(),
        rect.bottom(),
        accent,
        48,
        stroke,
    );
    painter.add(mesh);
    let radius = editor_theme::font::CAPTION_SIZE * if response.dragged() { 0.36 } else { 0.28 };
    let bounds = rect.shrink(radius.min(rect.width().min(rect.height()) * 0.5));
    let handle = egui::pos2(
        cutoff_x.clamp(bounds.left(), bounds.right()),
        egui::lerp(
            rect.bottom()..=rect.top(),
            normalized_log(config.q, 0.1, 32.0),
        )
        .clamp(bounds.top(), bounds.bottom()),
    );
    painter.circle_filled(handle, radius * 1.55, accent.gamma_multiply(0.22));
    painter.circle_filled(
        handle,
        radius,
        if response.hovered() {
            palette.text
        } else {
            accent
        },
    );
}

fn paint_ratio_response_preview(
    ui: &egui::Ui,
    rect: egui::Rect,
    config: FilterConfig,
    accent: egui::Color32,
    response: &egui::Response,
) {
    let painter = ui.painter_at(rect);
    let palette = editor_theme::semantic();
    let ratio_x = |ratio| egui::lerp(rect.left()..=rect.right(), normalized_ratio(ratio));
    for ratio in [0.0, 1.0, 2.0, 4.0, 16.0, 64.0, 256.0, 1_024.0] {
        let x = ratio_x(ratio);
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(
                editor_theme::shape::STROKE,
                palette.grid.gamma_multiply(0.28),
            ),
        );
        painter.text(
            egui::pos2(x, rect.bottom() - editor_theme::space::XXS),
            if ratio == MAX_RATIO {
                egui::Align2::RIGHT_BOTTOM
            } else {
                egui::Align2::LEFT_BOTTOM
            },
            format!("{ratio:.0}x"),
            editor_theme::font::caption(),
            palette.grid.gamma_multiply(0.62),
        );
    }
    let count = ((rect.width() / 2.0).ceil() as usize).clamp(64, 256);
    let band = if ratio_brickwall_bypassed(config.cutoff_hz, config.shape >= 0.5) {
        (MIN_RATIO, MAX_RATIO)
    } else if config.shape >= 0.5 {
        (MIN_RATIO, config.cutoff_hz)
    } else {
        (config.cutoff_hz, MAX_RATIO)
    };
    let phase_step = 1.0 / count as f32;
    let samples = (0..=count)
        .map(|index| {
            crate::oscillators::VaOscillator::preview_shape_ratio(
                2.0,
                index as f32 / count as f32,
                phase_step,
                0.5,
                crate::oscillators::PhaseWarpMode::None,
                0.0,
                band,
            )
        })
        .collect::<Vec<_>>();
    let stroke = egui::Stroke::new(
        editor_theme::shape::FOCUS_STROKE,
        accent.gamma_multiply(if response.hovered() { 1.0 } else { 0.9 }),
    );
    let mid = rect.center().y;
    painter.line_segment(
        [egui::pos2(rect.left(), mid), egui::pos2(rect.right(), mid)],
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            palette.grid.gamma_multiply(0.55),
        ),
    );
    painter.add(egui::Shape::line(
        samples
            .into_iter()
            .enumerate()
            .map(|(index, sample)| {
                egui::pos2(
                    egui::lerp(rect.left()..=rect.right(), index as f32 / count as f32),
                    mid - sample.clamp(-1.25, 1.25) * rect.height() * 0.34,
                )
            })
            .collect(),
        stroke,
    ));
    painter.text(
        rect.left_top() + egui::vec2(editor_theme::space::XXS, editor_theme::space::XXS),
        egui::Align2::LEFT_TOP,
        "POST-MASK SAW",
        editor_theme::font::caption(),
        palette.text_muted.gamma_multiply(0.62),
    );
}

pub(super) fn paint_pass_toggle(
    ui: &egui::Ui,
    rect: egui::Rect,
    lowpass: bool,
    response: &egui::Response,
    accent: egui::Color32,
) {
    let palette = editor_theme::semantic();
    if response.hovered() || response.is_pointer_button_down_on() {
        ui.painter().rect_filled(
            rect,
            editor_theme::shape::CONTROL_RADIUS,
            palette.control_hover,
        );
    }
    ui.painter().text(
        rect.left_top() + egui::vec2(editor_theme::space::XS, editor_theme::space::XXS),
        egui::Align2::LEFT_TOP,
        "PASS",
        editor_theme::font::caption(),
        palette.text_muted,
    );
    ui.painter().text(
        rect.center() + egui::vec2(0.0, editor_theme::space::XXS),
        egui::Align2::CENTER_CENTER,
        if lowpass { "LOW" } else { "HIGH" },
        editor_theme::font::value(),
        accent,
    );
}

fn paint_frequency_grid(painter: &egui::Painter, rect: egui::Rect, grid: egui::Color32) {
    for frequency in [100.0, 1_000.0, 10_000.0, 20_000.0] {
        let x = frequency_to_x(rect, frequency);
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(editor_theme::shape::STROKE, grid.gamma_multiply(0.28)),
        );
        let label = if frequency >= 1_000.0 {
            format!("{:.0}k", frequency / 1_000.0)
        } else {
            format!("{frequency:.0}")
        };
        let at_right_edge = frequency == MAX_CUTOFF_HZ;
        painter.text(
            egui::pos2(
                x + if at_right_edge {
                    -editor_theme::space::XXS
                } else {
                    editor_theme::space::XXS
                },
                rect.bottom() - editor_theme::space::XXS,
            ),
            if at_right_edge {
                egui::Align2::RIGHT_BOTTOM
            } else {
                egui::Align2::LEFT_BOTTOM
            },
            label,
            editor_theme::font::caption(),
            grid.gamma_multiply(0.62),
        );
    }
}

fn paint_stage_ticks(
    painter: &egui::Painter,
    rect: egui::Rect,
    config: FilterConfig,
    dsp_sample_rate: f32,
    accent: egui::Color32,
) {
    if matches!(config.mode, crate::filters::FilterMode::Scream) {
        if let Some(frequency) = config.scream_feedback_frequency(dsp_sample_rate)
            && (MIN_CUTOFF_HZ..=MAX_CUTOFF_HZ).contains(&frequency)
        {
            let x = frequency_to_x(rect, frequency);
            painter.line_segment(
                [
                    egui::pos2(x, rect.bottom() - rect.height() * 0.14),
                    egui::pos2(x, rect.bottom()),
                ],
                egui::Stroke::new(
                    editor_theme::shape::FOCUS_STROKE,
                    accent.gamma_multiply(0.58),
                ),
            );
        }
        return;
    }
    if config.mode != crate::filters::FilterMode::Phaser {
        return;
    }
    let count = usize::from(config.response_stage_count());
    let fractional_pole = config.effective_poles().fract();
    let pole_width = config.shape.mul_add(config.shape, 0.12);
    for index in 0..count {
        let frequency = config.stage_frequency(index, dsp_sample_rate);
        if !(MIN_CUTOFF_HZ..=MAX_CUTOFF_HZ).contains(&frequency) {
            continue;
        }
        let x = frequency_to_x(rect, frequency);
        let height = rect.height() * if count > 16 { 0.10 } else { 0.16 } * pole_width.sqrt();
        let strength = if index + 1 == count && fractional_pole > f32::EPSILON {
            fractional_pole
        } else {
            1.0
        };
        painter.line_segment(
            [
                egui::pos2(x, rect.bottom() - height),
                egui::pos2(x, rect.bottom()),
            ],
            egui::Stroke::new(
                editor_theme::shape::STROKE * pole_width,
                accent.gamma_multiply(strength * if count > 24 { 0.28 } else { 0.46 }),
            ),
        );
    }
}

pub(super) fn response_db(magnitude: f32) -> f32 {
    20.0 * magnitude.max(1.0e-12).log10()
}

#[cfg(test)]
pub(super) fn response_db_to_y(rect: egui::Rect, db: f32) -> f32 {
    response_db_to_y_with_ceiling(rect, db, MAX_RESPONSE_DB)
}

fn response_db_to_y_with_ceiling(rect: egui::Rect, db: f32, maximum_db: f32) -> f32 {
    let span = maximum_db - MIN_RESPONSE_DB;
    let normalized = (db - MIN_RESPONSE_DB) / span;
    egui::lerp(rect.bottom()..=rect.top(), normalized)
}

fn frequency_to_x(rect: egui::Rect, frequency: f32) -> f32 {
    egui::lerp(
        rect.left()..=rect.right(),
        normalized_log(frequency, MIN_CUTOFF_HZ, MAX_CUTOFF_HZ),
    )
}

fn response_points(
    rect: egui::Rect,
    config: FilterConfig,
    dsp_sample_rate: f32,
    count: usize,
) -> (Vec<egui::Pos2>, f32) {
    let mut frequencies = Vec::with_capacity(count + 10);
    for index in 0..=count {
        let x = index as f32 / count as f32;
        frequencies.push(MIN_CUTOFF_HZ * (MAX_CUTOFF_HZ / MIN_CUTOFF_HZ).powf(x));
    }
    let mut centers = vec![config.cutoff_hz];
    let stage_count = usize::from(config.response_stage_count());
    if matches!(config.mode, crate::filters::FilterMode::Phaser) {
        centers
            .extend((0..stage_count).map(|index| config.stage_frequency(index, dsp_sample_rate)));
    } else if let Some(frequency) = config.scream_feedback_frequency(dsp_sample_rate) {
        centers.push(frequency);
    }
    for center in centers {
        for offset_octaves in [-0.08, -0.04, -0.02, -0.01, 0.0, 0.01, 0.02, 0.04, 0.08] {
            let frequency = center * 2.0_f32.powf(offset_octaves);
            if frequency > MIN_CUTOFF_HZ && frequency < MAX_CUTOFF_HZ {
                frequencies.push(frequency);
            }
        }
    }
    frequencies.sort_by(f32::total_cmp);
    frequencies.dedup_by(|left, right| (*left - *right).abs() <= f32::EPSILON);
    let overflow_top = rect.top() - RESPONSE_OVERFLOW;
    let overflow_bottom = rect.bottom() + RESPONSE_OVERFLOW;
    let responses = frequencies
        .into_iter()
        .map(|frequency| {
            let db = response_db(config.response_magnitude(frequency, dsp_sample_rate));
            (frequency, db)
        })
        .collect::<Vec<_>>();
    let peak_db = responses
        .iter()
        .map(|(_, db)| *db)
        .fold(MAX_RESPONSE_DB, f32::max);
    let maximum_db = if peak_db > MAX_RESPONSE_DB {
        ((peak_db + 3.0) / 6.0).ceil() * 6.0
    } else {
        MAX_RESPONSE_DB
    };
    (
        responses
            .into_iter()
            .map(|(frequency, db)| {
                egui::pos2(
                    frequency_to_x(rect, frequency),
                    response_db_to_y_with_ceiling(rect, db, maximum_db)
                        .clamp(overflow_top, overflow_bottom),
                )
            })
            .collect(),
        maximum_db,
    )
}

fn paint_response_scope(
    painter: &egui::Painter,
    rect: egui::Rect,
    mode: crate::filters::FilterMode,
    maximum_db: f32,
    color: egui::Color32,
) {
    let label = match mode {
        crate::filters::FilterMode::Svf if maximum_db > MAX_RESPONSE_DB => {
            format!("+{maximum_db:.0} dB")
        }
        crate::filters::FilterMode::Phaser => "MAG ONLY".into(),
        crate::filters::FilterMode::Scream => "EST. @ -6 dBFS".into(),
        crate::filters::FilterMode::RatioBrickwall => return,
        _ => return,
    };
    painter.text(
        rect.left_top() + egui::vec2(editor_theme::space::XXS, editor_theme::space::XXS),
        egui::Align2::LEFT_TOP,
        label,
        editor_theme::font::caption(),
        color.gamma_multiply(0.62),
    );
}

pub(super) fn paint_type_dropdown(
    ui: &egui::Ui,
    rect: egui::Rect,
    mode: crate::filters::FilterMode,
    response: &egui::Response,
    accent: egui::Color32,
) {
    if !rect.is_positive() {
        return;
    }
    let palette = editor_theme::semantic();
    let active = response.hovered() || response.is_pointer_button_down_on();
    let painter = ui.painter_at(rect);
    if active {
        painter.rect_filled(
            rect,
            editor_theme::shape::CONTROL_RADIUS,
            palette.control_hover,
        );
    }
    painter.text(
        rect.left_top() + egui::vec2(editor_theme::space::XS, editor_theme::space::XXS),
        egui::Align2::LEFT_TOP,
        "TYPE",
        editor_theme::font::caption(),
        if active { accent } else { palette.text_muted },
    );
    painter.text(
        egui::pos2(
            rect.left() + editor_theme::space::XS,
            rect.center().y + editor_theme::space::XXS,
        ),
        egui::Align2::LEFT_CENTER,
        mode.label(),
        editor_theme::font::value(),
        if active { palette.text } else { accent },
    );
    let center = egui::pos2(
        rect.right() - editor_theme::space::SM,
        rect.center().y + editor_theme::space::XXS,
    );
    let arrow = egui::Stroke::new(
        editor_theme::shape::STROKE,
        if active { accent } else { palette.text_muted },
    );
    painter.line_segment(
        [
            center + egui::vec2(-3.0, -1.5),
            center + egui::vec2(0.0, 1.5),
        ],
        arrow,
    );
    painter.line_segment(
        [
            center + egui::vec2(0.0, 1.5),
            center + egui::vec2(3.0, -1.5),
        ],
        arrow,
    );
}

pub(crate) fn paint_metric_knob(
    ui: &egui::Ui,
    rect: egui::Rect,
    label: &str,
    value: &str,
    normalized: f32,
    response: &egui::Response,
    accent: egui::Color32,
) {
    if !rect.is_positive() {
        return;
    }
    let palette = editor_theme::semantic();
    let active = response.is_pointer_button_down_on() || response.dragged();
    let highlighted = active || response.hovered();
    let painter = ui.painter_at(rect);
    let center = egui::pos2(rect.center().x, rect.center().y + editor_theme::space::XXS);
    let radius = (rect.width().min(rect.height()) * 0.24).clamp(8.0, 22.0);
    let start = std::f32::consts::PI * 0.75;
    let span = std::f32::consts::PI * 1.5;
    let arc = |end: f32| {
        (0..=24)
            .map(|index| {
                let amount = index as f32 / 24.0;
                let angle = start + (end - start) * amount;
                center + egui::vec2(angle.cos() * radius, angle.sin() * radius)
            })
            .collect::<Vec<_>>()
    };
    painter.add(egui::Shape::line(
        arc(start + span),
        egui::Stroke::new(
            editor_theme::space::SM * 0.34,
            palette.grid.gamma_multiply(0.78),
        ),
    ));
    painter.add(egui::Shape::line(
        arc(start + span * normalized.clamp(0.0, 1.0)),
        egui::Stroke::new(
            editor_theme::space::SM * 0.34,
            accent.gamma_multiply(if highlighted { 1.0 } else { 0.82 }),
        ),
    ));
    painter.circle_filled(center, radius * 0.78, palette.control);
    painter.circle_stroke(
        center,
        radius * 0.78,
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            palette.grid.gamma_multiply(0.72),
        ),
    );
    let angle = start + span * normalized.clamp(0.0, 1.0);
    painter.line_segment(
        [
            center + egui::vec2(angle.cos(), angle.sin()) * radius * 0.20,
            center + egui::vec2(angle.cos(), angle.sin()) * radius * 0.62,
        ],
        egui::Stroke::new(
            editor_theme::shape::STROKE * 1.2,
            if highlighted { palette.text } else { accent },
        ),
    );
    painter.text(
        egui::pos2(
            rect.center().x,
            rect.top() + editor_theme::font::CAPTION_SIZE * 0.62,
        ),
        egui::Align2::CENTER_CENTER,
        label,
        editor_theme::font::caption(),
        if highlighted {
            accent
        } else {
            palette.text_muted
        },
    );
    painter.text(
        egui::pos2(
            rect.center().x,
            rect.bottom() - editor_theme::font::VALUE_SIZE * 0.58,
        ),
        egui::Align2::CENTER_CENTER,
        value,
        editor_theme::font::value(),
        if active { palette.text } else { accent },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plot() -> egui::Rect {
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(200.0, 80.0))
    }

    #[test]
    fn deep_notch_leaves_the_plot_floor() {
        let rect = plot();
        let y = response_db_to_y(rect, response_db(1.0e-6));
        assert!(
            y > rect.bottom(),
            "notch floor should continue below the plot, got {y} vs {}",
            rect.bottom()
        );
    }

    #[test]
    fn unity_gain_stays_inside_the_plot() {
        let rect = plot();
        let y = response_db_to_y(rect, 0.0);
        assert!(y > rect.top());
        assert!(y < rect.bottom());
    }

    #[test]
    fn resonance_peak_can_leave_the_plot_ceiling() {
        let rect = plot();
        let y = response_db_to_y(rect, 24.0);
        assert!(y < rect.top());
    }
}
