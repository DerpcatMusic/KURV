use crate::{WidgetTokens, chrome, tokens::LIGHT_TOKENS};
use egui::{Align2, FontId, Response, Sense, Ui, Vec2};
use std::ops::RangeInclusive;

pub fn slider(ui: &mut Ui, label: &str, value: &mut f32, range: RangeInclusive<f32>) -> Response {
    slider_with_tokens(ui, label, value, range, "", &LIGHT_TOKENS)
}

pub fn slider_with_tokens(
    ui: &mut Ui,
    label: &str,
    value: &mut f32,
    range: RangeInclusive<f32>,
    suffix: &str,
    tokens: &WidgetTokens,
) -> Response {
    let width = ui
        .available_width()
        .clamp(tokens.spacing.lg * 11.25, tokens.spacing.lg * 20.0);
    let height = tokens.spacing.lg * 2.0;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
    let colors = tokens.colors;
    let min = *range.start();
    let max = *range.end();
    let span = (max - min).max(f32::EPSILON);

    if (response.dragged() || response.clicked())
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let t = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        *value = min + span * t;
    }

    let label_width = tokens.spacing.lg * 4.75;
    let value_width = tokens.spacing.lg * 3.375;
    let track_height = tokens.spacing.xs * 1.85;
    let t = ((*value - min) / span).clamp(0.0, 1.0);
    let track = egui::Rect::from_min_max(
        egui::pos2(
            rect.left() + label_width,
            rect.center().y + tokens.spacing.xs * 0.5,
        ),
        egui::pos2(
            rect.right() - value_width,
            rect.center().y + tokens.spacing.xs * 0.5 + track_height,
        ),
    );
    let handle = egui::pos2(track.left() + track.width() * t, track.center().y);

    let painter = ui.painter_at(rect);
    painter.text(
        egui::pos2(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(tokens.spacing.sm + tokens.spacing.xs * 0.75),
        colors.text,
    );
    let interactive = response.hovered() || response.dragged();
    chrome::draw_slider_track(&painter, track, t, tokens, interactive);
    let handle_radius = tokens.spacing.xs + 1.25;
    chrome::draw_slider_knob(&painter, handle, handle_radius, tokens, interactive);
    painter.text(
        egui::pos2(rect.right(), rect.center().y),
        Align2::RIGHT_CENTER,
        format!("{:.1}{suffix}", *value),
        FontId::monospace(tokens.spacing.sm + tokens.spacing.xs * 0.625),
        colors.muted,
    );

    response
}
