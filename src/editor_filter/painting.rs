//! Paint-only helpers for the ordered filter card.

use crate::filters::FilterConfig;
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
    ui.painter().text(
        egui::pos2(
            identity.center().x,
            identity.top() + identity.height() * 0.24,
        ),
        egui::Align2::CENTER_CENTER,
        "FLT",
        editor_theme::font::caption(),
        if active { accent } else { palette.text_muted },
    );
    ui.painter().text(
        egui::pos2(
            identity.center().x,
            identity.top() + identity.height() * 0.34,
        ),
        egui::Align2::CENTER_CENTER,
        number.to_string(),
        editor_theme::font::title(),
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
    let painter = ui.painter_at(rect);
    let palette = editor_theme::semantic();
    paint_frequency_grid(&painter, rect, palette.grid);
    let cutoff_x = frequency_to_x(rect, config.cutoff_hz);
    let zero_y = response_db_to_y(rect, 0.0);
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
    let glow_mesh = editor_widgets::cached_stroke_mesh(
        ui,
        response.id.with("filter-response-glow"),
        (cache_key, glow.color.to_array()),
        || response_points(rect, config, dsp_sample_rate, count),
        glow,
    );
    painter.add(glow_mesh);
    let mesh = editor_widgets::cached_gradient_stroke_mesh(
        ui,
        response.id.with("filter-response-mesh"),
        (cache_key, stroke.color.to_array()),
        || response_points(rect, config, dsp_sample_rate, count),
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

fn paint_frequency_grid(painter: &egui::Painter, rect: egui::Rect, grid: egui::Color32) {
    for frequency in [100.0, 1_000.0, 10_000.0] {
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
        painter.text(
            egui::pos2(
                x + editor_theme::space::XXS,
                rect.bottom() - editor_theme::space::XXS,
            ),
            egui::Align2::LEFT_BOTTOM,
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
    if matches!(config.mode, crate::filters::FilterMode::Svf) {
        return;
    }
    let count = usize::from(config.stage_count());
    for index in 0..count {
        let frequency = config.stage_frequency(index, dsp_sample_rate);
        if !(MIN_CUTOFF_HZ..=MAX_CUTOFF_HZ).contains(&frequency) {
            continue;
        }
        let x = frequency_to_x(rect, frequency);
        let height = rect.height() * if count > 16 { 0.10 } else { 0.16 };
        painter.line_segment(
            [
                egui::pos2(x, rect.bottom() - height),
                egui::pos2(x, rect.bottom()),
            ],
            egui::Stroke::new(
                editor_theme::shape::STROKE,
                accent.gamma_multiply(if count > 24 { 0.28 } else { 0.46 }),
            ),
        );
    }
}

pub(super) fn response_db(magnitude: f32) -> f32 {
    20.0 * magnitude.max(1.0e-12).log10()
}

pub(super) fn response_db_to_y(rect: egui::Rect, db: f32) -> f32 {
    let span = MAX_RESPONSE_DB - MIN_RESPONSE_DB;
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
) -> Vec<egui::Pos2> {
    let mut frequencies = Vec::with_capacity(count + 10);
    for index in 0..=count {
        let x = index as f32 / count as f32;
        frequencies.push(MIN_CUTOFF_HZ * (MAX_CUTOFF_HZ / MIN_CUTOFF_HZ).powf(x));
    }
    let mut centers = vec![config.cutoff_hz];
    let stage_count = usize::from(config.stage_count());
    if !matches!(config.mode, crate::filters::FilterMode::Svf) {
        centers
            .extend((0..stage_count).map(|index| config.stage_frequency(index, dsp_sample_rate)));
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
    frequencies
        .into_iter()
        .map(|frequency| {
            let db = response_db(config.response_magnitude(frequency, dsp_sample_rate));
            egui::pos2(
                frequency_to_x(rect, frequency),
                response_db_to_y(rect, db).clamp(overflow_top, overflow_bottom),
            )
        })
        .collect()
}

pub(super) fn paint_readout(
    ui: &egui::Ui,
    rect: egui::Rect,
    label: &str,
    value: &str,
    normalized: f32,
    response: &egui::Response,
    accent: egui::Color32,
) {
    let palette = editor_theme::semantic();
    let active = response.is_pointer_button_down_on() || response.dragged();
    let text_x = rect.left() + editor_theme::space::XS;
    ui.painter().text(
        egui::pos2(text_x, rect.top() + editor_theme::font::CAPTION_SIZE * 0.55),
        egui::Align2::LEFT_CENTER,
        label,
        editor_theme::font::caption(),
        if active || response.hovered() {
            accent
        } else {
            palette.text_muted
        },
    );
    ui.painter().text(
        egui::pos2(
            text_x,
            rect.bottom() - editor_theme::font::VALUE_SIZE * 0.55,
        ),
        egui::Align2::LEFT_CENTER,
        value,
        editor_theme::font::value(),
        if active { palette.text } else { accent },
    );
    let y = rect.bottom() - editor_theme::shape::STROKE;
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), y),
            egui::pos2(
                egui::lerp(rect.left()..=rect.right(), normalized.clamp(0.0, 1.0)),
                y,
            ),
        ],
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            accent.gamma_multiply(if active { 1.0 } else { 0.72 }),
        ),
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
