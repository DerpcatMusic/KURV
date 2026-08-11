//! Paint-only helpers for the ordered filter card.

use crate::editor_theme;
use crate::filters::{FilterConfig, FilterMode};

use super::{
    MAX_CUTOFF_HZ, MAX_Q, MAX_RESPONSE_SEGMENTS, MIN_CUTOFF_HZ, MIN_Q, MIN_RESPONSE_SEGMENTS,
    denormalized_log, normalized_log,
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
    let close_color = if close_response.hovered() {
        palette.danger
    } else {
        palette.text_muted
    };
    let stroke = egui::Stroke::new(editor_theme::shape::STROKE, close_color);
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

    let point_count = ((rect.width() / editor_theme::space::XS.max(1.0)).ceil() as usize)
        .clamp(MIN_RESPONSE_SEGMENTS, MAX_RESPONSE_SEGMENTS);
    let mut points = Vec::with_capacity(point_count + 1);
    for index in 0..=point_count {
        let normalized = index as f32 / point_count as f32;
        let frequency = denormalized_log(normalized, MIN_CUTOFF_HZ, MAX_CUTOFF_HZ);
        let magnitude = response_magnitude(config.mode, frequency / config.cutoff_hz, config.q);
        let decibels = (20.0 * magnitude.max(f32::MIN_POSITIVE).log10()).clamp(-36.0, 12.0);
        points.push(egui::pos2(
            egui::lerp(rect.left()..=rect.right(), normalized),
            egui::lerp(rect.bottom()..=rect.top(), (decibels + 36.0) / 48.0),
        ));
    }
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(
            editor_theme::shape::FOCUS_STROKE,
            accent.gamma_multiply(if response.hovered() { 1.0 } else { 0.86 }),
        ),
    ));
    let handle_radius =
        editor_theme::font::CAPTION_SIZE * if response.dragged() { 0.34 } else { 0.27 };
    let handle_bounds = rect.shrink(handle_radius.min(rect.width().min(rect.height()) * 0.5));
    let handle = egui::pos2(
        cutoff_x.clamp(handle_bounds.left(), handle_bounds.right()),
        egui::lerp(
            rect.bottom()..=rect.top(),
            normalized_log(config.q, MIN_Q, MAX_Q),
        )
        .clamp(handle_bounds.top(), handle_bounds.bottom()),
    );
    painter.circle_filled(
        handle,
        handle_radius,
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
    let label_y = rect.top() + editor_theme::font::CAPTION_SIZE * 0.5;
    let value_y = rect.bottom() - editor_theme::font::VALUE_SIZE * 0.5;
    ui.painter().text(
        egui::pos2(rect.center().x, label_y),
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
        egui::pos2(rect.center().x, value_y),
        egui::Align2::CENTER_CENTER,
        value,
        editor_theme::font::value(),
        if active { palette.text } else { accent },
    );

    let track = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.bottom() - editor_theme::shape::STROKE),
        rect.right_bottom(),
    );
    ui.painter().rect_filled(
        egui::Rect::from_min_max(
            track.min,
            egui::pos2(
                egui::lerp(track.left()..=track.right(), normalized),
                track.bottom(),
            ),
        ),
        0.0,
        accent.gamma_multiply(if active { 1.0 } else { 0.72 }),
    );
}

fn response_magnitude(mode: FilterMode, ratio: f32, q: f32) -> f32 {
    let denominator = ((1.0 - ratio * ratio).powi(2) + (ratio / q).powi(2)).sqrt();
    match mode {
        FilterMode::LowPass => denominator.recip(),
        FilterMode::BandPass => ratio / denominator,
        FilterMode::HighPass => ratio * ratio / denominator,
    }
}
