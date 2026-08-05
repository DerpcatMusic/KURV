use crate::{
    chrome,
    tokens::{LIGHT_TOKENS, WidgetTokens, lerp_color, with_alpha},
};
use egui::{Align2, CursorIcon, FontId, Rect, Response, Sense, Stroke, Ui, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonTone {
    Primary,
    Neutral,
    Subtle,
    Danger,
    PowerOff,
    PowerOn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonSize {
    Compact,
    Regular,
}

pub fn button(ui: &mut Ui, label: &str, tone: ButtonTone, size: ButtonSize) -> Response {
    button_with_tokens(ui, label, tone, size, &LIGHT_TOKENS)
}

pub fn button_with_tokens(
    ui: &mut Ui,
    label: &str,
    tone: ButtonTone,
    size: ButtonSize,
    tokens: &WidgetTokens,
) -> Response {
    let height = match size {
        ButtonSize::Compact => tokens.spacing.sm * 3.0,
        ButtonSize::Regular => tokens.spacing.sm * 3.0 + tokens.spacing.xs * 1.5,
    };
    let width = (label.chars().count() as f32 * tokens.spacing.sm + tokens.spacing.lg * 1.75)
        .max(tokens.spacing.lg * 4.5);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let colors = tokens.colors;
    let hovered = response.hovered();
    let (fill, text, stroke) = match tone {
        ButtonTone::Primary => (
            colors.control_shell_fill(),
            colors.text,
            Stroke::new(tokens.stroke.control, colors.border),
        ),
        ButtonTone::Neutral => (
            colors.control_shell_fill(),
            colors.text,
            Stroke::new(tokens.stroke.control, colors.border),
        ),
        ButtonTone::Subtle => (
            colors.control_shell_fill(),
            colors.muted,
            Stroke::new(tokens.stroke.control, colors.border),
        ),
        ButtonTone::Danger => (
            colors.control_shell_fill(),
            colors.error,
            Stroke::new(tokens.stroke.control, colors.border),
        ),
        ButtonTone::PowerOff => (
            lerp_color(
                colors.control_shell_fill(),
                colors.error,
                if hovered { 0.18 } else { 0.10 },
            ),
            colors.error,
            Stroke::new(tokens.stroke.control, with_alpha(colors.error, 190)),
        ),
        ButtonTone::PowerOn => (
            colors.selected_fill(),
            colors.white,
            Stroke::new(tokens.stroke.control, colors.selected_fill()),
        ),
    };

    chrome::draw_flat_field_shell(ui.painter(), rect, tokens);
    ui.painter().rect(
        rect.shrink(1.0),
        tokens.radius.control,
        fill,
        stroke,
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(if size == ButtonSize::Compact {
            tokens.spacing.sm + tokens.spacing.xs * 0.75
        } else {
            tokens.spacing.sm + tokens.spacing.xs
        }),
        text,
    );
    response
}

pub fn icon_button_rect_with_tokens(
    ui: &mut Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    rect: Rect,
    active: bool,
    tooltip: Option<&str>,
    tokens: &WidgetTokens,
    paint_icon: impl FnOnce(&egui::Painter, Rect, egui::Color32),
) -> Response {
    let mut response = ui
        .interact(rect, ui.make_persistent_id(id_source), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    if let Some(tooltip) = tooltip {
        response = response.on_hover_text(tooltip);
    }

    chrome::draw_flat_field_shell(ui.painter(), rect, tokens);
    if active {
        chrome::draw_segment_pressed(ui.painter(), rect.shrink(1.0), 0, 1, tokens);
    } else if response.hovered() {
        chrome::draw_segment_hover(ui.painter(), rect.shrink(1.0), 0, 1, tokens);
    }

    let color = if active {
        tokens.colors.white
    } else if response.hovered() {
        tokens.colors.text
    } else {
        tokens.colors.text
    };
    paint_icon(ui.painter(), rect, color);
    response
}

pub fn dropdown_button_rect_with_tokens(
    ui: &mut Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    rect: Rect,
    tooltip: Option<&str>,
    tokens: &WidgetTokens,
    paint_contents: impl FnOnce(&egui::Painter, Rect, egui::Color32),
) -> Response {
    let mut response = ui
        .interact(rect, ui.make_persistent_id(id_source), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    if let Some(tooltip) = tooltip {
        response = response.on_hover_text(tooltip);
    }

    chrome::draw_flat_field_shell(ui.painter(), rect, tokens);
    if response.hovered() {
        chrome::draw_segment_hover(ui.painter(), rect.shrink(1.0), 0, 1, tokens);
    }

    let color = if response.hovered() {
        tokens.colors.text
    } else {
        tokens.colors.muted
    };
    paint_contents(ui.painter(), rect, color);
    response
}
