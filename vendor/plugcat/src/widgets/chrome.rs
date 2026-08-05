use crate::tokens::{WidgetTokens, with_alpha};
use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Stroke, Vec2};

pub fn segment_rect(rect: Rect, segments: usize, index: usize) -> Rect {
    let width = rect.width() / segments as f32;
    Rect::from_min_max(
        Pos2::new(rect.left() + width * index as f32, rect.top()),
        Pos2::new(rect.left() + width * (index + 1) as f32, rect.bottom()),
    )
}

pub fn segment_rounding(tokens: &WidgetTokens, index: usize, segments: usize) -> CornerRadius {
    let r = tokens.radius.control;
    if segments == 1 {
        return CornerRadius::same(r);
    }
    if index == 0 {
        CornerRadius {
            nw: r,
            sw: r,
            ..Default::default()
        }
    } else if index + 1 == segments {
        CornerRadius {
            ne: r,
            se: r,
            ..Default::default()
        }
    } else {
        CornerRadius::ZERO
    }
}

pub fn segment_text_color(tokens: &WidgetTokens, active: bool) -> Color32 {
    if active {
        Color32::WHITE
    } else {
        tokens.colors.text
    }
}

pub fn segment_label_font(compact: bool) -> egui::FontId {
    egui::FontId::new(
        if compact { 11.0 } else { 12.0 },
        egui::FontFamily::Proportional,
    )
}

pub fn draw_flat_field_shell(painter: &Painter, rect: Rect, tokens: &WidgetTokens) {
    painter.rect_filled(
        rect,
        tokens.radius.control,
        tokens.colors.control_shell_fill(),
    );
    painter.rect_stroke(
        rect.shrink(0.5),
        tokens.radius.control,
        control_stroke(tokens),
        egui::StrokeKind::Inside,
    );
}

pub fn draw_group_shell(painter: &Painter, rect: Rect, segments: usize, tokens: &WidgetTokens) {
    draw_flat_field_shell(painter, rect, tokens);
    if segments > 1 {
        let divider = Stroke::new(
            1.0_f32,
            with_alpha(
                tokens.colors.border,
                if tokens.light_visuals { 200 } else { 235 },
            ),
        );
        for index in 1..segments {
            let x = rect.left() + rect.width() * index as f32 / segments as f32;
            painter.line_segment(
                [
                    Pos2::new(x, rect.top() + 1.0),
                    Pos2::new(x, rect.bottom() - 1.0),
                ],
                divider,
            );
        }
    }
}

pub fn draw_segment_pressed(
    painter: &Painter,
    rect: Rect,
    index: usize,
    segments: usize,
    tokens: &WidgetTokens,
) {
    let rounding = segment_rounding(tokens, index, segments);
    let clip_rect = segment_clip_rect(rect, index, segments);
    let clipped = painter.with_clip_rect(clip_rect);
    clipped.rect_filled(rect, rounding, tokens.colors.selected_fill());
}

pub fn draw_segment_hover(
    painter: &Painter,
    rect: Rect,
    index: usize,
    segments: usize,
    tokens: &WidgetTokens,
) {
    let rounding = segment_rounding(tokens, index, segments);
    let clip_rect = segment_clip_rect(rect, index, segments);
    let clipped = painter.with_clip_rect(clip_rect);
    clipped.rect_filled(rect, rounding, tokens.colors.control_shell_fill());
}

pub fn draw_vertical_separator(painter: &Painter, x: f32, rect: Rect, tokens: &WidgetTokens) {
    painter.line_segment(
        [
            Pos2::new(x, rect.top() + 4.0),
            Pos2::new(x, rect.bottom() - 4.0),
        ],
        control_stroke(tokens),
    );
}

pub fn draw_slider_track(
    painter: &Painter,
    track: Rect,
    fill_right: f32,
    tokens: &WidgetTokens,
    interactive: bool,
) {
    let grow = if interactive { 1.14 } else { 1.0 };
    let center = track.center();
    let grown = Rect::from_center_size(center, Vec2::new(track.width(), track.height() * grow));
    let rounding = CornerRadius::same((grown.height() * 0.22).round().clamp(1.0, 3.0) as u8);

    painter.rect_filled(grown, rounding, tokens.colors.control_track());

    let active_right = grown.left() + grown.width() * fill_right.clamp(0.0, 1.0);
    if active_right > grown.left() + 0.5 {
        painter.rect_filled(
            Rect::from_min_max(grown.left_top(), Pos2::new(active_right, grown.bottom())),
            rounding,
            tokens.colors.control_accent(),
        );
    }
}

pub fn draw_slider_knob(
    painter: &Painter,
    center: Pos2,
    radius: f32,
    tokens: &WidgetTokens,
    interactive: bool,
) {
    let scale = if interactive { 1.08 } else { 1.0 };
    let size = Vec2::new(radius * 1.55 * scale, radius * 2.05 * scale);
    let rect = Rect::from_center_size(center, size);
    let rounding = CornerRadius::same((size.x.min(size.y) * 0.22).round().clamp(1.0, 4.0) as u8);

    painter.add(egui::Shape::Rect(
        egui::epaint::Shadow {
            offset: [0, 1],
            blur: if tokens.light_visuals { 3 } else { 4 },
            spread: 0,
            color: with_alpha(
                tokens.colors.shadow,
                if tokens.light_visuals { 64 } else { 100 },
            ),
        }
        .as_shape(rect, rounding),
    ));
    painter.rect_filled(rect, rounding, tokens.colors.surface_high);
    painter.line_segment(
        [
            Pos2::new(rect.left() + 2.0, rect.top() + 1.5),
            Pos2::new(rect.right() - 2.0, rect.top() + 1.5),
        ],
        Stroke::new(
            tokens.stroke.control * 0.75,
            with_alpha(tokens.colors.white, 96),
        ),
    );
}

pub fn section_caption_y(control_row_top: f32) -> f32 {
    control_row_top - 3.0
}

fn control_stroke(tokens: &WidgetTokens) -> Stroke {
    Stroke::new(tokens.stroke.control, tokens.colors.border)
}

fn segment_clip_rect(rect: Rect, index: usize, segments: usize) -> Rect {
    if segments <= 1 {
        return rect;
    }
    let overlap = 0.5;
    Rect::from_min_max(
        Pos2::new(
            rect.left() - if index == 0 { 0.0 } else { overlap },
            rect.top(),
        ),
        Pos2::new(
            rect.right() + if index + 1 == segments { 0.0 } else { overlap },
            rect.bottom(),
        ),
    )
}
