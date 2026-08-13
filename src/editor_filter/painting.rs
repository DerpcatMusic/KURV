//! Paint-only helpers for the ordered filter card.

use crate::filters::{FilterConfig, FilterMode};
use crate::{editor_theme, editor_widgets};

use super::{
    MAX_CUTOFF_HZ, MAX_RESPONSE_SEGMENTS, MIN_CUTOFF_HZ, MIN_RESPONSE_SEGMENTS, normalized_log,
};

pub(super) fn paint_header(
    ui: &egui::Ui,
    label_rect: egui::Rect,
    close_rect: egui::Rect,
    drag_response: &egui::Response,
    close_response: &egui::Response,
    accent: egui::Color32,
) {
    let palette = editor_theme::semantic();
    ui.painter().text(
        label_rect.left_center() + egui::vec2(editor_theme::space::XXS, 0.0),
        egui::Align2::LEFT_CENTER,
        "FILTER",
        editor_theme::font::caption(),
        if drag_response.dragged() || drag_response.hovered() {
            accent
        } else {
            palette.text_muted
        },
    );
    let cross = close_rect.shrink(close_rect.width() * 0.34);
    let color = if close_response.hovered() {
        palette.danger
    } else {
        palette.text_muted
    };
    let stroke = egui::Stroke::new(editor_theme::shape::STROKE, color);
    ui.painter()
        .line_segment([cross.left_top(), cross.right_bottom()], stroke);
    ui.painter()
        .line_segment([cross.right_top(), cross.left_bottom()], stroke);
}

pub(super) fn paint_response_preview(
    ui: &egui::Ui,
    rect: egui::Rect,
    config: FilterConfig,
    accent: egui::Color32,
    response: &egui::Response,
) {
    if !rect.is_positive() {
        return;
    }
    let painter = ui.painter_at(rect);
    let cutoff_x = egui::lerp(
        rect.left()..=rect.right(),
        normalized_log(config.cutoff_hz, MIN_CUTOFF_HZ, MAX_CUTOFF_HZ),
    );
    painter.line_segment(
        [
            egui::pos2(cutoff_x, rect.top()),
            egui::pos2(cutoff_x, rect.bottom()),
        ],
        egui::Stroke::new(editor_theme::shape::STROKE, accent.gamma_multiply(0.18)),
    );
    let count = ((rect.width() / editor_theme::space::XS.max(1.0)).ceil() as usize)
        .clamp(MIN_RESPONSE_SEGMENTS, MAX_RESPONSE_SEGMENTS);
    let stroke = egui::Stroke::new(
        editor_theme::shape::FOCUS_STROKE,
        accent.gamma_multiply(if response.hovered() { 1.0 } else { 0.86 }),
    );
    let mesh = editor_widgets::cached_stroke_mesh(
        ui,
        response.id.with("filter-response-mesh"),
        (
            config.mode as u8,
            config.cutoff_hz.to_bits(),
            config.q.to_bits(),
            config.slope_db_oct.to_bits(),
            config.morph.to_bits(),
            [
                rect.min.x.to_bits(),
                rect.min.y.to_bits(),
                rect.width().to_bits(),
                rect.height().to_bits(),
            ],
            count,
            painter.ctx().pixels_per_point().to_bits(),
            stroke.color.to_array(),
        ),
        || {
            (0..=count)
                .map(|index| {
                    let x = index as f32 / count as f32;
                    let frequency = MIN_CUTOFF_HZ * (MAX_CUTOFF_HZ / MIN_CUTOFF_HZ).powf(x);
                    let magnitude = response_magnitude(config, frequency);
                    let db = (20.0 * magnitude.max(0.000_001).log10()).clamp(-36.0, 12.0);
                    egui::pos2(
                        egui::lerp(rect.left()..=rect.right(), x),
                        egui::lerp(rect.bottom()..=rect.top(), (db + 36.0) / 48.0),
                    )
                })
                .collect()
        },
        stroke,
    );
    painter.add(mesh);
    let radius = editor_theme::font::CAPTION_SIZE * if response.dragged() { 0.34 } else { 0.27 };
    let bounds = rect.shrink(radius.min(rect.width().min(rect.height()) * 0.5));
    let handle = egui::pos2(
        cutoff_x.clamp(bounds.left(), bounds.right()),
        egui::lerp(
            rect.bottom()..=rect.top(),
            normalized_log(config.q, 0.1, 32.0),
        )
        .clamp(bounds.top(), bounds.bottom()),
    );
    painter.circle_filled(
        handle,
        radius,
        if response.hovered() {
            editor_theme::semantic().text
        } else {
            accent
        },
    );
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
    ui.painter().text(
        egui::pos2(
            rect.center().x,
            rect.top() + editor_theme::font::CAPTION_SIZE * 0.55,
        ),
        egui::Align2::CENTER_CENTER,
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
            rect.center().x,
            rect.bottom() - editor_theme::font::VALUE_SIZE * 0.55,
        ),
        egui::Align2::CENTER_CENTER,
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

fn response_magnitude(config: FilterConfig, frequency: f32) -> f32 {
    let ratio = frequency / config.cutoff_hz.max(MIN_CUTOFF_HZ);
    let stage = (config.slope_db_oct / 12.0).clamp(1.0, 4.0);
    match config.mode {
        FilterMode::Svf => {
            let denominator = ((1.0 - ratio * ratio).powi(2) + (ratio / config.q.max(0.1)).powi(2))
                .sqrt()
                .max(0.000_001);
            let blend = config.morph.mul_add(2.0, -1.0);
            let low = (-blend).max(0.0) / denominator;
            let band = (1.0 - blend * blend).max(0.0).sqrt() * ratio / denominator;
            let high = blend.max(0.0) * ratio * ratio / denominator;
            (low + band + high).powf(stage)
        }
        FilterMode::Phaser => {
            let phase = ratio.atan() * std::f32::consts::FRAC_2_PI;
            (0.5 + 0.5 * (phase * std::f32::consts::PI * stage).cos())
                .abs()
                .max(0.01)
        }
        FilterMode::Fibonacci => [1.0_f32, 2.0, 3.0, 5.0]
            .into_iter()
            .fold(1.0, |magnitude, fib| {
                let distance = (frequency
                    / (config.cutoff_hz * fib.powf(config.morph.mul_add(0.35, 0.65))))
                .log2();
                magnitude * (1.0 - (-distance * distance * config.q.max(0.1)).exp() * 0.92)
            })
            .max(0.01),
    }
}
