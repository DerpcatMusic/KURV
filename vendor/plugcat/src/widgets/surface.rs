use crate::{LIGHT_TOKENS, theme::UiTheme, tokens::WidgetTokens};
use egui::{Color32, Frame, InnerResponse, Margin, Painter, Rect, Stroke, Ui};

pub fn surface<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    surface_with_tokens(ui, &LIGHT_TOKENS, add_contents)
}

pub fn surface_with_tokens<R>(
    ui: &mut Ui,
    tokens: &WidgetTokens,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    surface_with_margin_with_tokens(
        ui,
        tokens,
        Margin::symmetric(
            tokens.spacing.md.round() as i8,
            tokens.spacing.md.round() as i8,
        ),
        add_contents,
    )
}

pub fn surface_with_margin_with_tokens<R>(
    ui: &mut Ui,
    tokens: &WidgetTokens,
    inner_margin: Margin,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let frame = Frame::new()
        .fill(tokens.colors.surface)
        .stroke(surface_stroke(tokens))
        .shadow(surface_shadow(tokens))
        .corner_radius(tokens.radius.panel)
        .inner_margin(inner_margin);
    let mut prepared = frame.begin(ui);
    let ret = add_contents(&mut prepared.content_ui);
    let response = prepared.end(ui);
    InnerResponse::new(ret, response)
}

pub fn surface_with_theme<R>(
    ui: &mut Ui,
    theme: &UiTheme,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    surface_with_margin_with_theme(
        ui,
        theme,
        Margin::symmetric(
            theme
                .metrics()
                .space(crate::layout::SpacingStep::Md)
                .round() as i8,
            theme
                .metrics()
                .space(crate::layout::SpacingStep::Md)
                .round() as i8,
        ),
        add_contents,
    )
}

pub fn surface_with_margin_with_theme<R>(
    ui: &mut Ui,
    theme: &UiTheme,
    inner_margin: Margin,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    surface_with_margin_with_tokens(ui, theme.palette(), inner_margin, add_contents)
}

pub fn paint_surface_chrome(_painter: &Painter, _rect: Rect, _tokens: &WidgetTokens) {}

fn surface_shadow(_tokens: &WidgetTokens) -> egui::epaint::Shadow {
    egui::epaint::Shadow {
        offset: [0, 0],
        blur: 0,
        spread: 0,
        color: Color32::TRANSPARENT,
    }
}

fn surface_stroke(tokens: &WidgetTokens) -> Stroke {
    Stroke::new(tokens.stroke.control, tokens.colors.border)
}
